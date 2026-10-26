use crate::{
    audio,
    config::{App, id, now},
};
use anyhow::{Context, Result, ensure};
use sqlx::Row;
use std::{
    collections::BTreeMap,
    ops::Bound,
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
/// Clean speech for comparing voices, as calibrated in mic-ingest: turns of 3 s or more, transcribed
/// with confidence 0.6 or more, not overlapping a neighbouring turn of someone else; at most 10 s each.
const EXCERPT_MIN_MS: i64 = 3000;
const EXCERPT_MAX_MS: i64 = 10000;
const MIN_CONFIDENCE: f64 = 0.6;
/// Spread over the whole recording, so a label is measured in every surrounding it occurs in.
const EXCERPTS: usize = 24;
/// Two labels this similar are one voice. In real recordings different people in one room scored
/// up to 0.59 and halves of one label 0.63-0.99; in mic-ingest's calibration no impostor reached it.
pub const SAME_VOICE: f32 = 0.78;

/// Up to `EXCERPTS` clean excerpts per label as (start, duration), from rows of (label, start, end,
/// confidence) in transcript order. A label needs two excerpts and 10 s of them to be compared.
fn excerpts(rows: &[(String, i64, i64, Option<f64>)]) -> BTreeMap<String, Vec<(i64, i64)>> {
    let mut found: BTreeMap<String, Vec<(i64, i64)>> = BTreeMap::new();
    for (i, (speaker, start, end, confidence)) in rows.iter().enumerate() {
        // Turns are ordered: overlap can only involve the neighbours.
        let overlaps = |j: usize| {
            rows.get(j)
                .is_some_and(|(other, s, e, _)| other != speaker && s < end && e > start)
        };
        if end - start < EXCERPT_MIN_MS
            || confidence.is_some_and(|c| c < MIN_CONFIDENCE)
            || overlaps(i.wrapping_sub(1))
            || overlaps(i + 1)
        {
            continue;
        }
        found
            .entry(speaker.clone())
            .or_default()
            .push((*start, (end - start).min(EXCERPT_MAX_MS)));
    }
    found.retain(|_, times| times.len() >= 2 && times.iter().map(|(_, d)| d).sum::<i64>() >= 10000);
    for times in found.values_mut() {
        if times.len() > EXCERPTS {
            let n = times.len();
            *times = (0..EXCERPTS).map(|k| times[k * n / EXCERPTS]).collect();
        }
    }
    found
}

/// One label's voice: the sum of its excerpts' unit embeddings, their number, and the label's speech.
#[derive(Clone, Debug)]
pub struct Cluster {
    pub sum: Vec<f32>,
    pub excerpts: usize,
    pub speech_ms: i64,
}

/// Merges labels that are one voice, the most similar pair first, while a pair reaches `SAME_VOICE`.
/// The label the owner decided, else the one with more speech, absorbs the other; labels the owner
/// decided differently never merge. Returns (absorbed, absorbing) pairs in merge order.
pub fn merge_labels(
    clusters: &mut BTreeMap<String, Cluster>,
    decided: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let mut merged = vec![];
    loop {
        let mut best: Option<(f32, String, String)> = None;
        for (a, x) in clusters.iter() {
            for (b, y) in clusters.range::<String, _>((Bound::Excluded(a), Bound::Unbounded)) {
                if decided
                    .get(a)
                    .zip(decided.get(b))
                    .is_some_and(|(p, q)| p != q)
                {
                    continue;
                }
                let score = cosine(&x.sum, &y.sum);
                if score >= SAME_VOICE && best.as_ref().is_none_or(|(s, _, _)| score > *s) {
                    best = Some((score, a.clone(), b.clone()));
                }
            }
        }
        let Some((_, a, b)) = best else {
            return merged;
        };
        let keep_a = match (decided.contains_key(&a), decided.contains_key(&b)) {
            (true, false) => true,
            (false, true) => false,
            _ => clusters[&a].speech_ms >= clusters[&b].speech_ms,
        };
        let (from, into) = if keep_a { (b, a) } else { (a, b) };
        let gone = clusters.remove(&from).expect("label present");
        let kept = clusters.get_mut(&into).expect("label present");
        for (x, y) in kept.sum.iter_mut().zip(&gone.sum) {
            *x += y;
        }
        kept.excerpts += gone.excerpts;
        kept.speech_ms += gone.speech_ms;
        merged.push((from, into));
    }
}

/// Every known person's similarity to `probe`, closest first. A person scores their closest stored
/// sample, so a profile from several surroundings matches in any of them.
pub fn candidates(probe: &[f32], profiles: &BTreeMap<String, Vec<Vec<f32>>>) -> Vec<(String, f32)> {
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
    scores
}
/// ONNX Runtime allows one environment per process, ever: it is created on first use and kept.
static ENVIRONMENT: std::sync::LazyLock<Result<ort::environment::Environment, String>> =
    std::sync::LazyLock::new(|| ort::init().build().map_err(|error| error.to_string()));

/// Unit embeddings of each label's clips.
fn embed(model: &Path, clips: Vec<(String, Vec<PathBuf>)>) -> Result<Vec<(String, Vec<Vec<f32>>)>> {
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
            result.push((speaker, vectors));
        }
    }
    Ok(result)
}
/// What voice analysis found in one recording.
#[derive(Default)]
pub struct Analysis {
    /// Labels found to be one voice, already merged in the transcript: (absorbed, absorbing).
    pub merged: Vec<(String, String)>,
    /// The remaining labels with enough clean speech to compare.
    pub labels: BTreeMap<String, LabelVoice>,
}
pub struct LabelVoice {
    pub vector: Vec<f32>,
    pub excerpts: usize,
    /// Known people from other recordings, closest first.
    pub candidates: Vec<(String, f32)>,
}

