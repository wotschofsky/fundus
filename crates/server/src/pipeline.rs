use crate::{
    audio,
    config::{App, id, now},
    grouping::{self, Source},
    provider::{self, Turn},
    voice,
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sqlx::Row;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

pub async fn worker(app: App, shutdown: tokio_util::sync::CancellationToken) {
    // A remote synchronous request in flight at shutdown is ambiguous, not automatically billable again.
    let _=sqlx::query("UPDATE entries SET stage='failed',failure='Server stopped during processing. Retry explicitly to resume.' WHERE stage NOT IN ('queued','complete','failed','paused') AND trashed_at IS NULL").execute(&app.db).await;
    loop {
        if shutdown.is_cancelled() {
            break;
        }
        let imports: Option<String> = sqlx::query_scalar(
            "SELECT id FROM imports WHERE state='verifying' ORDER BY created_at LIMIT 1",
        )
        .fetch_optional(&app.db)
        .await
        .unwrap_or(None);
        if let Some(batch) = imports {
            if shutdown.is_cancelled() {
                break;
            }
            if let Err(error) = finalize(&app, &batch).await {
                tracing::error!(import=%batch,error=%error,"import verification failed");
                let _ = sqlx::query("UPDATE imports SET state='failed' WHERE id=?")
                    .bind(batch)
                    .execute(&app.db)
                    .await;
            }
            continue;
        }
        let entry: Option<String> = sqlx::query_scalar(
            "SELECT id FROM entries WHERE stage='queued' AND trashed_at IS NULL ORDER BY created_at LIMIT 1",
        )
        .fetch_optional(&app.db)
        .await
        .unwrap_or(None);
        if let Some(entry) = entry {
            if shutdown.is_cancelled() {
                break;
            }
            // Drain the current job before exiting. Dropping this future on SIGTERM
            // loses an in-flight provider response and leaves its checkpoint ambiguous.
            // The shutdown token stops the next job; the process supervisor supplies
            // the outer hard-stop deadline for requests that never return.
            let work: Option<String> =
                sqlx::query_scalar("SELECT requested_work FROM entries WHERE id=?")
                    .bind(&entry)
                    .fetch_one(&app.db)
                    .await
                    .unwrap_or(None);
            let result = if work.as_deref() == Some("speakers") {
                reanalyze_speakers(&app, &entry).await
            } else {
                process(&app, &entry).await
            };
            if let Err(error) = result {
                tracing::error!(entry=%entry,error=%error,"entry processing failed");
                let _ = sqlx::query(
                    "UPDATE entries SET stage='failed',failure=?,updated_at=? WHERE id=?",
                )
                .bind(error.to_string().chars().take(1000).collect::<String>())
                .bind(now())
                .bind(&entry)
                .execute(&app.db)
                .await;
            }
            continue;
        }
        tokio::select! {_=app.wake.notified()=>{},_=tokio::time::sleep(std::time::Duration::from_secs(3))=>{},_=shutdown.cancelled()=>break}
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
    let mut sources = vec![];
    let mut seen = HashSet::new();
    for row in rows {
        let uid: String = row.get("id");
        let original: String = row.get("original_name");
        let (format, hash) = match audio::verify(&app.upload_path(&uid)).await {
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
async fn prepare_working_audio(app: &App, entry: &str) -> Result<(std::path::PathBuf, usize)> {
    let dir = app.entry_dir(entry);
    tokio::fs::create_dir_all(&dir).await?;
    let rows=sqlx::query("SELECT u.id,u.format_json FROM uploads u JOIN entry_sources es ON es.upload_id=u.id WHERE es.entry_id=? ORDER BY es.position").bind(entry).fetch_all(&app.db).await?;
    ensure!(!rows.is_empty(), "Recording has no audio sources");
    // Server-owned filenames only. Rewriting this concat list cannot introduce user paths or shell syntax.
    let mut concat = String::new();
    for (i, row) in rows.iter().enumerate() {
        let uid: String = row.get("id");
        let local = dir.join(format!("source-{i}"));
        if !local.exists() {
            let pending = dir.join(format!("source-{i}.partial"));
            tokio::fs::copy(app.upload_path(&uid), &pending).await?;
            commit_artifact(&pending, &local).await?;
        }
        concat.push_str(&format!("file 'source-{i}'\n"));
    }
    tokio::fs::write(dir.join("sources.txt"), concat).await?;
    let merged = dir.join("working.wav");
    if !merged.exists() {
        let trim = sqlx::query("SELECT trim_start_frame,trim_end_frame FROM entries WHERE id=?")
            .bind(entry)
            .fetch_one(&app.db)
            .await?;
        let trim_start: Option<i64> = trim.get("trim_start_frame");
        let trim_end: Option<i64> = trim.get("trim_end_frame");
        let pending = dir.join("working.partial.wav");
        let mut args = vec![
            "-f".into(),
            "concat".into(),
            "-safe".into(),
            "1".into(),
            "-i".into(),
            dir.join("sources.txt").display().to_string(),
        ];
        if let (Some(start), Some(end)) = (trim_start, trim_end) {
            args.extend([
                "-af".into(),
                format!("atrim=start_sample={start}:end_sample={end},asetpts=PTS-STARTPTS"),
            ]);
        }
        args.extend([
            "-c:a".into(),
            "pcm_f32le".into(),
            "-rf64".into(),
            "auto".into(),
            pending.display().to_string(),
        ]);
        audio::ffmpeg(&args).await?;
        commit_artifact(&pending, &merged).await?;
    }
    Ok((merged, rows.len()))
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

pub async fn reanalyze_speakers(app: &App, entry: &str) -> Result<()> {
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    stage(app, entry, "preparing").await?;
    let (merged, source_count) = prepare_working_audio(app, entry).await?;
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
    warnings.retain(|warning| !warning.starts_with("Speaker recognition unavailable:"));
    if let Err(error) = voice::analyze(app, entry, &merged).await {
        warnings.push(format!("Speaker recognition unavailable: {error}"));
    }
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    stage(app, entry, "enriching").await?;
    let rows = sqlx::query("SELECT speaker,text FROM segments WHERE entry_id=? ORDER BY position")
        .bind(entry)
        .fetch_all(&app.db)
        .await?;
    let text = rows
        .iter()
        .map(|row| {
            format!(
                "[{}] {}",
                row.get::<String, _>("speaker"),
                row.get::<String, _>("text")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let identity_path = app.entry_dir(entry).join("identity-analysis.json");
    let notes = if identity_path.exists() {
        serde_json::from_slice(&tokio::fs::read(&identity_path).await?).map_err(anyhow::Error::from)
    } else {
        provider::enrich(app, &text.chars().take(100000).collect::<String>()).await
    };
    match notes {
        Ok(notes) => {
            let pending = app.entry_dir(entry).join("identity-analysis.partial.json");
            tokio::fs::write(&pending, serde_json::to_vec_pretty(&notes)?).await?;
            commit_artifact(
                &pending,
                &app.entry_dir(entry).join("identity-analysis.json"),
            )
            .await?;
            apply_name_evidence(app, entry, &notes).await?;
        }
        Err(error) => warnings.push(format!("Speaker name analysis unavailable: {error}")),
    }
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    sqlx::query("UPDATE entries SET stage='complete',active_stage=NULL,failure=NULL,requested_work=NULL,pause_requested=0,warnings=?,updated_at=? WHERE id=?")
        .bind(serde_json::to_string(&warnings)?).bind(now()).bind(entry).execute(&app.db).await?;
    let _ = tokio::fs::remove_file(merged).await;
    for index in 0..source_count {
        let _ = tokio::fs::remove_file(app.entry_dir(entry).join(format!("source-{index}"))).await;
    }
    Ok(())
}

pub async fn process(app: &App, entry: &str) -> Result<()> {
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    stage(app, entry, "preparing").await?;
    let dir = app.entry_dir(entry);
    let (merged, source_count) = prepare_working_audio(app, entry).await?;
    let playback = dir.join("playback.m4a");
    if !playback.exists() {
        let pending = dir.join("playback.partial.m4a");
        audio::ffmpeg(&[
            "-i".into(),
            merged.display().to_string(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "96k".into(),
            "-movflags".into(),
            "+faststart".into(),
            pending.display().to_string(),
        ])
        .await?;
        commit_artifact(&pending, &playback).await?;
    }
    let duration: i64 = sqlx::query_scalar("SELECT duration_ms FROM entries WHERE id=?")
        .bind(entry)
        .fetch_one(&app.db)
        .await?;
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    let count = (duration + 599999) / 600000;
    stage(app, entry, "transcribing").await?;
    let mut all: Vec<(i64, Turn)> = vec![];
    for p in 0..count {
        if pause_at_checkpoint(app, entry).await? {
            return Ok(());
        }
        let prior = sqlx::query("SELECT state,raw_json FROM parts WHERE entry_id=? AND part=?")
            .bind(entry)
            .bind(p)
            .fetch_optional(&app.db)
            .await?;
        let (turns, raw) = if let Some(row) = prior {
            let raw: Option<String> = row.get("raw_json");
            if let Some(raw) = raw {
                let raw: Value = serde_json::from_str(&raw)?;
                let turns: Vec<Turn> = serde_json::from_value(raw["segments"].clone())?;
                (turns, raw)
            } else {
                anyhow::bail!(
                    "A transcription request may have been submitted. Retry explicitly to authorize another request."
                )
            }
        } else {
            let part = dir.join(format!("part-{p}.wav"));
            audio::excerpt(
                &merged,
                p * 600000,
                (duration - p * 600000).min(600000),
                &part,
            )
            .await?;
            sqlx::query("INSERT INTO parts(entry_id,part,state)VALUES(?,?,'submitted')")
                .bind(entry)
                .bind(p)
                .execute(&app.db)
                .await?;
            let result = provider::transcribe(app, &part).await?;
            sqlx::query("UPDATE parts SET state='complete',raw_json=? WHERE entry_id=? AND part=?")
                .bind(serde_json::to_string(&result.1)?)
                .bind(entry)
                .bind(p)
                .execute(&app.db)
                .await?;
            tokio::fs::write(
                dir.join(format!("part-{p}.json")),
                serde_json::to_vec_pretty(&result.1)?,
            )
            .await?;
            let _ = tokio::fs::remove_file(part).await;
            result
        };
        if pause_at_checkpoint(app, entry).await? {
            return Ok(());
        }
        let _ = raw;
        all.extend(turns.into_iter().map(|t| (p, t)));
    }
    let mut tx = app.db.begin().await?;
    // IDs are deterministic across retries; speaker labels are scoped to the provider request.
    for (position, (part, t)) in all.iter().enumerate() {
        let label = format!("{part}:{}", t.speaker);
        let start = part * 600000 + (t.start * 1000.).round() as i64;
        let end = (part * 600000 + (t.end * 1000.).round() as i64).min(duration);
        sqlx::query("INSERT OR IGNORE INTO speakers(entry_id,label)VALUES(?,?)")
            .bind(entry)
            .bind(&label)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO segments(id,entry_id,position,speaker,start_ms,end_ms,text)VALUES(?,?,?,?,?,?,?) ON CONFLICT(entry_id,position)DO UPDATE SET start_ms=excluded.start_ms,end_ms=excluded.end_ms,text=excluded.text").bind(format!("{entry}:{position}")).bind(entry).bind(position as i64).bind(label).bind(start).bind(end).bind(&t.text).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    let mut warnings: Vec<String> = vec![];
    stage(app, entry, "recognizing").await?;
    if let Err(e) = voice::analyze(app, entry, &merged).await {
        warnings.push(format!("Speaker recognition unavailable: {e}"));
    }
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    stage(app, entry, "enriching").await?;
    let text = all
        .iter()
        .map(|(part, t)| format!("[{part}:{}] {}", t.speaker, t.text))
        .collect::<Vec<_>>()
        .join("\n");
    let enrichment_path = dir.join("enrichment.json");
    let enriched = if enrichment_path.exists() {
        serde_json::from_slice::<Value>(&tokio::fs::read(&enrichment_path).await?)
            .map_err(anyhow::Error::from)
    } else {
        provider::enrich(app, &text.chars().take(100000).collect::<String>()).await
    };
    match enriched {
        Ok(notes) => {
            let pending = dir.join("enrichment.partial.json");
            tokio::fs::write(&pending, serde_json::to_vec_pretty(&notes)?).await?;
            commit_artifact(&pending, &enrichment_path).await?;
            let title = notes["title"].as_str().unwrap_or("");
            let summary = notes["summary"].as_str().unwrap_or("");
            if !title.trim().is_empty() {
                sqlx::query("UPDATE entries SET title=CASE WHEN title_manual=1 THEN title ELSE ? END,summary=? WHERE id=? AND summary IS NULL").bind(title.chars().take(240).collect::<String>()).bind(summary).bind(entry).execute(&app.db).await?;
            }
            apply_name_evidence(app, entry, &notes).await?;
        }
        Err(e) => warnings.push(format!("Summary unavailable: {e}")),
    }
    if pause_at_checkpoint(app, entry).await? {
        return Ok(());
    }
    stage(app, entry, "indexing").await?;
    if let Err(e) = index(app, entry).await {
        warnings.push(format!("Semantic search unavailable: {e}"));
    }
    sqlx::query("UPDATE entries SET stage='complete',active_stage=NULL,failure=NULL,pause_requested=0,requested_work=NULL,warnings=?,audio_path='playback.m4a',updated_at=? WHERE id=?").bind(serde_json::to_string(&warnings)?).bind(now()).bind(entry).execute(&app.db).await?;
    let _ = tokio::fs::remove_file(&merged).await;
    for i in 0..source_count {
        let _ = tokio::fs::remove_file(dir.join(format!("source-{i}"))).await;
    }
    Ok(())
}
async fn apply_name_evidence(app: &App, entry: &str, notes: &Value) -> Result<()> {
    if let Some(names) = notes["names"].as_array() {
        for n in names {
            let (Some(label), Some(name), Some(quote)) = (
                n["speaker"].as_str(),
                n["name"].as_str(),
                n["quote"].as_str(),
            ) else {
                continue;
            };
            if name.is_empty() || name.len() > 120 || quote.is_empty() {
                continue;
            }
            let spoken: Vec<String> =
                sqlx::query_scalar("SELECT text FROM segments WHERE entry_id=? AND speaker=?")
                    .bind(entry)
                    .bind(label)
                    .fetch_all(&app.db)
                    .await?;
            if !spoken.iter().any(|s| s.contains(quote))
                || !quote.to_lowercase().contains(&name.to_lowercase())
            {
                continue;
            }
            // A model finding is evidence for review, not an automatic voice enrollment.
            sqlx::query(
                "UPDATE speakers SET evidence=? WHERE entry_id=? AND label=? AND state!='manual'",
            )
            .bind(
                json!([{"source":"transcript","name":name,"quote":quote,"applied":false}])
                    .to_string(),
            )
            .bind(entry)
            .bind(label)
            .execute(&app.db)
            .await?;
        }
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
    let texts: Vec<String> = chunks.iter().map(|(_, _, t)| t.clone()).collect();
    let mut vectors = HashMap::new();
    let mut embedding_error = None;
    for (batch, values) in texts.chunks(32).enumerate() {
        match provider::embeddings(app, values).await {
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
