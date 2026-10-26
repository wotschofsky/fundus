use crate::{
    audio,
    config::{App, id, now},
    grouping::{self, Source},
    provider::{self, Job, NameFinding, ProviderError, Utterance},
    voice,
};
use anyhow::{Result, bail, ensure};
use futures_util::future::join_all;
use serde_json::json;
use sqlx::Row;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

const AMBIGUOUS: &str = "A transcription request may have been submitted. Retry explicitly to authorize another request.";
/// Entries processed at the same time. Most of a job is waiting for AssemblyAI or Groq; local
/// CPU work and Groq requests have their own limits on `App`.
const CONCURRENCY: usize = 6;
/// AssemblyAI's limit for one job.
const MAX_DURATION_MS: i64 = 10 * 60 * 60 * 1000;

pub async fn worker(app: App, shutdown: CancellationToken) {
    recover(&app).await;
    let mut jobs = tokio::task::JoinSet::new();
    loop {
        if shutdown.is_cancelled() {
            break;
        }
        let import: Option<String> = sqlx::query_scalar(
            "SELECT id FROM imports WHERE state='verifying' ORDER BY created_at LIMIT 1",
        )
        .fetch_optional(&app.db)
        .await
        .unwrap_or(None);
        if let Some(batch) = import {
            if let Err(error) = finalize(&app, &batch).await {
                tracing::error!(import=%batch,error=%error,"import verification failed");
                let _ = sqlx::query("UPDATE imports SET state='failed' WHERE id=?")
                    .bind(batch)
                    .execute(&app.db)
                    .await;
            }
            continue;
        }
        if jobs.len() < CONCURRENCY
            && let Some((entry, work)) = claim(&app).await
        {
            let (app, stop) = (app.clone(), shutdown.clone());
            jobs.spawn(async move { run(&app, &entry, work.as_deref(), &stop).await });
            continue;
        }
        tokio::select! {
            _ = app.wake.notified() => {}
            _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {}
            Some(Err(error)) = jobs.join_next(), if !jobs.is_empty() => tracing::error!(%error, "processing task panicked"),
            _ = shutdown.cancelled() => break,
        }
    }
    // Drain: every in-flight job finishes its current provider request and saves it before exit.
    while jobs.join_next().await.is_some() {}
}

/// After a crash, work that cannot bill twice resumes on its own, including a request whose job
/// is found by its saved upload URL. One saved without that URL waits for an explicit Retry.
async fn recover(app: &App) {
    let active = "stage NOT IN ('queued','complete','failed','paused') AND trashed_at IS NULL";
    let _ = sqlx::query(&format!("UPDATE entries SET stage='failed',failure='Server stopped while submitting the transcription. Retry explicitly to resubmit.' WHERE {active} AND EXISTS(SELECT 1 FROM transcriptions t WHERE t.entry_id=entries.id AND t.state='submitted' AND t.audio_url IS NULL)"))
        .execute(&app.db)
        .await;
    let _ = sqlx::query(&format!("UPDATE entries SET stage='queued' WHERE {active}"))
        .execute(&app.db)
        .await;
}

/// Atomically takes the oldest queued entry, so concurrent jobs never share one.
async fn claim(app: &App) -> Option<(String, Option<String>)> {
    sqlx::query_as("UPDATE entries SET stage='preparing',active_stage='preparing',failure=NULL,updated_at=? WHERE id=(SELECT id FROM entries WHERE stage='queued' AND trashed_at IS NULL ORDER BY created_at LIMIT 1) AND stage='queued' RETURNING id,requested_work")
        .bind(now())
        .fetch_optional(&app.db)
        .await
        .unwrap_or_else(|error| {
            tracing::error!(%error, "could not claim queued work");
            None
        })
}

