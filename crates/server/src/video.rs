//! Video recordings. A recording made from one untrimmed video file also plays its picture in
//! the web library; its transcript comes from the audio like any other (`pipeline`).
//!
//! What verification found in the upload (`audio::Video`) decides the plan once, when the entry
//! is created. A plain MP4 every browser plays is served as it was uploaded. Anything else is
//! converted to `video.mp4` by a job of its own, so a long conversion never holds up the
//! transcript.
//!
//! `entries.video_status`: NULL without video, `original`, or `queued` → `converting` →
//! `ready` or `failed`. `video_width` and `video_height` are the size the browser shows, set
//! with the status. `video_attempts` counts conversions the server did not live through.

use crate::{
    audio::{self, Format, Video},
    config::{App, RESERVE, free_space, now},
    pipeline::commit_artifact,
};
use anyhow::{Context, Result, ensure};
use std::path::Path;
use tokio_util::sync::CancellationToken;

/// A converted video, beside `playback.m4a`.
pub const FILE: &str = "video.mp4";
const PARTIAL: &str = "video.partial.mp4";
/// Conversions the server may die in before the video is given up.
const ATTEMPTS: i64 = 2;
/// The bitrate ceiling of a conversion, video and audio, which bounds the disk it may need.
const MAX_BITS_PER_SECOND: u64 = 3_000_000 + 320_000;
/// Brands browsers read as MP4. QuickTime (`qt  `) is not one.
const MP4_BRANDS: [&str; 9] = [
    "isom", "iso2", "iso4", "iso5", "iso6", "mp41", "mp42", "avc1", "M4V ",
];

#[derive(Debug, PartialEq)]
pub enum Plan {
    /// The upload plays as it is, in Chrome, Firefox and Safari.
    Original { width: u32, height: u32 },
    /// H.264 SDR, the shorter side at most 720 pixels.
    Convert { width: u32, height: u32 },
}
impl Plan {
    /// The entry's `video_status`, `video_width` and `video_height`.
    pub fn columns(&self) -> (&'static str, i64, i64) {
        match *self {
            Plan::Original { width, height } => ("original", width.into(), height.into()),
            Plan::Convert { width, height } => ("queued", width.into(), height.into()),
        }
    }
}

/// The video of a recording made from `sources`. Joining or trimming video would need its
/// streams to match, so only one untrimmed file gets one.
pub fn plan(sources: &[&Format], trimmed: bool) -> Option<Plan> {
    let ([format], false) = (sources, trimmed) else {
        return None;
    };
    let video = format.video.as_ref()?;
    let (width, height) = video.display();
    Some(if plays_as_is(format, video) {
        Plan::Original { width, height }
    } else {
        let (width, height) = fit(width, height);
        Plan::Convert { width, height }
    })
}

/// A plain MP4: one H.264 picture, 8-bit 4:2:0, SDR, square pixels, and one AAC track. Nothing
/// else in the file: the timecode and metadata tracks of iPhone videos are known to stall
/// players.
fn plays_as_is(format: &Format, video: &Video) -> bool {
    MP4_BRANDS.contains(&video.brand.as_str())
        && video.streams == 2
        && video.codec == "h264"
        && matches!(video.pix_fmt.as_str(), "yuv420p" | "yuvj420p")
        && video.sar[0] == video.sar[1]
        && !video.hdr()
        && format.codec == "aac"
}

/// The shorter side at most 720 pixels, the shape kept, both sides even for H.264.
fn fit(width: u32, height: u32) -> (u32, u32) {
    let short = u64::from(width.min(height));
    let target = short.min(720);
    let side = |n: u32| ((u64::from(n) * target / short) & !1).max(2) as u32;
    (side(width), side(height))
}

/// After a restart: a conversion the server died in is queued again, until it has done so
/// `ATTEMPTS` times. Runs before the worker claims anything.
pub async fn recover(app: &App) {
    let failure = "The server stopped while converting this video, more than once.";
    let given_up: Vec<String> = sqlx::query_scalar!("UPDATE entries SET video_status='failed',video_failure=?,video_attempts=0 WHERE video_status='converting' AND video_attempts>=? RETURNING id AS \"id!\"", failure, ATTEMPTS)
        .fetch_all(&app.db)
        .await
        .unwrap_or_default();
    for entry in given_up {
        let _ = tokio::fs::remove_file(app.entry_dir(&entry).join(PARTIAL)).await;
    }
    let _ =
        sqlx::query!("UPDATE entries SET video_status='queued' WHERE video_status='converting'")
            .execute(&app.db)
            .await;
}

