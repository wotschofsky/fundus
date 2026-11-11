use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, process::Stdio};
use tokio::{io::AsyncReadExt, process::Command};

/// What verification found in an upload: its first audio stream, and its picture if it has one.
/// Stored as `uploads.format_json`; rows from before a field existed read it as its default.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Format {
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_depth: u16,
    pub is_float: bool,
    pub frames: u64,
    /// The audio codec, as FFmpeg names it ("aac", "flac", "pcm_s24le").
    #[serde(default)]
    pub codec: String,
    /// Where the audio starts on the file's timeline. Decoding starts at its first sample, so
    /// that is the transcript's time 0.
    #[serde(default)]
    pub start_ms: i64,
    /// How long the audio's timestamps run. Where they skip, this is longer than the samples,
    /// which the transcript plays back to back. 0 when the file does not say.
    #[serde(default)]
    pub span_ms: i64,
    #[serde(default)]
    pub video: Option<Video>,
    /// The start the file's own tags give, stated (Apple's `creationdate`) or worked out from
    /// when the file was finished (`made_at` below); None when they give none.
    #[serde(default)]
    pub made_at: Option<DateTime<Utc>>,
}
/// The first video stream that is not cover art. Facts only: what to do with them is
/// `video::plan`'s decision, so it can change without probing the uploads again.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Video {
    /// Its index in the file.
    pub stream: u64,
    pub codec: String,
    pub pix_fmt: String,
    /// Coded size; `display` applies the pixel shape and rotation.
    pub width: u32,
    pub height: u32,
    /// Sample (pixel) aspect ratio; [1, 1] is square.
    pub sar: [u32; 2],
    /// Degrees, from the display matrix.
    pub rotation: i32,
    /// `color_transfer`: "smpte2084" (PQ) and "arib-std-b67" (HLG) are HDR.
    pub transfer: String,
    /// The file's MP4 major brand ("isom", "mp42", "qt  " for QuickTime); empty in other containers.
    pub brand: String,
    /// Streams in the file, cover art and data tracks included.
    pub streams: usize,
    /// Where the picture starts on the file's timeline.
    #[serde(default)]
    pub start_ms: i64,
    /// Where the file's timeline starts: FFmpeg's filters count time from there, not from 0.
    #[serde(default)]
    pub file_start_ms: i64,
}
impl Video {
    /// The size a player shows: square pixels, upright.
    pub fn display(&self) -> (u32, u32) {
        let [num, den] = self.sar;
        let width = (u64::from(self.width) * u64::from(num) + u64::from(den) / 2) / u64::from(den);
        // At least a pixel, whatever pixel shape a file claims.
        let width = width.clamp(1, u64::from(u32::MAX)) as u32;
        if self.rotation.rem_euclid(180) == 90 {
            (self.height, width)
        } else {
            (width, self.height)
        }
    }
    pub fn hdr(&self) -> bool {
        matches!(self.transfer.as_str(), "smpte2084" | "arib-std-b67")
    }
}
impl Format {
    pub fn duration_ms(&self) -> i64 {
        (self.frames as f64 / self.sample_rate as f64 * 1000.).round() as i64
    }
}
/// FFmpeg stopped with an error. Everything it printed is kept for callers that look for a
/// cause; the message shows the first 500 characters.
#[derive(Debug)]
pub struct Failed(pub String);
impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let shown: String = self.0.chars().take(500).collect();
        write!(f, "Media processing failed: {shown}")
    }
}
impl std::error::Error for Failed {}
pub async fn ffmpeg(args: &[String]) -> Result<()> {
    let output = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-v", "error", "-y"])
        .args(args)
        .kill_on_drop(true)
        .output()
        .await
        .context("FFmpeg is required on the server")?;
    if !output.status.success() {
        // ponytail: all of stderr is held, as `output()` has always buffered it; read it into a
        // bounded buffer if a file can make FFmpeg print megabytes even at `-v error`.
        return Err(Failed(String::from_utf8_lossy(&output.stderr).into_owned()).into());
    }
    Ok(())
}
pub async fn inspect(path: &Path) -> Result<Format> {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries"])
        .arg(concat!(
            "stream=index,codec_type,codec_name,sample_rate,channels,bits_per_raw_sample,",
            "bits_per_sample,sample_fmt,pix_fmt,width,height,sample_aspect_ratio,color_transfer,start_time,duration",
            ":stream_disposition=attached_pic:stream_side_data=rotation:format=start_time,duration",
            ":format_tags=major_brand,creation_time,com.apple.quicktime.creationdate"
        ))
        .args(["-of", "json"])
        .arg(path)
        .kill_on_drop(true)
        .output()
        .await?;
    ensure!(
        output.status.success(),
        "Unreadable or unfinished media file"
    );
    let v: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    // The stream `-map 0:a:0` decodes: the first audio stream.
    let s = streams
        .iter()
        .find(|s| s["codec_type"] == "audio")
        .context("The file has no audio to transcribe")?;
    let rate = s["sample_rate"]
        .as_str()
        .unwrap_or("")
        .parse::<u32>()
        .context("Missing audio sample rate")?;
    let channels = s["channels"].as_u64().context("Missing channel count")? as u16;
    ensure!(
        (8000..=384000).contains(&rate) && (1..=8).contains(&channels),
        "Unsupported audio format"
    );
    let sample = s["sample_fmt"].as_str().unwrap_or("");
    let is_float = sample.starts_with("flt") || sample.starts_with("dbl");
    let bit_depth = s["bits_per_raw_sample"]
        .as_str()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|n| *n > 0)
        .or_else(|| {
            s["bits_per_sample"]
                .as_u64()
                .filter(|n| *n > 0)
                .map(|n| n as u16)
        })
        .unwrap_or(if sample.starts_with("dbl") {
            64
        } else if is_float || sample.starts_with("s32") {
            32
        } else {
            16
        });
    Ok(Format {
        sample_rate: rate,
        channels,
        bit_depth,
        is_float,
        frames: 0,
        codec: text(&s["codec_name"]),
        start_ms: millis(&s["start_time"]),
        span_ms: millis(&s["duration"]),
        video: picture(&streams, &v["format"]),
        made_at: made_at(&v["format"], millis(&s["start_time"])),
    })
}
/// Apple's `creationdate` is the start, with its offset. `creation_time` is UTC and written as
/// the file is finished (an Apple export of a 44-minute recording put it 44 minutes after the
/// start its name gave), so the start is that less the file's length.
/// ponytail: some cameras write the start into `creation_time`, or local time marked UTC; a
/// per-maker exception goes here if one shows up.
/// Both give when the file began; the transcript begins with its sound, `audio_ms` into the
/// file's timeline, so that is added.
fn made_at(file: &serde_json::Value, audio_ms: i64) -> Option<DateTime<Utc>> {
    let tags = &file["tags"];
    // Tools that do not know the time write 1904 or 1970.
    let known = |at: &DateTime<chrono::FixedOffset>| at.year() >= 2000;
    let apple = tags["com.apple.quicktime.creationdate"]
        .as_str()
        .and_then(|s| {
            DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z")
                .or_else(|_| DateTime::parse_from_rfc3339(s))
                .ok()
        })
        .filter(known);
    let finished = || {
        let at = DateTime::parse_from_rfc3339(tags["creation_time"].as_str()?).ok()?;
        // Without the file's length, its end says nothing about its start.
        let length: f64 = file["duration"].as_str()?.parse().ok()?;
        Some(at - chrono::Duration::milliseconds((length * 1000.).round() as i64)).filter(known)
    };
    let leader = chrono::Duration::milliseconds(audio_ms - millis(&file["start_time"]));
    Some((apple.or_else(finished)? + leader).with_timezone(&Utc))
}
fn text(value: &serde_json::Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}
/// FFprobe's seconds, as "1.000000"; 0 when it has none.
fn millis(value: &serde_json::Value) -> i64 {
    let seconds = value.as_str().and_then(|s| s.parse::<f64>().ok());
    seconds.map_or(0, |s| (s * 1000.).round() as i64)
}
/// Cover art (`attached_pic`, common in m4a and mp3) is not a picture.
fn picture(streams: &[serde_json::Value], file: &serde_json::Value) -> Option<Video> {
    let s = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)?;
    let size = |key: &str| s[key].as_u64().filter(|n| *n > 0).map(|n| n as u32);
    let sar = text(&s["sample_aspect_ratio"])
        .split_once(':')
        .and_then(|(num, den)| Some([num.parse().ok()?, den.parse().ok()?]))
        .filter(|[num, den]: &[u32; 2]| *num > 0 && *den > 0)
        .unwrap_or([1, 1]);
    let rotation = s["side_data_list"]
        .as_array()
        .and_then(|list| list.iter().find_map(|d| d["rotation"].as_f64()))
        .unwrap_or(0.)
        .round() as i32;
    Some(Video {
        stream: s["index"].as_u64()?,
        codec: text(&s["codec_name"]),
        pix_fmt: text(&s["pix_fmt"]),
        width: size("width")?,
        height: size("height")?,
        sar,
        rotation,
        transfer: text(&s["color_transfer"]),
        brand: text(&file["tags"]["major_brand"]),
        streams: streams.len(),
        start_ms: millis(&s["start_time"]),
        file_start_ms: millis(&file["start_time"]),
    })
}