/// Measures each label's voice in `audio_path` (the playback file), merges labels that are one
/// voice and compares the rest with known people. Who a speaker is gets decided in `identity`.
pub async fn analyze(app: &App, entry: &str, audio_path: &Path) -> Result<Analysis> {
    let _cpu = app.cpu.acquire().await?;
    let model = app
        .config
        .voice_model
        .clone()
        .context("Configure VAULT_VOICE_MODEL and ONNX Runtime for local recognition")?;
    ensure!(model.is_file(), "Speaker model file is missing");
    let rows: Vec<(String, i64, i64, Option<f64>)> = sqlx::query_as(
        "SELECT speaker,start_ms,end_ms,confidence FROM segments WHERE entry_id=? ORDER BY position",
    )
    .bind(entry)
    .fetch_all(&app.db)
    .await?;
    let dir = app.entry_dir(entry);
    let result = measure(app, entry, audio_path, model, excerpts(&rows)).await;
    // Keep only durable vectors, also after a failure; excerpts can be rebuilt from the archive.
    let mut items = tokio::fs::read_dir(&dir).await?;
    while let Some(item) = items.next_entry().await? {
        if item.file_name().to_string_lossy().starts_with("voice-") {
            let _ = tokio::fs::remove_file(item.path()).await;
        }
    }
    let mut clusters = result?;
    for (speaker, start, end, _) in &rows {
        if let Some(cluster) = clusters.get_mut(speaker) {
            cluster.speech_ms += end - start;
        }
    }
    let decided: BTreeMap<String, String> = sqlx::query_as(
        "SELECT label,COALESCE(person_id,'') FROM speakers WHERE entry_id=? AND state IN ('manual','manual_unknown')",
    )
    .bind(entry)
    .fetch_all(&app.db)
    .await?
    .into_iter()
    .collect();
    let merged = merge_labels(&mut clusters, &decided);
    let mut tx = app.db.begin().await?;
    for (from, into) in &merged {
        tracing::info!(%entry, %from, %into, "merged speaker labels with one voice");
        sqlx::query("UPDATE segments SET speaker=? WHERE entry_id=? AND speaker=?")
            .bind(into)
            .bind(entry)
            .bind(from)
            .execute(&mut *tx)
            .await?;
        for (table, column) in [
            ("speakers", "label"),
            ("voice_probes", "speaker"),
            ("voice_samples", "speaker"),
        ] {
            sqlx::query(&format!(
                "DELETE FROM {table} WHERE entry_id=? AND {column}=?"
            ))
            .bind(entry)
            .bind(from)
            .execute(&mut *tx)
            .await?;
        }
    }
    let mut labels = BTreeMap::new();
    for (label, cluster) in clusters {
        let vector = normalized(cluster.sum);
        sqlx::query("INSERT INTO voice_probes(entry_id,speaker,model,vector_json)VALUES(?,?,?,?) ON CONFLICT(entry_id,speaker,model)DO UPDATE SET vector_json=excluded.vector_json").bind(entry).bind(&label).bind(MODEL).bind(serde_json::to_string(&vector)?).execute(&mut *tx).await?;
        labels.insert(
            label,
            LabelVoice {
                vector,
                excerpts: cluster.excerpts,
                candidates: vec![],
            },
        );
    }
    tx.commit().await?;
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
    for voice in labels.values_mut() {
        voice.candidates = candidates(&voice.vector, &profiles);
    }
    Ok(Analysis { merged, labels })
}
async fn measure(
    app: &App,
    entry: &str,
    audio_path: &Path,
    model: PathBuf,
    excerpts: BTreeMap<String, Vec<(i64, i64)>>,
) -> Result<BTreeMap<String, Cluster>> {
    let mut clips = vec![];
    let dir = app.entry_dir(entry);
    for (speaker, times) in excerpts {
        let mut files = vec![];
        for (i, (start, duration)) in times.into_iter().enumerate() {
            let path = dir.join(format!("voice-{}-{i}.wav", crate::auth::hash(&speaker)));
            audio::excerpt(audio_path, start, duration, &path).await?;
            files.push(path);
        }
        clips.push((speaker, files));
    }
    let outputs = tokio::task::spawn_blocking(move || embed(&model, clips)).await??;
    Ok(outputs
        .into_iter()
        .map(|(speaker, vectors)| {
            let mut sum = vec![0.; vectors[0].len()];
            for v in &vectors {
                for (x, y) in sum.iter_mut().zip(v) {
                    *x += y;
                }
            }
            let cluster = Cluster {
                sum,
                excerpts: vectors.len(),
                speech_ms: 0,
            };
            (speaker, cluster)
        })
        .collect())
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
    fn cluster(sum: Vec<f32>, speech_ms: i64) -> Cluster {
        Cluster {
            sum,
            excerpts: 4,
            speech_ms,
        }
    }
    #[test]
    fn excerpts_are_clean_long_turns_spread_over_the_recording() {
        let row = |s: &str, start: i64, end: i64, c: Option<f64>| (s.to_string(), start, end, c);
        let mut rows = vec![
            row("A", 0, 2_000, None),            // too short
            row("A", 2_000, 8_000, Some(0.4)),   // unsure transcription
            row("A", 8_000, 14_000, None),       // overlaps B's next turn
            row("B", 13_000, 20_000, Some(0.9)), // overlaps A
            row("B", 20_000, 26_000, Some(0.9)), // too little in total for B
            row("C", 26_000, 31_000, Some(0.9)),
            row("C", 31_000, 50_000, Some(0.9)), // capped at 10 s
        ];
        let found = excerpts(&rows);
        assert_eq!(found.keys().collect::<Vec<_>>(), ["C"]);
        assert_eq!(found["C"], [(26_000, 5_000), (31_000, 10_000)]);
        // A long day contributes excerpts from all of it, not only from its start.
        rows = (0..100)
            .map(|i| row("D", i * 60_000, i * 60_000 + 5_000, None))
            .collect();
        let spread = &excerpts(&rows)["D"];
        assert_eq!(spread.len(), EXCERPTS);
        assert!(spread.last().unwrap().0 > 90 * 60_000);
    }
    #[test]
    fn one_voice_under_several_labels_is_merged_into_the_main_label() {
        let mut clusters = BTreeMap::from([
            ("A".to_string(), cluster(vec![1., 0., 0.], 60_000)),
            ("B".to_string(), cluster(vec![0., 1., 0.], 50_000)),
            // Same voice as A, less speech: absorbed by A. Different people stay apart.
            ("C".to_string(), cluster(vec![0.95, 0.1, 0.], 5_000)),
            ("D".to_string(), cluster(vec![0.5, 0.5, 0.707], 5_000)),
        ]);
        let merged = merge_labels(&mut clusters, &BTreeMap::new());
        assert_eq!(merged, [("C".to_string(), "A".to_string())]);
        assert_eq!(clusters.keys().collect::<Vec<_>>(), ["A", "B", "D"]);
        assert_eq!(clusters["A"].speech_ms, 65_000);
    }
    #[test]
    fn the_owners_decisions_win_and_are_never_merged_across() {
        let decided = BTreeMap::from([("B".to_string(), "lena".to_string())]);
        let mut clusters = BTreeMap::from([
            ("A".to_string(), cluster(vec![1., 0.], 90_000)),
            ("B".to_string(), cluster(vec![0.99, 0.05], 1_000)),
        ]);
        // The named label keeps its name, even with less speech.
        assert_eq!(
            merge_labels(&mut clusters, &decided),
            [("A".to_string(), "B".to_string())]
        );
        let decided = BTreeMap::from([
            ("A".to_string(), "felix".to_string()),
            ("B".to_string(), "lena".to_string()),
        ]);
        let mut clusters = BTreeMap::from([
            ("A".to_string(), cluster(vec![1., 0.], 90_000)),
            ("B".to_string(), cluster(vec![0.99, 0.05], 1_000)),
        ]);
        assert!(merge_labels(&mut clusters, &decided).is_empty());
    }
    #[test]
    fn candidates_score_a_persons_closest_sample() {
        let profiles = BTreeMap::from([
            ("a".into(), vec![vec![0., 1.], vec![1., 0.]]),
            ("b".into(), vec![vec![0.6, 0.8]]),
        ]);
        let found = candidates(&[1., 0.], &profiles);
        assert_eq!(found[0], ("a".into(), 1.));
        assert_eq!(found[1].0, "b");
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
        assert_eq!(vectors[0].1[0].len(), 256);
        assert!(cosine(&vectors[0].1[0], &vectors[1].1[0]) > 0.999);
    }
}
