use crate::{
    audio,
    config::{App, id, now},
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sqlx::Row;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub const MODEL: &str = "wespeaker-resnet34-lm-onnx-kaldi-v1";
pub fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0. {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.;
    }
    let d = a.iter().zip(b).map(|(a, b)| a * b).sum::<f32>();
    let n =
        a.iter().map(|x| x * x).sum::<f32>().sqrt() * b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0. { d / n } else { 0. }
}
pub fn match_person(
    probe: &[f32],
    profiles: &BTreeMap<String, Vec<Vec<f32>>>,
    strong: f32,
    margin: f32,
) -> Option<(String, f32)> {
    let mut scores: Vec<_> = profiles
        .iter()
        .map(|(p, s)| {
            (
                p.clone(),
                s.iter().map(|v| cosine(probe, v)).fold(-1., f32::max),
            )
        })
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let best = scores.first()?;
    if best.1 >= strong && (scores.len() == 1 || best.1 - scores[1].1 >= margin) {
        Some(best.clone())
    } else {
        None
    }
}
/// ONNX Runtime allows one environment per process, ever: it is created on first use and kept.
static ENVIRONMENT: std::sync::LazyLock<Result<ort::environment::Environment, String>> =
    std::sync::LazyLock::new(|| ort::init().build().map_err(|error| error.to_string()));