/// Atomically takes the oldest queued video of a recording outside Trash.
pub async fn claim(app: &App) -> Option<String> {
    let timestamp = now();
    sqlx::query_scalar!("UPDATE entries SET video_status='converting',video_attempts=video_attempts+1,updated_at=? WHERE id=(SELECT id FROM entries WHERE video_status='queued' AND trashed_at IS NULL ORDER BY created_at LIMIT 1) AND video_status='queued' RETURNING id AS \"id!\"", timestamp)
        .fetch_optional(&app.db)
        .await
        .unwrap_or_else(|error| {
            tracing::error!(%error, "could not claim a video conversion");
            None
        })
}

/// Converts a claimed video and records how it ended. `stop` (shutdown, or Trash through
/// `App::stop_conversion`) kills FFmpeg and puts the video back in the queue.
pub async fn convert(app: &App, entry: &str, stop: CancellationToken) {
    // Trashed after the claim but before `stop` was registered, so Trash could not reach it.
    let trashed: Option<bool> = sqlx::query_scalar!(
        "SELECT trashed_at IS NOT NULL AS \"trashed!: bool\" FROM entries WHERE id=?",
        entry
    )
    .fetch_optional(&app.db)
    .await
    .unwrap_or(None);
    if trashed != Some(false) {
        stop.cancel();
    }
    let result = tokio::select! {
        // `biased`: a stopped conversion does not start FFmpeg first.
        biased;
        _ = stop.cancelled() => None,
        result = run(app, entry) => Some(result),
    };
    if !matches!(result, Some(Ok(()))) {
        let _ = tokio::fs::remove_file(app.entry_dir(entry).join(PARTIAL)).await;
    }
    let timestamp = now();
    let (status, failure) = match result {
        Some(Ok(())) => ("ready", None),
        Some(Err(error)) => {
            tracing::error!(entry=%entry, error=%error, "video conversion failed");
            (
                "failed",
                Some(error.to_string().chars().take(1000).collect()),
            )
        }
        None => ("queued", None::<String>),
    };
    let saved = sqlx::query!("UPDATE entries SET video_status=?,video_failure=?,video_attempts=0,updated_at=? WHERE id=? AND video_status='converting'", status, failure, timestamp, entry)
        .execute(&app.db)
        .await;
    if let Err(error) = saved {
        tracing::error!(entry=%entry, %error, "could not save how a video conversion ended");
    }
}

async fn run(app: &App, entry: &str) -> Result<()> {
    let dir = app.entry_dir(entry);
    // Finished before a restart could record it.
    if dir.join(FILE).exists() {
        return Ok(());
    }
    let row = sqlx::query!("SELECT es.upload_id,u.format_json,e.duration_ms,e.video_width,e.video_height FROM entries e JOIN entry_sources es ON es.entry_id=e.id JOIN uploads u ON u.id=es.upload_id WHERE e.id=?", entry)
        .fetch_one(&app.db)
        .await?;
    let format: Format = serde_json::from_str(row.format_json.as_deref().unwrap_or("null"))?;
    let video = format
        .video
        .as_ref()
        .context("This recording has no video")?;
    let size = (row.video_width, row.video_height);
    let (Some(width), Some(height)) = size else {
        anyhow::bail!("This recording has no video size");
    };
    let _cpu = app.cpu.acquire().await?;
    // Room for the largest file the bitrate ceiling allows, above what the database keeps free.
    let largest = row.duration_ms.max(0) as u64 * MAX_BITS_PER_SECOND / 8000 * 105 / 100;
    ensure!(
        free_space(&app.config.data)? >= RESERVE + largest,
        "Not enough free disk space to convert this video"
    );
    tokio::fs::create_dir_all(&dir).await?;
    let source = app.upload_path(&row.upload_id);
    let partial = dir.join(PARTIAL);
    audio::ffmpeg(&arguments(
        &source,
        &format,
        video,
        (width, height),
        &partial,
    ))
    .await?;
    commit_artifact(&partial, &dir.join(FILE)).await
}

