use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, process::Stdio};
use tokio::{io::AsyncReadExt, process::Command};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Format {
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_depth: u16,
    pub is_float: bool,
    pub frames: u64,
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
    let output=Command::new("ffprobe").args(["-v","error","-select_streams","a:0","-show_entries","stream=sample_rate,channels,bits_per_raw_sample,bits_per_sample,sample_fmt,duration,duration_ts,time_base","-of","json"]).arg(path).kill_on_drop(true).output().await?;
    ensure!(
        output.status.success(),
        "Unreadable or unfinished audio file"
    );
    let v: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let s = &v["streams"][0];
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
    })
}
/// A file's first real video stream. `copy_*`: plays in every browser as it is.
pub struct Video {
    pub index: u64,
    pub copy_video: bool,
    pub copy_audio: bool,
}
/// The video in a recording, if any. Cover art (`attached_pic`, common in m4a and mp3) is not.
pub async fn video(path: &Path) -> Result<Option<Video>> {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries"])
        .arg("stream=index,codec_type,codec_name,pix_fmt:stream_disposition=attached_pic")
        .args(["-of", "json"])
        .arg(path)
        .kill_on_drop(true)
        .output()
        .await?;
    ensure!(output.status.success(), "Unreadable video file");
    let v: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let Some(video) = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
    else {
        return Ok(None);
    };
    // The audio `verify` and `render` use: the first audio stream.
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    Ok(Some(Video {
        index: video["index"].as_u64().context("Missing stream index")?,
        // 8-bit 4:2:0 H.264 plays everywhere; 10-bit or 4:2:2 does not.
        copy_video: video["codec_name"] == "h264"
            && ["yuv420p", "yuvj420p"].contains(&video["pix_fmt"].as_str().unwrap_or("")),
        copy_audio: audio.is_some_and(|a| a["codec_name"] == "aac"),
    }))
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