async fn run(app: &App, entry: &str, work: Option<&str>, stop: &CancellationToken) {
    let result = if work == Some("speakers") {
        reanalyze_speakers(app, entry).await
    } else {
        process_until(app, entry, stop).await
    };
    if let Err(error) = result {
        tracing::error!(entry=%entry,error=%error,"entry processing failed");
        let _ = sqlx::query("UPDATE entries SET stage='failed',failure=?,updated_at=? WHERE id=?")
            .bind(error.to_string().chars().take(1000).collect::<String>())
            .bind(now())
            .bind(entry)
            .execute(&app.db)
            .await;
    }
}
pub async fn finalize(app: &App, batch: &str) -> Result<()> {
    let import = sqlx::query("SELECT recorder_timezone,session_plan FROM imports WHERE id=?")
        .bind(batch)
        .fetch_one(&app.db)
        .await?;
    let zone: String = import.get("recorder_timezone");
    let tz = zone.parse()?;
    let plans: Option<Vec<grouping::SessionPlan>> = import
        .get::<Option<String>, _>("session_plan")
        .map(|plan| serde_json::from_str(&plan))
        .transpose()?;
    let rows = sqlx::query("SELECT * FROM uploads WHERE import_id=? ORDER BY relative_path")
        .bind(batch)
        .fetch_all(&app.db)
        .await?;
    // Decoding is the slow part: verify the uploads in parallel, then record them in order.
    let verified = join_all(rows.iter().map(|row| async move {
        let _cpu = app.cpu.acquire().await?;
        audio::verify(&app.upload_path(&row.get::<String, _>("id"))).await
    }))
    .await;
    let mut sources = vec![];
    let mut seen = HashSet::new();
    for (row, verified) in rows.into_iter().zip(verified) {
        let uid: String = row.get("id");
        let original: String = row.get("original_name");
        let (format, hash) = match verified {
            Ok(verified) => verified,
            Err(error) => {
                sqlx::query("UPDATE uploads SET state='invalid',failure=? WHERE id=?")
                    .bind(error.to_string())
                    .bind(&uid)
                    .execute(&app.db)
                    .await?;
                continue;
            }
        };
        let claimed: Option<String> = row.get("claimed_pcm_hash");
        if claimed.as_ref().is_some_and(|claimed| claimed != &hash) {
            sqlx::query(
                "UPDATE uploads SET state='invalid',failure='PCM verification mismatch' WHERE id=?",
            )
            .bind(&uid)
            .execute(&app.db)
            .await?;
            continue;
        }
        sqlx::query(
            "UPDATE uploads SET state='verified',pcm_hash=?,format_json=?,failure=NULL WHERE id=?",
        )
        .bind(&hash)
        .bind(serde_json::to_string(&format)?)
        .bind(&uid)
        .execute(&app.db)
        .await?;
        if plans.is_none() {
            let known: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entry_sources es JOIN uploads u ON u.id=es.upload_id JOIN entries e ON e.id=es.entry_id WHERE u.pcm_hash=? AND e.trashed_at IS NULL")
                .bind(&hash).fetch_one(&app.db).await?;
            if known > 0 || !seen.insert(hash.clone()) {
                sqlx::query("UPDATE uploads SET state='duplicate' WHERE id=?")
                    .bind(&uid)
                    .execute(&app.db)
                    .await?;
                continue;
            }
        }
        let name = Path::new(&original)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&original);
        let parsed = grouping::parse_dji(name, tz);
        let source_id = row
            .get::<Option<String>, _>("source_id")
            .unwrap_or_else(|| uid.clone());
        sources.push((
            source_id,
            Source {
                id: uid,
                filename: original.clone(),
                hash,
                format,
                recorded_at: parsed.as_ref().map(|(time, _)| *time),
                stream: parsed.map(|(_, stream)| stream).unwrap_or(original),
            },
        ));
    }
    let groups = if let Some(plans) = &plans {
        grouping::planned_sessions(sources, plans)?
    } else {
        grouping::group(sources.into_iter().map(|(_, source)| source).collect())
            .into_iter()
            .map(|sources| {
                let frames = sources.iter().map(|source| source.format.frames).sum();
                grouping::PlannedSession {
                    sources,
                    start_frame: 0,
                    end_frame: frames,
                    total_frames: frames,
                }
            })
            .collect()
    };
    // No entry is queued until every explicit session has been validated.
    let mut tx = app.db.begin().await?;
    for (session_index, group) in groups.into_iter().enumerate() {
        let fingerprint = group.fingerprint();
        let eid = id();
        let first = &group.sources[0];
        let duration = if group.is_trimmed() {
            group.duration_ms()
        } else {
            group
                .sources
                .iter()
                .map(|source| source.format.duration_ms())
                .sum()
        };
        let recorded = group
            .recorded_at()
            .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        let title = recorded
            .as_ref()
            .map(|date| format!("Recording · {}", &date[..10]))
            .unwrap_or_else(|| {
                Path::new(&first.filename)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Recording")
                    .to_string()
            });
        let trim_start = group.is_trimmed().then_some(group.start_frame as i64);
        let trim_end = group.is_trimmed().then_some(group.end_frame as i64);
        let inserted = sqlx::query("INSERT OR IGNORE INTO entries(id,fingerprint,title,recorded_at,duration_ms,timezone,stage,created_at,updated_at,trim_start_frame,trim_end_frame)VALUES(?,?,?,?,?,?,'queued',?,?,?,?)")
            .bind(&eid).bind(&fingerprint).bind(title).bind(recorded).bind(duration).bind(&zone).bind(now()).bind(now()).bind(trim_start).bind(trim_end).execute(&mut *tx).await?;
        let entry_id = if inserted.rows_affected() > 0 {
            let mut offset = 0;
            for (position, source) in group.sources.iter().enumerate() {
                sqlx::query("INSERT INTO entry_sources(entry_id,upload_id,position,offset_ms,recorded_at)VALUES(?,?,?,?,?)")
                    .bind(&eid).bind(&source.id).bind(position as i64).bind(offset)
                    .bind(source.recorded_at.map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Millis,true))).execute(&mut *tx).await?;
                offset += source.format.duration_ms();
            }
            eid
        } else {
            for source in &group.sources {
                sqlx::query("UPDATE uploads SET state='duplicate' WHERE id=?")
                    .bind(&source.id)
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query("UPDATE entries SET trashed_at=NULL WHERE fingerprint=?")
                .bind(&fingerprint)
                .execute(&mut *tx)
                .await?;
            sqlx::query_scalar("SELECT id FROM entries WHERE fingerprint=?")
                .bind(&fingerprint)
                .fetch_one(&mut *tx)
                .await?
        };
        sqlx::query(
            "INSERT OR IGNORE INTO import_entries(import_id,entry_id,session_index)VALUES(?,?,?)",
        )
        .bind(batch)
        .bind(entry_id)
        .bind(session_index as i64)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE imports SET state='complete' WHERE id=?")
        .bind(batch)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn stage(app: &App, entry: &str, stage: &str) -> Result<()> {
    sqlx::query("UPDATE entries SET stage=?,active_stage=?,failure=NULL,updated_at=? WHERE id=?")
        .bind(stage)
        .bind(stage)
        .bind(now())
        .bind(entry)
        .execute(&app.db)
        .await?;
    Ok(())
}

/// Decodes the entry's sources once, applies the sample-exact trim, and writes whatever is
/// missing of the browser playback file and the 16 kHz mono FLAC used for transcription and
/// voice probes. Returns the FLAC.
async fn render(app: &App, entry: &str, playback: bool) -> Result<PathBuf> {
    let _cpu = app.cpu.acquire().await?;
    let dir = app.entry_dir(entry);
    tokio::fs::create_dir_all(&dir).await?;
    let rendition = dir.join("transcription.flac");
    let playback = playback && !dir.join("playback.m4a").exists();
    let mut targets: Vec<(&str, &str, Vec<&str>)> = vec![];
    if playback {
        let codec = vec!["-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart"];
        targets.push(("playback.partial.m4a", "playback.m4a", codec));
    }
    if !rendition.exists() {
        let codec = vec![
            "-ac",
            "1",
            "-ar",
            "16000",
            "-sample_fmt",
            "s16",
            "-c:a",
            "flac",
        ];
        targets.push(("transcription.partial.flac", "transcription.flac", codec));
    }
    if targets.is_empty() {
        return Ok(rendition);
    }
    let rows = sqlx::query("SELECT es.upload_id,e.trim_start_frame,e.trim_end_frame FROM entry_sources es JOIN entries e ON e.id=es.entry_id WHERE es.entry_id=? ORDER BY es.position")
        .bind(entry)
        .fetch_all(&app.db)
        .await?;
    ensure!(!rows.is_empty(), "Recording has no audio sources");
    // Server-owned paths only, passed as arguments: no user input reaches FFmpeg's syntax.
    let mut args = vec![];
    let mut filter = String::new();
    for (index, row) in rows.iter().enumerate() {
        let upload: String = row.get("upload_id");
        args.extend(["-i".into(), app.upload_path(&upload).display().to_string()]);
        filter.push_str(&format!("[{index}:a:0]"));
    }
    filter.push_str(&format!("concat=n={}:v=0:a=1", rows.len()));
    let trim: (Option<i64>, Option<i64>) = (
        rows[0].get("trim_start_frame"),
        rows[0].get("trim_end_frame"),
    );
    if let (Some(start), Some(end)) = trim {
        filter.push_str(&format!(
            ",atrim=start_sample={start}:end_sample={end},asetpts=PTS-STARTPTS"
        ));
    }
    if targets.len() > 1 {
        filter.push_str(&format!(",asplit={}", targets.len()));
    }
    for index in 0..targets.len() {
        filter.push_str(&format!("[out{index}]"));
    }
    args.extend(["-filter_complex".into(), filter]);
    for (index, (partial, _, codec)) in targets.iter().enumerate() {
        args.extend(["-map".into(), format!("[out{index}]")]);
        args.extend(codec.iter().map(|value| value.to_string()));
        args.push(dir.join(partial).display().to_string());
    }
    audio::ffmpeg(&args).await?;
    for (partial, done, _) in targets {
        commit_artifact(&dir.join(partial), &dir.join(done)).await?;
    }
    Ok(rendition)
}

async fn pause_at_checkpoint(app: &App, entry: &str) -> Result<bool> {
    let row = sqlx::query("SELECT stage,pause_requested,trashed_at FROM entries WHERE id=?")
        .bind(entry)
        .fetch_one(&app.db)
        .await?;
    let requested = row.get::<bool, _>("pause_requested")
        || row.get::<String, _>("stage") == "paused"
        || row.get::<Option<String>, _>("trashed_at").is_some();
    if requested {
        sqlx::query("UPDATE entries SET stage='paused',pause_requested=0,updated_at=? WHERE id=?")
            .bind(now())
            .bind(entry)
            .execute(&app.db)
            .await?;
    }
    Ok(requested)
}

/// The whole recording as one AssemblyAI job, checkpointed so it is paid for once: the upload
/// URL is saved before the job is requested, its ID the moment it exists, and its result once
/// it completes. Returns None when a pause or shutdown interrupts; the next run continues.
async fn transcribe(
    app: &App,
    entry: &str,
    rendition: &Path,
    stop: &CancellationToken,
) -> Result<Option<Vec<Utterance>>> {
    let saved: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT remote_id,audio_url,utterances FROM transcriptions WHERE entry_id=?",
    )
    .bind(entry)
    .fetch_optional(&app.db)
    .await?;
    let job = match saved {
        Some((_, _, Some(utterances))) => return Ok(Some(serde_json::from_str(&utterances)?)),
        Some((Some(job), _, None)) => job,
        // A request whose answer was lost: its job is found by the upload URL, or it never arrived.
        Some((None, Some(audio), None)) => match provider::find(app, &audio).await? {
            Some(job) => save_job(app, entry, &job).await?,
            None if stop.is_cancelled() => return Ok(None),
            None => submit(app, entry, &audio).await?,
        },
        // Saved before upload URLs were: only the owner can decide to pay again.
        Some((None, None, None)) => bail!(AMBIGUOUS),
        None => {
            let duration: i64 = sqlx::query_scalar("SELECT duration_ms FROM entries WHERE id=?")
                .bind(entry)
                .fetch_one(&app.db)
                .await?;
            ensure!(
                duration <= MAX_DURATION_MS,
                "AssemblyAI transcribes at most 10 hours per recording. Split this session into shorter ones and import it again."
            );
            if stop.is_cancelled() {
                return Ok(None);
            }
            let audio = provider::upload(app, rendition).await?;
            if stop.is_cancelled() {
                return Ok(None);
            }
            sqlx::query("INSERT INTO transcriptions(entry_id,state,audio_url,submitted_at)VALUES(?,'submitted',?,?)")
                .bind(entry).bind(&audio).bind(now()).execute(&app.db).await?;
            submit(app, entry, &audio).await?
        }
    };
    loop {
        let status = match provider::poll(app, &job).await {
            Ok(status) => status,
            Err(error) => {
                let failure = error.downcast_ref::<ProviderError>();
                if failure.is_some_and(ProviderError::not_found) {
                    return Err(fail_transcription(
                        app,
                        entry,
                        "AssemblyAI no longer has this recording's job. Retry to submit it again.",
                    )
                    .await);
                }
                if !failure.is_some_and(ProviderError::transient) {
                    return Err(error);
                }
                // An outage does not stop the job at AssemblyAI: keep waiting for its result.
                tracing::warn!(%entry, %error, "AssemblyAI unavailable; waiting for the job");
                Job::Pending
            }
        };
        match status {
            Job::Completed { utterances, raw } => {
                let dir = app.entry_dir(entry);
                tokio::fs::write(dir.join("transcription.partial.json"), raw).await?;
                commit_artifact(
                    &dir.join("transcription.partial.json"),
                    &dir.join("transcription.json"),
                )
                .await?;
                sqlx::query(
                    "UPDATE transcriptions SET state='complete',utterances=? WHERE entry_id=?",
                )
                .bind(serde_json::to_string(&utterances)?)
                .bind(entry)
                .execute(&app.db)
                .await?;
                // Notes from an earlier transcript name other labels and other text.
                for stale in ["enrichment.json", "identity-analysis.json"] {
                    let _ = tokio::fs::remove_file(dir.join(stale)).await;
                }
                sqlx::query("UPDATE entries SET summary=NULL WHERE id=?")
                    .bind(entry)
                    .execute(&app.db)
                    .await?;
                // The vault keeps the result; AssemblyAI need not keep the audio or transcript.
                if let Err(error) = provider::delete(app, &job).await {
                    tracing::warn!(%entry, %job, %error, "could not delete the job at AssemblyAI");
                }
                return Ok(Some(utterances));
            }
            Job::Failed(message) => {
                return Err(fail_transcription(app, entry, &format!("AssemblyAI could not transcribe this recording ({message}). Retry to submit it again.")).await);
            }
            Job::Pending => {
                if pause_at_checkpoint(app, entry).await? {
                    return Ok(None);
                }
                tokio::select! {
                    _ = tokio::time::sleep(app.config.poll_interval) => {}
                    _ = stop.cancelled() => return Ok(None),
                }
            }
        }
    }
}