/// Server-owned paths and numbers only: no user input reaches FFmpeg's syntax. FFmpeg turns
/// the picture upright before the filter, so `size` is the upright size.
fn arguments(
    source: &Path,
    format: &Format,
    video: &Video,
    (width, height): (i64, i64),
    to: &Path,
) -> Vec<String> {
    let mut filter = format!("scale={width}:{height},setsar=1");
    if video.hdr() {
        // Tone-mapped to SDR after scaling, on a fraction of the pixels.
        filter.push_str(",zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv");
    }
    filter.push_str(",format=yuv420p");
    let audio = if format.codec == "aac" {
        "-c:a copy"
    } else {
        "-c:a aac -b:a 128k"
    };
    // Two threads each way, one for filters: the container has two CPUs, and every decoding
    // thread holds frames of the full-size picture in memory.
    let mut args: Vec<String> = ["-filter_threads", "1", "-threads", "2", "-i"]
        .map(String::from)
        .into();
    args.push(source.display().to_string());
    args.extend(["-map".into(), format!("0:{}", video.stream)]);
    args.extend(["-map", "0:a:0", "-vf"].map(String::from));
    args.push(filter);
    args.extend(
        "-c:v libx264 -preset veryfast -crf 23 -maxrate 3M -bufsize 6M -threads 2"
            .split(' ')
            .map(String::from),
    );
    args.extend(audio.split(' ').map(String::from));
    args.extend(["-movflags", "+faststart"].map(String::from));
    args.push(to.display().to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(codec: &str, video: Video) -> Format {
        Format {
            sample_rate: 48000,
            channels: 2,
            bit_depth: 16,
            is_float: true,
            frames: 48000,
            codec: codec.into(),
            video: Some(video),
        }
    }
    /// A phone's MP4: H.264 1080p, AAC, nothing else.
    fn phone() -> Video {
        Video {
            stream: 0,
            codec: "h264".into(),
            pix_fmt: "yuv420p".into(),
            width: 1920,
            height: 1080,
            sar: [1, 1],
            rotation: 0,
            transfer: "bt709".into(),
            brand: "isom".into(),
            streams: 2,
        }
    }

    #[test]
    fn a_plain_mp4_plays_as_it_is_and_anything_else_is_converted() {
        let plan_of = |codec: &str, video: Video| plan(&[&format(codec, video)], false);
        let original = Some(Plan::Original {
            width: 1920,
            height: 1080,
        });
        let converted = Some(Plan::Convert {
            width: 1280,
            height: 720,
        });
        assert_eq!(plan_of("aac", phone()), original);
        // Full-range 4:2:0 is the same picture to a browser.
        let full_range = Video {
            pix_fmt: "yuvj420p".into(),
            ..phone()
        };
        assert_eq!(plan_of("aac", full_range), original);
        for (codec, video) in [
            (
                "aac",
                Video {
                    brand: "qt  ".into(),
                    ..phone()
                },
            ),
            (
                "aac",
                Video {
                    brand: String::new(),
                    ..phone()
                },
            ), // Matroska
            (
                "aac",
                Video {
                    streams: 3,
                    ..phone()
                },
            ),
            (
                "aac",
                Video {
                    codec: "hevc".into(),
                    ..phone()
                },
            ),
            (
                "aac",
                Video {
                    pix_fmt: "yuv420p10le".into(),
                    ..phone()
                },
            ),
            (
                "aac",
                Video {
                    transfer: "arib-std-b67".into(),
                    ..phone()
                },
            ),
            ("opus", phone()),
        ] {
            assert_eq!(
                plan_of(codec, video.clone()),
                converted,
                "{codec} {video:?}"
            );
        }
    }

    #[test]
    fn the_size_is_upright_square_and_at_most_720_on_the_shorter_side() {
        let convert = |video: Video| plan(&[&format("opus", video)], false);
        let size = |width, height| Some(Plan::Convert { width, height });
        // A portrait phone video: landscape frames turned by the display matrix.
        assert_eq!(
            convert(Video {
                rotation: -90,
                ..phone()
            }),
            size(720, 1280)
        );
        assert_eq!(
            convert(Video {
                rotation: 270,
                ..phone()
            }),
            size(720, 1280)
        );
        // Anamorphic 1440×1080 shows as 1920×1080.
        let wide = Video {
            width: 1440,
            sar: [4, 3],
            ..phone()
        };
        assert_eq!(convert(wide), size(1280, 720));
        // Small videos are not enlarged; odd sides become even.
        assert_eq!(
            convert(Video {
                width: 640,
                height: 360,
                ..phone()
            }),
            size(640, 360)
        );
        assert_eq!(
            convert(Video {
                width: 1001,
                height: 563,
                ..phone()
            }),
            size(1000, 562)
        );
        // An ultrawide screen recording keeps its shape.
        assert_eq!(
            convert(Video {
                width: 5120,
                height: 1440,
                ..phone()
            }),
            size(2560, 720)
        );
    }

    #[test]
    fn only_one_untrimmed_video_file_gets_a_video() {
        let video = format("aac", phone());
        let audio = Format {
            video: None,
            ..video.clone()
        };
        assert!(plan(&[&video], false).is_some());
        assert_eq!(plan(&[&video], true), None);
        assert_eq!(plan(&[&video, &video], false), None);
        assert_eq!(plan(&[&audio], false), None);
    }
}
