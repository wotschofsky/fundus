use anyhow::{Context, Result, bail, ensure};
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
    #[serde(default)]
    pub video: Option<Video>,
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
pub async fn ffmpeg(args: &[String]) -> Result<()> {
    let output = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-v", "error", "-y"])
        .args(args)
        .kill_on_drop(true)
        .output()
        .await
        .context("FFmpeg is required on the server")?;
    if !output.status.success() {
        bail!(
            "Media processing failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(500)
                .collect::<String>()
        );
    }
    Ok(())
}
pub async fn inspect(path: &Path) -> Result<Format> {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries"])
        .arg(concat!(
            "stream=index,codec_type,codec_name,sample_rate,channels,bits_per_raw_sample,",
            "bits_per_sample,sample_fmt,pix_fmt,width,height,sample_aspect_ratio,color_transfer,start_time",
            ":stream_disposition=attached_pic:stream_side_data=rotation:format=start_time:format_tags=major_brand"
        ))
        .args(["-of", "json"])
        .arg(path)
        .kill_on_drop(true)
        .output()
        .await?;
    ensure!(
        output.status.success(),
        "Unreadable or unfinished audio file"
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
        video: picture(&streams, &v["format"]),
    })
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