/// The paid request. Never repeated here: a lost answer is resolved by `provider::find`.
async fn submit(app: &App, entry: &str, audio: &str) -> Result<String> {
    let context = crate::settings::load(app).await?.recording_context;
    let prompt = Some(context.trim()).filter(|text| !text.is_empty());
    let job = match provider::submit(app, audio, prompt).await {
        Ok(job) => job,
        // A definite refusal created no job: the next attempt uploads and submits afresh.
        Err(error)
            if error
                .downcast_ref::<ProviderError>()
                .is_some_and(ProviderError::rejected) =>
        {
            return Err(fail_transcription(app, entry, &error.to_string()).await);
        }
        Err(error) => match provider::find(app, audio).await {
            Ok(Some(job)) => job,
            // The job may still exist; the saved upload URL lets the next run look again.
            _ => return Err(error),
        },
    };
    save_job(app, entry, &job).await
}

async fn save_job(app: &App, entry: &str, job: &str) -> Result<String> {
    tracing::info!(%entry, %job, "AssemblyAI job created");
    sqlx::query("UPDATE transcriptions SET state='processing',remote_id=? WHERE entry_id=?")
        .bind(job)
        .bind(entry)
        .execute(&app.db)
        .await?;
    Ok(job.into())
}

/// Forgets a job that cannot produce a result and fails the entry in one step, so a restart in
/// between cannot resubmit without the owner's Retry.
async fn fail_transcription(app: &App, entry: &str, message: &str) -> anyhow::Error {
    let result = async {
        let mut tx = app.db.begin().await?;
        sqlx::query("DELETE FROM transcriptions WHERE entry_id=?")
            .bind(entry)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE entries SET stage='failed',failure=?,updated_at=? WHERE id=?")
            .bind(message)
            .bind(now())
            .bind(entry)
            .execute(&mut *tx)
            .await?;
        tx.commit().await
    }
    .await;
    match result {
        Ok(()) => anyhow::anyhow!("{message}"),
        Err(error) => error.into(),
    }
}

