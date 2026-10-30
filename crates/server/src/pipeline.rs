use crate::{
    audio,
    config::{App, id, now},
    grouping::{self, Source},
    identity,
    provider::{self, Job, ProviderError, Utterance},
    voice::{self, Analysis},
};
use anyhow::{Result, bail, ensure};
use futures_util::future::join_all;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

const AMBIGUOUS: &str = "A transcription request may have been submitted. Retry explicitly to authorize another request.";
/// Entries processed at the same time. Most of a job is waiting for AssemblyAI or OpenAI; local
/// CPU work has its own limit on `App`.
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
        let import: Option<String> = sqlx::query_scalar!(
            "SELECT id AS \"id!\" FROM imports WHERE state='verifying' ORDER BY created_at LIMIT 1"
        )
        .fetch_optional(&app.db)
        .await
        .unwrap_or(None);
        if let Some(batch) = import {
            if let Err(error) = finalize(&app, &batch).await {
                tracing::error!(import=%batch,error=%error,"import verification failed");
                let _ = sqlx::query!("UPDATE imports SET state='failed' WHERE id=?", batch)
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
    let _ = sqlx::query!("UPDATE entries SET stage='failed',failure='Server stopped while submitting the transcription. Retry explicitly to resubmit.' WHERE stage NOT IN ('queued','complete','failed','paused') AND trashed_at IS NULL AND EXISTS(SELECT 1 FROM transcriptions t WHERE t.entry_id=entries.id AND t.state='submitted' AND t.audio_url IS NULL)")
        .execute(&app.db)
        .await;
    let _ = sqlx::query!("UPDATE entries SET stage='queued' WHERE stage NOT IN ('queued','complete','failed','paused') AND trashed_at IS NULL")
        .execute(&app.db)
        .await;
}

/// Atomically takes the oldest queued entry, so concurrent jobs never share one.
async fn claim(app: &App) -> Option<(String, Option<String>)> {
    let timestamp = now();
    sqlx::query!("UPDATE entries SET stage='preparing',active_stage='preparing',failure=NULL,updated_at=? WHERE id=(SELECT id FROM entries WHERE stage='queued' AND trashed_at IS NULL ORDER BY created_at LIMIT 1) AND stage='queued' RETURNING id AS \"id!\",requested_work", timestamp)
        .fetch_optional(&app.db)
        .await
        .unwrap_or_else(|error| {
            tracing::error!(%error, "could not claim queued work");
            None
        })
        .map(|row| (row.id, row.requested_work))
}

async fn run(app: &App, entry: &str, work: Option<&str>, stop: &CancellationToken) {
    let result = match work {
        Some("speakers") => reanalyze_speakers(app, entry).await,
        Some("summary") => regenerate_summary(app, entry).await,
        _ => process_until(app, entry, stop).await,
    };
    if let Err(error) = result {
        tracing::error!(entry=%entry,error=%error,"entry processing failed");
        let failure = error.to_string().chars().take(1000).collect::<String>();
        let timestamp = now();
        let _ = sqlx::query!(
            "UPDATE entries SET stage='failed',failure=?,updated_at=? WHERE id=?",
            failure,
            timestamp,
            entry
        )
        .execute(&app.db)
        .await;
    }
}
pub async fn finalize(app: &App, batch: &str) -> Result<()> {
    let import = sqlx::query!(
        "SELECT recorder_timezone,session_plan FROM imports WHERE id=?",
        batch
    )
    .fetch_one(&app.db)
    .await?;
    let zone: String = import.recorder_timezone.clone();
    let tz = zone.parse()?;
    let plans: Option<Vec<grouping::SessionPlan>> = import
        .session_plan
        .clone()
        .map(|plan| serde_json::from_str(&plan))
        .transpose()?;
    let rows = sqlx::query!("SELECT id AS \"id!\", original_name, claimed_pcm_hash, source_id FROM uploads WHERE import_id=? ORDER BY relative_path", batch)
        .fetch_all(&app.db)
        .await?;
    // Decoding is the slow part: verify the uploads in parallel, then record them in order.
    let verified = join_all(rows.iter().map(|row| async move {
        let _cpu = app.cpu.acquire().await?;
        audio::verify(&app.upload_path(&row.id.clone())).await
    }))
    .await;
    let mut sources = vec![];
    let mut seen = HashSet::new();
    for (row, verified) in rows.into_iter().zip(verified) {
        let uid: String = row.id.clone();
        let original: String = row.original_name.clone();
        let (format, hash) = match verified {
            Ok(verified) => verified,
            Err(error) => {
                let failure = error.to_string();
                sqlx::query!(
                    "UPDATE uploads SET state='invalid',failure=? WHERE id=?",
                    failure,
                    uid
                )
                .execute(&app.db)
                .await?;
                continue;
            }
        };
        let claimed: Option<String> = row.claimed_pcm_hash.clone();
        if claimed.as_ref().is_some_and(|claimed| claimed != &hash) {
            sqlx::query!(
                "UPDATE uploads SET state='invalid',failure='PCM verification mismatch' WHERE id=?",
                uid
            )
            .execute(&app.db)
            .await?;
            continue;
        }
        let format_json = serde_json::to_string(&format)?;
        sqlx::query!(
            "UPDATE uploads SET state='verified',pcm_hash=?,format_json=?,failure=NULL WHERE id=?",
            hash,
            format_json,
            uid
        )
        .execute(&app.db)
        .await?;
        if plans.is_none() {
            let known: i64 = sqlx::query_scalar!("SELECT COUNT(*) FROM entry_sources es JOIN uploads u ON u.id=es.upload_id JOIN entries e ON e.id=es.entry_id WHERE u.pcm_hash=? AND e.trashed_at IS NULL", hash).fetch_one(&app.db).await?;
            if known > 0 || !seen.insert(hash.clone()) {
                sqlx::query!("UPDATE uploads SET state='duplicate' WHERE id=?", uid)
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
        let source_id = row.source_id.clone().unwrap_or_else(|| uid.clone());
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
        let created_at = now();
        let updated_at = now();
        let inserted = sqlx::query!("INSERT OR IGNORE INTO entries(id,fingerprint,title,recorded_at,duration_ms,timezone,stage,created_at,updated_at,trim_start_frame,trim_end_frame)VALUES(?,?,?,?,?,?,'queued',?,?,?,?)", eid, fingerprint, title, recorded, duration, zone, created_at, updated_at, trim_start, trim_end).execute(&mut *tx).await?;
        let entry_id = if inserted.rows_affected() > 0 {
            let mut offset = 0;
            for (position, source) in group.sources.iter().enumerate() {
                let query_position = position as i64;
                let recorded_at = source
                    .recorded_at
                    .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
                sqlx::query!("INSERT INTO entry_sources(entry_id,upload_id,position,offset_ms,recorded_at)VALUES(?,?,?,?,?)", eid, source.id, query_position, offset, recorded_at).execute(&mut *tx).await?;
                offset += source.format.duration_ms();
            }
            eid
        } else {
            for source in &group.sources {
                sqlx::query!("UPDATE uploads SET state='duplicate' WHERE id=?", source.id)
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query!(
                "UPDATE entries SET trashed_at=NULL WHERE fingerprint=?",
                fingerprint
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query_scalar!(
                "SELECT id AS \"id!\" FROM entries WHERE fingerprint=?",
                fingerprint
            )
            .fetch_one(&mut *tx)
            .await?
        };
        let query_session_index = session_index as i64;
        sqlx::query!(
            "INSERT OR IGNORE INTO import_entries(import_id,entry_id,session_index)VALUES(?,?,?)",
            batch,
            entry_id,
            query_session_index
        )
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query!("UPDATE imports SET state='complete' WHERE id=?", batch)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn stage(app: &App, entry: &str, stage: &str) -> Result<()> {
    let timestamp = now();
    sqlx::query!(
        "UPDATE entries SET stage=?,active_stage=?,failure=NULL,updated_at=? WHERE id=?",
        stage,
        stage,
        timestamp,
        entry
    )
    .execute(&app.db)
    .await?;
    Ok(())
}

/// Decodes the entry's sources once, applies the sample-exact trim, and writes whatever is
/// missing of the playback file (also the source of voice probes) and, for an `upload`, the
/// 16 kHz mono FLAC sent to AssemblyAI. Returns the FLAC's path.
async fn render(app: &App, entry: &str, upload: bool) -> Result<PathBuf> {
    let _cpu = app.cpu.acquire().await?;
    let dir = app.entry_dir(entry);
    tokio::fs::create_dir_all(&dir).await?;
    let rendition = dir.join("transcription.flac");
    let mut targets: Vec<(&str, &str, Vec<&str>)> = vec![];
    if !dir.join("playback.m4a").exists() {
        let codec = vec!["-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart"];
        targets.push(("playback.partial.m4a", "playback.m4a", codec));
    }
    if upload && !rendition.exists() {
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
    let rows = sqlx::query!("SELECT es.upload_id,e.trim_start_frame,e.trim_end_frame FROM entry_sources es JOIN entries e ON e.id=es.entry_id WHERE es.entry_id=? ORDER BY es.position", entry)
        .fetch_all(&app.db)
        .await?;
    ensure!(!rows.is_empty(), "Recording has no audio sources");
    // Server-owned paths only, passed as arguments: no user input reaches FFmpeg's syntax.
    let mut args = vec![];
    let mut filter = String::new();
    for (index, row) in rows.iter().enumerate() {
        let upload: String = row.upload_id.clone();
        args.extend(["-i".into(), app.upload_path(&upload).display().to_string()]);
        filter.push_str(&format!("[{index}:a:0]"));
    }
    filter.push_str(&format!("concat=n={}:v=0:a=1", rows.len()));
    let trim: (Option<i64>, Option<i64>) = (rows[0].trim_start_frame, rows[0].trim_end_frame);
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
    let row = sqlx::query!(
        "SELECT stage,pause_requested,trashed_at FROM entries WHERE id=?",
        entry
    )
    .fetch_one(&app.db)
    .await?;
    let requested = (row.pause_requested != 0)
        || row.stage.clone() == "paused"
        || row.trashed_at.clone().is_some();
    if requested {
        let timestamp = now();
        sqlx::query!(
            "UPDATE entries SET stage='paused',pause_requested=0,updated_at=? WHERE id=?",
            timestamp,
            entry
        )
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
    let saved = sqlx::query!(
        "SELECT remote_id,audio_url,utterances FROM transcriptions WHERE entry_id=?",
        entry
    )
    .fetch_optional(&app.db)
    .await?
    .map(|row| (row.remote_id, row.audio_url, row.utterances));
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
            let duration: i64 =
                sqlx::query_scalar!("SELECT duration_ms FROM entries WHERE id=?", entry)
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
            let timestamp = now();
            sqlx::query!("INSERT INTO transcriptions(entry_id,state,audio_url,submitted_at)VALUES(?,'submitted',?,?)", entry, audio, timestamp).execute(&app.db).await?;
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
                // Notes from an earlier transcript name other labels and other text.
                for stale in ["enrichment.json", "identity-analysis.json", "summary.json"] {
                    let _ = tokio::fs::remove_file(dir.join(stale)).await;
                }
                let mut tx = app.db.begin().await?;
                let utterances_json = serde_json::to_string(&utterances)?;
                sqlx::query!(
                    "UPDATE transcriptions SET state='complete',utterances=? WHERE entry_id=?",
                    utterances_json,
                    entry
                )
                .execute(&mut *tx)
                .await?;
                // A new job's "A" can be someone else: names and voices of the earlier labels go.
                for query in [
                    sqlx::query!("DELETE FROM speakers WHERE entry_id=?", entry),
                    sqlx::query!("DELETE FROM voice_samples WHERE entry_id=?", entry),
                    sqlx::query!("DELETE FROM voice_probes WHERE entry_id=?", entry),
                ] {
                    query.execute(&mut *tx).await?;
                }
                sqlx::query!("UPDATE entries SET summary=NULL WHERE id=?", entry)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
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
    sqlx::query!(
        "UPDATE transcriptions SET state='processing',remote_id=? WHERE entry_id=?",
        job,
        entry
    )
    .execute(&app.db)
    .await?;
    Ok(job.into())
}

/// Forgets a job that cannot produce a result and fails the entry in one step, so a restart in
/// between cannot resubmit without the owner's Retry.
async fn fail_transcription(app: &App, entry: &str, message: &str) -> anyhow::Error {
    let result = async {
        let mut tx = app.db.begin().await?;
        sqlx::query!("DELETE FROM transcriptions WHERE entry_id=?", entry)
            .execute(&mut *tx)
            .await?;
        let timestamp = now();
        sqlx::query!(
            "UPDATE entries SET stage='failed',failure=?,updated_at=? WHERE id=?",
            message,
            timestamp,
            entry
        )
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
    let duration: i64 = sqlx::query_scalar!("SELECT duration_ms FROM entries WHERE id=?", entry)
        .fetch_one(&app.db)
        .await?;
    let mut tx = app.db.begin().await?;
    sqlx::query!("DELETE FROM segments WHERE entry_id=?", entry)
        .execute(&mut *tx)
        .await?;
    // IDs are deterministic across retries.
    for (position, utterance) in utterances.iter().enumerate() {
        sqlx::query!(
            "INSERT OR IGNORE INTO speakers(entry_id,label)VALUES(?,?)",
            entry,
            utterance.speaker
        )
        .execute(&mut *tx)
        .await?;
        let segment_id = format!("{entry}:{position}");
        let query_position = position as i64;
        let start_ms = utterance.start.min(duration);
        let end_ms = utterance.end.min(duration);
        let text = utterance.text.trim();
        sqlx::query!("INSERT INTO segments(id,entry_id,position,speaker,start_ms,end_ms,text,confidence)VALUES(?,?,?,?,?,?,?,?)", segment_id, entry, query_position, utterance.speaker, start_ms, end_ms, text, utterance.confidence)
            .execute(&mut *tx)
            .await?;
    }
    for query in [
        sqlx::query!(
            "DELETE FROM voice_samples WHERE entry_id=? AND speaker NOT IN (SELECT speaker FROM segments WHERE entry_id=?)",
            entry,
            entry
        ),
        sqlx::query!(
            "DELETE FROM voice_probes WHERE entry_id=? AND speaker NOT IN (SELECT speaker FROM segments WHERE entry_id=?)",
            entry,
            entry
        ),
    ] {
        query.execute(&mut *tx).await?;
    }
    sqlx::query!("DELETE FROM speakers WHERE entry_id=? AND label NOT IN (SELECT speaker FROM segments WHERE entry_id=?)", entry, entry)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn transcript_lines(app: &App, entry: &str) -> Result<Vec<String>> {
    let rows = sqlx::query!(
        "SELECT speaker,text FROM segments WHERE entry_id=? ORDER BY position",
        entry
    )
    .fetch_all(&app.db)
    .await?;
    Ok(rows
        .iter()
        .map(|row| format!("[{}] {}", row.speaker.clone(), row.text.clone()))
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
    let playback = app.entry_dir(entry).join("playback.m4a");
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    stage(app, entry, "recognizing").await?;
    // Labels merged by an earlier analysis are split again and decided afresh.
    let saved: Option<String> = sqlx::query_scalar!("SELECT utterances AS \"utterances!\" FROM transcriptions WHERE entry_id=? AND utterances IS NOT NULL", entry)
    .fetch_optional(&app.db)
    .await?;
    if let Some(saved) = saved {
        // The job's full response also has each utterance's confidence, which older rows lack.
        let utterances =
            match tokio::fs::read(app.entry_dir(entry).join("transcription.json")).await {
                Ok(raw) => provider::saved_utterances(&raw)?,
                Err(_) => serde_json::from_str(&saved)?,
            };
        store_segments(app, entry, &utterances).await?;
    }
    sqlx::query!("UPDATE speakers SET person_id=NULL,state='unknown',evidence='[]' WHERE entry_id=? AND state NOT IN ('manual','manual_unknown')", entry).execute(&app.db).await?;
    let previous: String = sqlx::query_scalar!("SELECT warnings FROM entries WHERE id=?", entry)
        .fetch_one(&app.db)
        .await?;
    let mut warnings: Vec<String> = serde_json::from_str(&previous)?;
    warnings.retain(|warning| {
        !warning.starts_with("Speaker recognition unavailable:")
            && !warning.starts_with("Speaker name analysis unavailable:")
    });
    // As in processing: both at once, results written afterwards.
    let (voice, notes) = tokio::join!(
        voice::analyze(app, entry, &playback),
        cached_notes(app, entry, "identity-analysis")
    );
    let analysis = voice.unwrap_or_else(|error| {
        warnings.push(format!("Speaker recognition unavailable: {error}"));
        Analysis::default()
    });
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    stage(app, entry, "enriching").await?;
    let names = match notes {
        Ok(mut notes) => {
            relabel(&mut notes, &analysis.merged);
            save_notes(app, entry, &notes, true).await?;
            notes.names
        }
        Err(error) => {
            warnings.push(format!("Speaker name analysis unavailable: {error}"));
            vec![]
        }
    };
    identity::apply(app, entry, &analysis, &names).await?;
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    complete(app, entry, &warnings, Some(&rendition)).await
}

/// A fresh title (unless set by hand) and summary from the current transcript. Speakers are
/// kept: the transcript already carries merged labels, so the placeholders need no relabeling.
pub async fn regenerate_summary(app: &App, entry: &str) -> Result<()> {
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    stage(app, entry, "enriching").await?;
    let previous: String = sqlx::query_scalar!("SELECT warnings FROM entries WHERE id=?", entry)
        .fetch_one(&app.db)
        .await?;
    let mut warnings: Vec<String> = serde_json::from_str(&previous)?;
    warnings.retain(|warning| !warning.starts_with("Summary unavailable:"));
    match cached_notes(app, entry, "summary").await {
        Ok(notes) => save_notes(app, entry, &notes, true).await?,
        Err(error) => warnings.push(format!("Summary unavailable: {error}")),
    }
    complete(app, entry, &warnings, None).await
}

/// The generated title (unless the owner set one) and summary. Reanalyze `replace`s them with
/// fresh notes; processing keeps a summary that is already there.
async fn save_notes(app: &App, entry: &str, notes: &provider::Notes, replace: bool) -> Result<()> {
    if notes.title.trim().is_empty() {
        return Ok(());
    }
    let mut tx = app.db.begin().await?;
    let title = notes.title.trim().chars().take(240).collect::<String>();
    let summary = notes.summary.trim();
    sqlx::query!("UPDATE entries SET title=CASE WHEN title_manual=1 THEN title ELSE ? END,summary=? WHERE id=? AND (? OR summary IS NULL)", title, summary, entry, replace).execute(&mut *tx).await?;
    sqlx::query!(
        "UPDATE passage_fts SET title=(SELECT title FROM entries WHERE id=?) WHERE entry_id=?",
        entry,
        entry
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn complete(
    app: &App,
    entry: &str,
    warnings: &[String],
    rendition: Option<&Path>,
) -> Result<()> {
    let warnings_json = serde_json::to_string(warnings)?;
    let timestamp = now();
    sqlx::query!("UPDATE entries SET stage='complete',active_stage=NULL,failure=NULL,requested_work=NULL,pause_requested=0,warnings=?,updated_at=? WHERE id=?", warnings_json, timestamp, entry).execute(&app.db).await?;
    if let Some(rendition) = rendition {
        let _ = tokio::fs::remove_file(rendition).await;
    }
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
    let ambiguous: Option<String> = sqlx::query_scalar!("SELECT state FROM transcriptions WHERE entry_id=? AND state='submitted' AND audio_url IS NULL", entry)
    .fetch_optional(&app.db)
    .await?;
    ensure!(ambiguous.is_none(), AMBIGUOUS);
    stage(app, entry, "preparing").await?;
    // Once uploaded, AssemblyAI has the audio: a resumed or retried entry needs no new FLAC.
    let uploaded: bool = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM transcriptions WHERE entry_id=?) AS \"exists!: bool\"",
        entry
    )
    .fetch_one(&app.db)
    .await?;
    let rendition = render(app, entry, !uploaded).await?;
    let playback = app.entry_dir(entry).join("playback.m4a");
    // Playback is ready: listen while the rest is processed.
    let timestamp = now();
    sqlx::query!(
        "UPDATE entries SET audio_path='playback.m4a',updated_at=? WHERE id=?",
        timestamp,
        entry
    )
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
        // Voice analysis is local and the notes come from OpenAI: both run at once, and their
        // results are written afterwards, so neither overwrites the other's speaker evidence.
        let (voice, notes) = tokio::join!(
            voice::analyze(app, entry, &playback),
            cached_notes(app, entry, "enrichment")
        );
        let analysis = voice.unwrap_or_else(|e| {
            warnings.push(format!("Speaker recognition unavailable: {e}"));
            Analysis::default()
        });
        if pause_at_checkpoint(app, entry).await? {
            return Ok(());
        }
        stage(app, entry, "enriching").await?;
        let names = match notes {
            Ok(mut notes) => {
                relabel(&mut notes, &analysis.merged);
                save_notes(app, entry, &notes, false).await?;
                notes.names
            }
            Err(e) => {
                warnings.push(format!("Summary unavailable: {e}"));
                vec![]
            }
        };
        identity::apply(app, entry, &analysis, &names).await?;
        if pause_at_checkpoint(app, entry).await? {
            return Ok(());
        }
    }
    stage(app, entry, "indexing").await?;
    if let Err(e) = index(app, entry).await {
        warnings.push(format!("Semantic search unavailable: {e}"));
    }
    complete(app, entry, &warnings, Some(&rendition)).await
}

/// Notes are written while voice analysis may merge labels: point placeholders and name findings
/// of an absorbed label at the label it joined, in merge order so chains resolve.
fn relabel(notes: &mut provider::Notes, merged: &[(String, String)]) {
    for (from, into) in merged {
        notes.summary = notes
            .summary
            .replace(&format!("{{{from}}}"), &format!("{{{into}}}"));
        for finding in &mut notes.names {
            if finding.speaker.trim().trim_matches(['[', ']']) == from {
                finding.speaker = into.clone();
            }
        }
    }
}

pub async fn index(app: &App, entry: &str) -> Result<()> {
    let rows = sqlx::query!("SELECT id AS \"id!\",start_ms,end_ms,text FROM segments WHERE entry_id=? ORDER BY position", entry)
    .fetch_all(&app.db)
    .await?;
    let title: String = sqlx::query_scalar!("SELECT title FROM entries WHERE id=?", entry)
        .fetch_one(&app.db)
        .await?;
    let mut chunks: Vec<(i64, i64, String)> = vec![];
    let mut start = 0usize;
    while start < rows.len() {
        let mut end = start;
        let mut chars = 0;
        while end < rows.len() && (chars < 1800 || end == start) {
            chars += rows[end].text.clone().len();
            end += 1;
        }
        chunks.push((
            rows[start].start_ms,
            rows[end - 1].end_ms,
            rows[start..end]
                .iter()
                .map(|r| r.text.clone())
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
    sqlx::query!("DELETE FROM passage_fts WHERE entry_id=?", entry)
        .execute(&mut *tx)
        .await?;
    sqlx::query!("DELETE FROM passages WHERE entry_id=?", entry)
        .execute(&mut *tx)
        .await?;
    for (i, (start, end, text)) in chunks.into_iter().enumerate() {
        let pid = format!("{entry}:p{i}");
        let query_position = i as i64;
        let embedding_json = vectors.get(&i).map(|v| serde_json::to_string(v).unwrap());
        sqlx::query!("INSERT INTO passages(id,entry_id,position,start_ms,end_ms,text,embedding)VALUES(?,?,?,?,?,?,?)", pid, entry, query_position, start, end, text, embedding_json).execute(&mut *tx).await?;
        sqlx::query!(
            "INSERT INTO passage_fts(id,entry_id,title,text)VALUES(?,?,?,?)",
            pid,
            entry,
            title,
            text
        )
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