fn embed(model: &Path, clips: Vec<(String, Vec<PathBuf>)>) -> Result<Vec<(String, Vec<f32>)>> {
    use kaldi_fbank::{FbankOptions, OnlineFbank, WindowType};
    use ort::{session::Session, value::Tensor};
    let environment = ENVIRONMENT
        .as_ref()
        .map_err(|error| anyhow::anyhow!("ONNX Runtime unavailable: {error}"))?;
    let mut session = Session::builder(environment)?
        .with_intra_threads(2)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .commit_from_file(model)?;
    let mut result = vec![];
    for (speaker, paths) in clips {
        let mut vectors = vec![];
        for path in paths {
            let mut reader = hound::WavReader::open(path)?;
            let samples: Vec<f32> = reader
                .samples::<i16>()
                .map(|x| x.map(|n| n as f32))
                .collect::<std::result::Result<_, _>>()?;
            if samples.is_empty() {
                continue;
            }
            let wave: Vec<f32> = (0..160000).map(|i| samples[i % samples.len()]).collect();
            let mut options = FbankOptions::default();
            options.frame_opts.samp_freq = 16000.;
            options.frame_opts.dither = 0.;
            options.frame_opts.window_type = WindowType::Hamming;
            options.frame_opts.snip_edges = true;
            options.mel_opts.num_bins = 80;
            options.use_energy = false;
            let mut fbank = OnlineFbank::new(options);
            fbank.accept_waveform(16000., &wave);
            fbank.input_finished();
            let frames = fbank.num_frames_ready() as usize;
            ensure!(frames > 0, "Empty acoustic features");
            let mut features: Vec<f32> = (0..frames)
                .flat_map(|i| fbank.get_frame(i as i32).to_vec())
                .collect();
            for bin in 0..80 {
                let mean = (0..frames).map(|i| features[i * 80 + bin]).sum::<f32>() / frames as f32;
                for i in 0..frames {
                    features[i * 80 + bin] -= mean;
                }
            }
            let input = Tensor::from_array(([1usize, frames, 80], features.into_boxed_slice()))?;
            let outputs = session.run(ort::inputs![input])?;
            let (_, v) = outputs[0].try_extract_tensor::<f32>()?;
            ensure!(
                v.len() == 256 && v.iter().all(|x| x.is_finite()),
                "Invalid speaker embedding"
            );
            vectors.push(normalized(v.to_vec()));
        }
        if !vectors.is_empty() {
            let mut mean = vec![0.; 256];
            for v in vectors {
                for (i, x) in v.iter().enumerate() {
                    mean[i] += x;
                }
            }
            result.push((speaker, normalized(mean)));
        }
    }
    Ok(result)
}
pub async fn analyze(app: &App, entry: &str, audio_path: &Path) -> Result<()> {
    let _cpu = app.cpu.acquire().await?;
    let model = app
        .config
        .voice_model
        .clone()
        .context("Configure VAULT_VOICE_MODEL and ONNX Runtime for local recognition")?;
    ensure!(model.is_file(), "Speaker model file is missing");
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT speaker,start_ms,end_ms FROM segments WHERE entry_id=? ORDER BY position",
    )
    .bind(entry)
    .fetch_all(&app.db)
    .await?;
    let mut excerpts: BTreeMap<String, Vec<(i64, i64)>> = BTreeMap::new();
    for (speaker, start, end) in &rows {
        // Only clean speech: long enough and not overlapping anyone else.
        // ponytail: O(n²) over utterances, ~25M cheap comparisons for a long day; sort by start if it shows.
        if end - start < 3000
            || rows
                .iter()
                .any(|(other, s, e)| other != speaker && s < end && e > start)
        {
            continue;
        }
        excerpts
            .entry(speaker.clone())
            .or_default()
            .push((*start, (end - start).min(10000)));
    }
    let dir = app.entry_dir(entry);
    let result = probe_and_match(app, entry, audio_path, model, excerpts).await;
    // Keep only durable vectors, also after a failure; excerpts can be rebuilt from the archive.
    let mut items = tokio::fs::read_dir(&dir).await?;
    while let Some(item) = items.next_entry().await? {
        if item.file_name().to_string_lossy().starts_with("voice-") {
            let _ = tokio::fs::remove_file(item.path()).await;
        }
    }
    result
}
async fn probe_and_match(
    app: &App,
    entry: &str,
    audio_path: &Path,
    model: PathBuf,
    excerpts: BTreeMap<String, Vec<(i64, i64)>>,
) -> Result<()> {
    let mut clips = vec![];
    let dir = app.entry_dir(entry);
    for (speaker, mut times) in excerpts {
        times.sort_by_key(|(_, d)| std::cmp::Reverse(*d));
        times.truncate(8);
        if times.len() < 2 || times.iter().map(|(_, d)| d).sum::<i64>() < 10000 {
            continue;
        }
        let mut files = vec![];
        for (i, (start, duration)) in times.into_iter().enumerate() {
            let path = dir.join(format!("voice-{}-{i}.wav", crate::auth::hash(&speaker)));
            audio::excerpt(audio_path, start, duration, &path).await?;
            files.push(path);
        }
        clips.push((speaker, files));
    }
    let outputs = tokio::task::spawn_blocking(move || embed(&model, clips)).await??;
    let samples = sqlx::query(
        "SELECT vs.person_id,vs.vector_json FROM voice_samples vs JOIN entries e ON e.id=vs.entry_id WHERE vs.entry_id!=? AND vs.model=? AND e.trashed_at IS NULL",
    )
    .bind(entry)
    .bind(MODEL)
    .fetch_all(&app.db)
    .await?;
    let mut profiles: BTreeMap<String, Vec<Vec<f32>>> = BTreeMap::new();
    for row in samples {
        let vector: Vec<f32> = serde_json::from_str(&row.get::<String, _>("vector_json"))?;
        profiles
            .entry(row.get("person_id"))
            .or_default()
            .push(vector);
    }
    let thresholds = crate::settings::load(app).await?.voice_thresholds;
    let strong = thresholds.strong;
    let margin = thresholds.margin;
    for (label, vector) in outputs {
        sqlx::query("INSERT INTO voice_probes(entry_id,speaker,model,vector_json)VALUES(?,?,?,?) ON CONFLICT(entry_id,speaker,model)DO UPDATE SET vector_json=excluded.vector_json").bind(entry).bind(&label).bind(MODEL).bind(serde_json::to_string(&vector)?).execute(&app.db).await?;
        if let Some((person, score)) = match_person(&vector, &profiles, strong, margin) {
            sqlx::query("UPDATE speakers SET person_id=?,state='automatic',evidence=? WHERE entry_id=? AND label=? AND state NOT IN ('manual','manual_unknown')").bind(person).bind(json!([{"source":"voice","score":score,"model":MODEL}]).to_string()).bind(entry).bind(label).execute(&app.db).await?;
        }
    }
    Ok(())
}
pub async fn assign(app: &App, entry: &str, label: &str, person: Option<&str>) -> Result<()> {
    let mut tx = app.db.begin().await?;
    let existing: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM speakers WHERE entry_id=? AND label=?")
            .bind(entry)
            .bind(label)
            .fetch_one(&mut *tx)
            .await?;
    ensure!(existing == 1, "Speaker not found");
    sqlx::query(
        "UPDATE speakers SET person_id=?,state=?,evidence='[]' WHERE entry_id=? AND label=?",
    )
    .bind(person)
    .bind(if person.is_some() {
        "manual"
    } else {
        "manual_unknown"
    })
    .bind(entry)
    .bind(label)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM voice_samples WHERE entry_id=? AND speaker=?")
        .bind(entry)
        .bind(label)
        .execute(&mut *tx)
        .await?;
    if let Some(person) = person {
        let vector: Option<String> = sqlx::query_scalar(
            "SELECT vector_json FROM voice_probes WHERE entry_id=? AND speaker=? AND model=?",
        )
        .bind(entry)
        .bind(label)
        .bind(MODEL)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(vector) = vector {
            sqlx::query("INSERT INTO voice_samples(id,person_id,entry_id,speaker,model,vector_json,provenance,created_at)VALUES(?,?,?,?,?,?,'manual',?)").bind(id()).bind(person).bind(entry).bind(label).bind(MODEL).bind(vector).bind(now()).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM voice_samples WHERE person_id=? AND model=? AND id NOT IN(SELECT id FROM voice_samples WHERE person_id=? AND model=? ORDER BY created_at DESC LIMIT 24)").bind(person).bind(MODEL).bind(person).bind(MODEL).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_ambiguous_and_unknown() {
        let p = BTreeMap::from([
            ("a".into(), vec![vec![1., 0.]]),
            ("b".into(), vec![vec![0.99, 0.01]]),
        ]);
        assert!(match_person(&[1., 0.], &p, 0.8, 0.1).is_none());
        assert!(match_person(&[0., 1.], &p, 0.8, 0.1).is_none());
    }
    #[test]
    fn clear_known_voice_matches() {
        let p = BTreeMap::from([
            ("a".into(), vec![vec![1., 0.]]),
            ("b".into(), vec![vec![0., 1.]]),
        ]);
        assert_eq!(match_person(&[1., 0.], &p, 0.8, 0.1).unwrap().0, "a");
    }
}

#[cfg(test)]
mod runtime_test {
    use super::*;
    #[test]
    #[ignore = "requires VAULT_VOICE_MODEL and ORT_DYLIB_PATH"]
    fn pinned_model_runs_on_rust_features() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("synthetic.wav");
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..160000 {
            writer
                .write_sample(((i as f64 * 0.073).sin() * 16000.) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
        let model = std::env::var("VAULT_VOICE_MODEL").unwrap();
        let vectors = embed(
            Path::new(&model),
            vec![
                ("first".into(), vec![path.clone()]),
                ("second".into(), vec![path]),
            ],
        )
        .unwrap();
        assert_eq!(vectors[0].1.len(), 256);
        assert!(cosine(&vectors[0].1, &vectors[1].1) > 0.999);
    }
}