/// Replaces the entry's segments. Speaker labels that no longer occur (a legacy multi-part
/// transcript replaced by one job) are removed together with their voice data.
async fn store_segments(app: &App, entry: &str, utterances: &[Utterance]) -> Result<()> {
    let duration: i64 = sqlx::query_scalar("SELECT duration_ms FROM entries WHERE id=?")
        .bind(entry)
        .fetch_one(&app.db)
        .await?;
    let mut tx = app.db.begin().await?;
    sqlx::query("DELETE FROM segments WHERE entry_id=?")
        .bind(entry)
        .execute(&mut *tx)
        .await?;
    // IDs are deterministic across retries.
    for (position, utterance) in utterances.iter().enumerate() {
        sqlx::query("INSERT OR IGNORE INTO speakers(entry_id,label)VALUES(?,?)")
            .bind(entry)
            .bind(&utterance.speaker)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO segments(id,entry_id,position,speaker,start_ms,end_ms,text)VALUES(?,?,?,?,?,?,?)")
            .bind(format!("{entry}:{position}"))
            .bind(entry)
            .bind(position as i64)
            .bind(&utterance.speaker)
            .bind(utterance.start.min(duration))
            .bind(utterance.end.min(duration))
            .bind(utterance.text.trim())
            .execute(&mut *tx)
            .await?;
    }
    for table in ["voice_samples", "voice_probes"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE entry_id=? AND speaker NOT IN (SELECT speaker FROM segments WHERE entry_id=?)"))
            .bind(entry)
            .bind(entry)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DELETE FROM speakers WHERE entry_id=? AND label NOT IN (SELECT speaker FROM segments WHERE entry_id=?)")
        .bind(entry)
        .bind(entry)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn transcript_lines(app: &App, entry: &str) -> Result<Vec<String>> {
    let rows = sqlx::query("SELECT speaker,text FROM segments WHERE entry_id=? ORDER BY position")
        .bind(entry)
        .fetch_all(&app.db)
        .await?;
    Ok(rows
        .iter()
        .map(|row| {
            format!(
                "[{}] {}",
                row.get::<String, _>("speaker"),
                row.get::<String, _>("text")
            )
        })
        .collect())
}

/// Cached provider output: reused on retry so a completed request is not paid for twice.
async fn cached_notes(app: &App, entry: &str, name: &str) -> Result<provider::Notes> {
    let path = app.entry_dir(entry).join(format!("{name}.json"));
    if let Ok(saved) = tokio::fs::read(&path).await
        && let Ok(notes) = serde_json::from_slice(&saved)
    {
        return Ok(notes);
    }
    let notes = provider::enrich(app, &transcript_lines(app, entry).await?).await?;
    let pending = app.entry_dir(entry).join(format!("{name}.partial.json"));
    tokio::fs::write(&pending, serde_json::to_vec_pretty(&notes)?).await?;
    commit_artifact(&pending, &path).await?;
    Ok(notes)
}

pub async fn reanalyze_speakers(app: &App, entry: &str) -> Result<()> {
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    stage(app, entry, "preparing").await?;
    let rendition = render(app, entry, false).await?;
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    stage(app, entry, "recognizing").await?;
    sqlx::query("UPDATE speakers SET person_id=NULL,state='unknown',evidence='[]' WHERE entry_id=? AND state NOT IN ('manual','manual_unknown')").bind(entry).execute(&app.db).await?;
    let previous: String = sqlx::query_scalar("SELECT warnings FROM entries WHERE id=?")
        .bind(entry)
        .fetch_one(&app.db)
        .await?;
    let mut warnings: Vec<String> = serde_json::from_str(&previous)?;
    warnings.retain(|warning| {
        !warning.starts_with("Speaker recognition unavailable:")
            && !warning.starts_with("Speaker name analysis unavailable:")
    });
    // As in processing: both at once, results written afterwards.
    let (voice, notes) = tokio::join!(
        voice::analyze(app, entry, &rendition),
        cached_notes(app, entry, "identity-analysis")
    );
    if let Err(error) = voice {
        warnings.push(format!("Speaker recognition unavailable: {error}"));
    }
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    stage(app, entry, "enriching").await?;
    match notes {
        Ok(notes) => {
            save_notes(app, entry, &notes, true).await?;
            apply_name_evidence(app, entry, &notes.names).await?
        }
        Err(error) => warnings.push(format!("Speaker name analysis unavailable: {error}")),
    }
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    complete(app, entry, &warnings, &rendition).await
}

/// The generated title (unless the owner set one) and summary. Reanalyze `replace`s them with
/// fresh notes; processing keeps a summary that is already there.
async fn save_notes(app: &App, entry: &str, notes: &provider::Notes, replace: bool) -> Result<()> {
    if notes.title.trim().is_empty() {
        return Ok(());
    }
    let mut tx = app.db.begin().await?;
    sqlx::query("UPDATE entries SET title=CASE WHEN title_manual=1 THEN title ELSE ? END,summary=? WHERE id=? AND (? OR summary IS NULL)")
        .bind(notes.title.trim().chars().take(240).collect::<String>()).bind(notes.summary.trim()).bind(entry).bind(replace).execute(&mut *tx).await?;
    sqlx::query(
        "UPDATE passage_fts SET title=(SELECT title FROM entries WHERE id=?) WHERE entry_id=?",
    )
    .bind(entry)
    .bind(entry)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn complete(app: &App, entry: &str, warnings: &[String], rendition: &Path) -> Result<()> {
    sqlx::query("UPDATE entries SET stage='complete',active_stage=NULL,failure=NULL,requested_work=NULL,pause_requested=0,warnings=?,updated_at=? WHERE id=?")
        .bind(serde_json::to_string(warnings)?).bind(now()).bind(entry).execute(&app.db).await?;
    let _ = tokio::fs::remove_file(rendition).await;
    Ok(())
}

pub async fn process(app: &App, entry: &str) -> Result<()> {
    process_until(app, entry, &CancellationToken::new()).await
}

/// `stop` ends a wait for AssemblyAI early (shutdown); the job resumes after restart.
async fn process_until(app: &App, entry: &str, stop: &CancellationToken) -> Result<()> {
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    let ambiguous: Option<String> = sqlx::query_scalar(
        "SELECT state FROM transcriptions WHERE entry_id=? AND state='submitted' AND audio_url IS NULL",
    )
    .bind(entry)
    .fetch_optional(&app.db)
    .await?;
    ensure!(ambiguous.is_none(), AMBIGUOUS);
    stage(app, entry, "preparing").await?;
    let rendition = render(app, entry, true).await?;
    // Playback is ready: listen while the rest is processed.
    sqlx::query("UPDATE entries SET audio_path='playback.m4a',updated_at=? WHERE id=?")
        .bind(now())
        .bind(entry)
        .execute(&app.db)
        .await?;
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    stage(app, entry, "transcribing").await?;
    let Some(utterances) = transcribe(app, entry, &rendition, stop).await? else {
        return Ok(());
    };
    store_segments(app, entry, &utterances).await?;
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    let mut warnings: Vec<String> = vec![];
    if !utterances.is_empty() {
        stage(app, entry, "recognizing").await?;
        // Voice analysis is local and the notes come from Groq: both run at once, and their
        // results are written afterwards, so neither overwrites the other's speaker evidence.
        let (voice, notes) = tokio::join!(
            voice::analyze(app, entry, &rendition),
            cached_notes(app, entry, "enrichment")
        );
        if let Err(e) = voice {
            warnings.push(format!("Speaker recognition unavailable: {e}"));
        }
        if pause_at_checkpoint(app, entry).await? {
            return Ok(());
        }
        stage(app, entry, "enriching").await?;
        match notes {
            Ok(notes) => {
                save_notes(app, entry, &notes, false).await?;
                apply_name_evidence(app, entry, &notes.names).await?;
            }
            Err(e) => warnings.push(format!("Summary unavailable: {e}")),
        }
        if pause_at_checkpoint(app, entry).await? {
            return Ok(());
        }
    }
    stage(app, entry, "indexing").await?;
    if let Err(e) = index(app, entry).await {
        warnings.push(format!("Semantic search unavailable: {e}"));
    }
    complete(app, entry, &warnings, &rendition).await
}

/// A model finding is evidence for review, never an automatic voice enrollment. It counts only
/// when the quote was spoken by that speaker and contains the name, and only when no other
/// finding names that speaker differently or gives the name to another speaker.
async fn apply_name_evidence(app: &App, entry: &str, names: &[NameFinding]) -> Result<()> {
    let mut valid = vec![];
    for finding in names {
        let name = finding.name.trim();
        // The model sometimes repeats the transcript's brackets: "[A]" for speaker A.
        let speaker = finding.speaker.trim().trim_matches(['[', ']']);
        if name.is_empty() || name.len() > 120 || finding.quote.is_empty() {
            continue;
        }
        let spoken: Vec<String> =
            sqlx::query_scalar("SELECT text FROM segments WHERE entry_id=? AND speaker=?")
                .bind(entry)
                .bind(speaker)
                .fetch_all(&app.db)
                .await?;
        if spoken.iter().any(|text| text.contains(&finding.quote))
            && finding.quote.to_lowercase().contains(&name.to_lowercase())
        {
            valid.push((speaker, name.to_lowercase(), finding));
        }
    }
    let mut applied = HashSet::new();
    for (speaker, name, finding) in &valid {
        let contradicted = valid
            .iter()
            .any(|(other, other_name, _)| (other == speaker) != (other_name == name));
        if contradicted || !applied.insert(*speaker) {
            continue;
        }
        // Added beside a voice match, never over it; the owner's own decisions stay untouched.
        let current: Option<String> = sqlx::query_scalar("SELECT evidence FROM speakers WHERE entry_id=? AND label=? AND state NOT IN ('manual','manual_unknown')")
            .bind(entry).bind(speaker).fetch_optional(&app.db).await?;
        let Some(current) = current else { continue };
        let mut evidence: Vec<serde_json::Value> =
            serde_json::from_str(&current).unwrap_or_default();
        evidence.retain(|item| item["source"] != "transcript");
        evidence.push(json!({"source":"transcript","name":finding.name.trim(),"quote":finding.quote,"applied":false}));
        sqlx::query("UPDATE speakers SET evidence=? WHERE entry_id=? AND label=?")
            .bind(serde_json::to_string(&evidence)?)
            .bind(entry)
            .bind(speaker)
            .execute(&app.db)
            .await?;
    }
    Ok(())
}

pub async fn index(app: &App, entry: &str) -> Result<()> {
    let rows = sqlx::query(
        "SELECT id,start_ms,end_ms,text FROM segments WHERE entry_id=? ORDER BY position",
    )
    .bind(entry)
    .fetch_all(&app.db)
    .await?;
    let title: String = sqlx::query_scalar("SELECT title FROM entries WHERE id=?")
        .bind(entry)
        .fetch_one(&app.db)
        .await?;
    let mut chunks: Vec<(i64, i64, String)> = vec![];
    let mut start = 0usize;
    while start < rows.len() {
        let mut end = start;
        let mut chars = 0;
        while end < rows.len() && (chars < 1800 || end == start) {
            chars += rows[end].get::<String, _>("text").len();
            end += 1;
        }
        chunks.push((
            rows[start].get("start_ms"),
            rows[end - 1].get("end_ms"),
            rows[start..end]
                .iter()
                .map(|r| r.get::<String, _>("text"))
                .collect::<Vec<_>>()
                .join(" "),
        ));
        if end == rows.len() {
            break;
        }
        start = if end - start > 2 { end - 2 } else { end };
    }
    let mut vectors = HashMap::new();
    let mut embedding_error = None;
    if provider::semantic_search(app) {
        let texts: Vec<String> = chunks.iter().map(|(_, _, t)| t.clone()).collect();
        let batches = join_all(
            texts
                .chunks(32)
                .map(|values| provider::embeddings(app, values, true)),
        )
        .await;
        for (batch, result) in batches.into_iter().enumerate() {
            match result {
                Ok(v) => {
                    for (i, x) in v.into_iter().enumerate() {
                        vectors.insert(batch * 32 + i, x);
                    }
                }
                Err(e) => {
                    embedding_error = Some(e);
                    break;
                }
            }
        }
    }
    let mut tx = app.db.begin().await?;
    sqlx::query("DELETE FROM passage_fts WHERE entry_id=?")
        .bind(entry)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM passages WHERE entry_id=?")
        .bind(entry)
        .execute(&mut *tx)
        .await?;
    for (i, (start, end, text)) in chunks.into_iter().enumerate() {
        let pid = format!("{entry}:p{i}");
        sqlx::query("INSERT INTO passages(id,entry_id,position,start_ms,end_ms,text,embedding)VALUES(?,?,?,?,?,?,?)").bind(&pid).bind(entry).bind(i as i64).bind(start).bind(end).bind(&text).bind(vectors.get(&i).map(|v|serde_json::to_string(v).unwrap())).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO passage_fts(id,entry_id,title,text)VALUES(?,?,?,?)")
            .bind(pid)
            .bind(entry)
            .bind(&title)
            .bind(text)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    if let Some(e) = embedding_error {
        return Err(e);
    }
    Ok(())
}

async fn commit_artifact(pending: &Path, final_path: &Path) -> Result<()> {
    tokio::fs::OpenOptions::new()
        .write(true)
        .open(pending)
        .await?
        .sync_all()
        .await?;
    tokio::fs::rename(pending, final_path).await?;
    Ok(())
}