// Canonical integer samples are signed 32-bit LE, left aligned (FFmpeg s32le).
// Float samples retain their source width. Format fields domain-separate the hash.
pub async fn verify(path: &Path) -> Result<(Format, String)> {
    let mut f = inspect(path).await?;
    let codec = if f.is_float {
        if f.bit_depth > 32 { "f64le" } else { "f32le" }
    } else {
        "s32le"
    };
    let mut child = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-f", codec, "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut out = child.stdout.take().context("Missing audio output")?;
    let mut hash = Sha256::new();
    hash.update(format!(
        "vault-pcm-v1:{}:{}:{}:{}\n",
        f.sample_rate, f.channels, f.bit_depth, f.is_float
    ));
    let mut buf = [0u8; 65536];
    let mut bytes = 0u64;
    let bytes_per_frame =
        u64::from(f.channels) * if f.is_float && f.bit_depth > 32 { 8 } else { 4 };
    loop {
        let n = out.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        ensure!(
            bytes <= u64::from(f.sample_rate) * bytes_per_frame * 86400,
            "Recording exceeds 24 hours"
        );
        hash.update(&buf[..n]);
    }
    ensure!(
        child.wait().await?.success(),
        "Audio could not be decoded completely"
    );
    ensure!(
        bytes > 0 && bytes.is_multiple_of(bytes_per_frame),
        "Empty or truncated audio"
    );
    f.frames = bytes / bytes_per_frame;
    Ok((f, hex::encode(hash.finalize())))
}
pub async fn excerpt(path: &Path, start_ms: i64, duration_ms: i64, to: &Path) -> Result<()> {
    ffmpeg(&[
        "-ss".into(),
        format!("{:.3}", start_ms as f64 / 1000.),
        "-i".into(),
        path.display().to_string(),
        "-t".into(),
        format!("{:.3}", duration_ms as f64 / 1000.),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        "16000".into(),
        "-c:a".into(),
        "pcm_s16le".into(),
        to.display().to_string(),
    ])
    .await
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn made_at_reads_the_start_from_the_files_tags() {
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        // The uploaded Apple export: finished at 08:07:38 UTC after 43:58.98.
        let export = json!({"duration": "2638.981667",
            "tags": {"creation_time": "2026-11-03T08:07:38.000000Z"}});
        assert_eq!(made_at(&export, 0), Some(at("2026-11-03T07:23:39.018Z")));
        let iphone = json!({"duration": "60.0", "tags": {
            "creation_time": "2026-11-03T08:08:38Z",
            "com.apple.quicktime.creationdate": "2026-11-03T08:22:32+0100"}});
        assert_eq!(made_at(&iphone, 0), Some(at("2026-11-03T07:22:32Z")));
        let unset = json!({"duration": "1.0", "tags": {"creation_time": "1970-01-01T00:00:00Z"}});
        assert_eq!(made_at(&unset, 0), None);
        assert_eq!(made_at(&json!({"duration": "1.0"}), 0), None);
        // Without the file's length the end is not a start.
        let unknown = json!({"tags": {"creation_time": "2026-11-03T08:07:38Z"}});
        assert_eq!(made_at(&unknown, 0), None);
        // An Apple tag that does not know the time leaves creation_time to answer.
        let blank = json!({"duration": "60.0", "tags": {
            "creation_time": "2026-11-03T08:08:38Z",
            "com.apple.quicktime.creationdate": "1904-01-01T00:00:00+0000"}});
        assert_eq!(made_at(&blank, 0), Some(at("2026-11-03T08:07:38Z")));
        let zulu = json!({"duration": "60.0", "tags": {
            "creation_time": "2026-11-03T08:08:38Z",
            "com.apple.quicktime.creationdate": "2026-11-03T07:22:32.000Z"}});
        assert_eq!(made_at(&zulu, 0), Some(at("2026-11-03T07:22:32Z")));
        // A file that began 5 minutes before its sound: the transcript starts with the sound.
        let leader = json!({"start_time": "0.000000", "duration": "1200.0",
            "tags": {"creation_time": "2026-11-03T10:20:00Z"}});
        assert_eq!(made_at(&leader, 300_000), Some(at("2026-11-03T10:05:00Z")));
    }
}
