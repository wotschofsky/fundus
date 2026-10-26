mod common;
use common::{Options, Provider, until};
use std::sync::Arc;
use transcript_vault::{
    audio,
    config::{App, Config, now},
    pipeline, retrieval, voice,
};

fn wav(path: &std::path::Path) {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for i in 0..(48000 * 12) {
        let value = ((i as f64 * 0.03).sin() * 8000000.) as i32;
        writer.write_sample(value).unwrap();
        writer.write_sample(-value).unwrap();
    }
    writer.finalize().unwrap();
}
async fn setup(
    options: Options,
) -> (
    tempfile::TempDir,
    App,
    Arc<Provider>,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::from_env().unwrap();
    config.data = dir.path().join("vault");
    config.voice_model = None;
    let mut app = App::new(config).await.unwrap();
    let (provider, task) = common::connect(&mut app, options).await;
    (dir, app, provider, task)
}
async fn add(app: &App, source: &std::path::Path, batch: &str, uid: &str) {
    tokio::fs::copy(source, app.upload_path(uid)).await.unwrap();
    let length = std::fs::metadata(source).unwrap().len() as i64;
    sqlx::query(
        "INSERT INTO imports(id,state,recorder_timezone,created_at)VALUES(?,'verifying','UTC',?)",
    )
    .bind(batch)
    .bind(now())
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO uploads(id,import_id,filename,original_name,relative_path,length,offset,state)VALUES(?,?,'recording.wav','recording.wav','recording.wav',?,?,'received')").bind(uid).bind(batch).bind(length).bind(length).execute(&app.db).await.unwrap();
}
/// One finalized entry from a synthetic 12-second stereo recording.
async fn entry(app: &App, dir: &std::path::Path, batch: &str) -> String {
    let source = dir.join(format!("{batch}.wav"));
    wav(&source);
    add(app, &source, batch, &format!("{batch}-upload")).await;
    pipeline::finalize(app, batch).await.unwrap();
    sqlx::query_scalar("SELECT id FROM entries ORDER BY created_at DESC LIMIT 1")
        .fetch_one(&app.db)
        .await
        .unwrap()
}
async fn text(app: &App, sql: &str, entry: &str) -> String {
    sqlx::query_scalar(sql)
        .bind(entry)
        .fetch_one(&app.db)
        .await
        .unwrap()
}
async fn stage(app: &App, entry: &str) -> String {
    text(app, "SELECT stage FROM entries WHERE id=?", entry).await
}

#[tokio::test]
async fn audio_to_search_checkpoints_and_pcm_deduplication() {
    let (dir, app, provider, task) = setup(Options {
        pending_polls: 1,
        ..Default::default()
    })
    .await;
    let entry = entry(&app, dir.path(), "batch-a").await;
    // Interrupted preparation leaves only partial artifacts; a resume replaces them.
    std::fs::create_dir_all(app.entry_dir(&entry)).unwrap();
    for name in [
        "playback.partial.m4a",
        "transcription.partial.flac",
        "enrichment.partial.json",
    ] {
        std::fs::write(app.entry_dir(&entry).join(name), b"interrupted").unwrap();
    }
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(stage(&app, &entry).await, "complete");
    assert_eq!(provider.submits(), 1);
    // The job waited through a pending poll, and the whole recording went up as one rendition.
    assert!(provider.polls.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    let request = provider.transcripts.lock().unwrap()[0].clone();
    assert_eq!(request["speaker_labels"], true);
    assert_eq!(request["speech_models"][0], "universal-3-5-pro");
    assert_eq!(request["language_codes"], serde_json::json!(["de", "en"]));
    assert!(request.get("prompt").is_none(), "no context, no prompt");
    let dir_entry = app.entry_dir(&entry);
    assert!(dir_entry.join("playback.m4a").exists());
    assert!(dir_entry.join("transcription.json").exists());
    assert!(
        !dir_entry.join("transcription.flac").exists(),
        "rendition is temporary"
    );
    let detail = retrieval::get(&app, &entry).await.unwrap();
    assert_eq!(detail["title"], "Cedar launch planning");
    assert_eq!(detail["audio_available"], true);
    // Only the unconfigured voice model is reported; summaries and search worked.
    assert_eq!(detail["warnings"].as_array().unwrap().len(), 1);
    assert!(
        detail["warnings"][0]
            .as_str()
            .unwrap()
            .starts_with("Speaker recognition unavailable")
    );
    assert_eq!(detail["speakers"].as_array().unwrap().len(), 2);
    // Name evidence was validated against speaker A's own words; it never enrolls a voice.
    assert_eq!(detail["speakers"][0]["label"], "A");
    assert_eq!(detail["speakers"][0]["evidence"][0]["name"], "Morgan");
    assert_eq!(detail["speakers"][0]["person_id"], serde_json::Value::Null);
    let result = retrieval::search(
        &app,
        retrieval::Search {
            query: "cedar".into(),
            filters: Default::default(),
        },
    )
    .await
    .unwrap();
    assert_eq!(result["mode"], "hybrid");
    assert_eq!(result["results"][0]["entry_id"], entry);
    assert!(
        result["results"][0]["url"]
            .as_str()
            .unwrap()
            .contains("?t=")
    );
    let transcript = retrieval::transcript(
        &app,
        retrieval::Read {
            entry_id: entry.clone(),
            cursor: Some(0),
            limit: Some(1),
            from_ms: None,
            to_ms: None,
        },
        300,
    )
    .await
    .unwrap();
    assert_eq!(transcript["segments"].as_array().unwrap().len(), 1);
    assert_eq!(transcript["next_cursor"], 1);
    // An explicit retry reuses the saved result rather than billing transcription again.
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(provider.submits(), 1);
    let source = dir.path().join("batch-a.wav");
    let flac = dir.path().join("source.flac");
    audio::ffmpeg(&[
        "-i".into(),
        source.display().to_string(),
        "-c:a".into(),
        "flac".into(),
        flac.display().to_string(),
    ])
    .await
    .unwrap();
    assert_eq!(
        audio::verify(&source).await.unwrap().1,
        audio::verify(&flac).await.unwrap().1
    );
    add(&app, &flac, "batch-b", "upload-b").await;
    pipeline::finalize(&app, "batch-b").await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let duplicate: String = sqlx::query_scalar("SELECT state FROM uploads WHERE id='upload-b'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(duplicate, "duplicate");
    // A manual correction changes the sample owner.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_samples")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(count, 0);
    for id in ["morgan", "avery"] {
        sqlx::query("INSERT INTO people(id,name)VALUES(?,?)")
            .bind(id)
            .bind(id)
            .execute(&app.db)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO voice_probes(entry_id,speaker,model,vector_json)VALUES(?,'A',?,'[1.0,0.0]')",
    )
    .bind(&entry)
    .bind(voice::MODEL)
    .execute(&app.db)
    .await
    .unwrap();
    voice::assign(&app, &entry, "A", Some("morgan"))
        .await
        .unwrap();
    voice::assign(&app, &entry, "A", Some("avery"))
        .await
        .unwrap();
    let owner: String = sqlx::query_scalar("SELECT person_id FROM voice_samples")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(owner, "avery");
    task.abort();
}

#[tokio::test]
async fn graceful_shutdown_saves_the_job_and_restart_resumes_polling() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (dir, app, provider, task) = setup(Options {
        gate: Some(gate.clone()),
        pending_polls: 1,
        ..Default::default()
    })
    .await;
    let entry = entry(&app, dir.path(), "shutdown").await;
    let stop = tokio_util::sync::CancellationToken::new();
    let mut worker = tokio::spawn(pipeline::worker(app.clone(), stop.clone()));
    until(|| provider.submits() == 1).await;
    sqlx::query("INSERT INTO entries(id,fingerprint,title,duration_ms,timezone,stage,created_at,updated_at) VALUES('next-job','next-job','Synthetic queued job',1,'UTC','queued',?,?)")
        .bind(now()).bind(now()).execute(&app.db).await.unwrap();
    stop.cancel();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut worker)
            .await
            .is_err(),
        "Shutdown discarded the in-flight transcription request instead of draining it"
    );
    gate.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap();
    // The job ID was saved; the remaining wait was abandoned rather than holding up shutdown.
    let job = text(
        &app,
        "SELECT remote_id FROM transcriptions WHERE entry_id=?",
        &entry,
    )
    .await;
    assert_eq!(job, "job-0");
    assert_eq!(stage(&app, &entry).await, "transcribing");
    assert_eq!(
        stage(&app, "next-job").await,
        "queued",
        "Shutdown must not start the next job"
    );
    sqlx::query("DELETE FROM entries WHERE id='next-job'")
        .execute(&app.db)
        .await
        .unwrap();
    // After a restart the same job is polled to completion, never submitted again.
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(pipeline::worker(app.clone(), stop.clone()));
    let mut done = false;
    for _ in 0..1000 {
        if stage(&app, &entry).await == "complete" {
            done = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    stop.cancel();
    worker.await.unwrap();
    assert!(done, "the resumed job did not complete");
    assert_eq!(provider.submits(), 1);
    task.abort();
}

#[tokio::test]
async fn ambiguous_submission_requires_explicit_retry_and_saved_jobs_resume() {
    let (_dir, app, provider, task) = setup(Options::default()).await;
    for id in ["interrupted", "resumable"] {
        sqlx::query("INSERT INTO entries(id,fingerprint,title,duration_ms,timezone,stage,active_stage,created_at,updated_at) VALUES(?,?,'Synthetic job',1,'UTC','transcribing','transcribing',?,?)")
            .bind(id).bind(id).bind(now()).bind(now()).execute(&app.db).await.unwrap();
    }
    sqlx::query("INSERT INTO transcriptions(entry_id,state,submitted_at) VALUES('interrupted','submitted',?)")
        .bind(now())
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO transcriptions(entry_id,state,remote_id,submitted_at) VALUES('resumable','processing','job-7',?)")
        .bind(now())
        .execute(&app.db)
        .await
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    pipeline::worker(app.clone(), stop).await;
    let (stage_name, failure): (String, String) =
        sqlx::query_as("SELECT stage,failure FROM entries WHERE id='interrupted'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(stage_name, "failed");
    assert!(failure.contains("Retry explicitly"));
    assert_eq!(stage(&app, "resumable").await, "queued");
    assert_eq!(provider.submits(), 0);
    // Processing on its own never resubmits an ambiguous request.
    let error = pipeline::process(&app, "interrupted").await.unwrap_err();
    assert!(error.to_string().contains("Retry explicitly"));
    // The explicit Retry clears only an ambiguous marker; a saved job ID survives it.
    let admin = transcript_vault::auth::issue(&app, "test owner", "admin")
        .await
        .unwrap();
    let router =
        transcript_vault::api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    sqlx::query("UPDATE entries SET stage='failed' WHERE id IN ('interrupted','resumable')")
        .execute(&app.db)
        .await
        .unwrap();
    for id in ["interrupted", "resumable"] {
        use tower::ServiceExt;
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/v1/entries/{id}/retry"))
                    .header("Authorization", format!("Bearer {admin}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
    let remaining: Vec<String> = sqlx::query_scalar("SELECT entry_id FROM transcriptions")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(remaining, vec!["resumable".to_string()]);
    task.abort();
}

#[tokio::test]
async fn failed_job_clears_its_checkpoint_so_retry_resubmits() {
    let (dir, app, provider, task) = setup(Options {
        failing_jobs: 1,
        ..Default::default()
    })
    .await;
    let entry = entry(&app, dir.path(), "failing").await;
    let error = pipeline::process(&app, &entry).await.unwrap_err();
    assert!(error.to_string().contains("synthetic failure"));
    let saved: i64 = sqlx::query_scalar("SELECT count(*) FROM transcriptions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(saved, 0);
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(provider.submits(), 2);
    assert_eq!(stage(&app, &entry).await, "complete");
    task.abort();
}

#[tokio::test]
async fn optional_providers_degrade_to_keyword_search_and_a_visible_warning() {
    let (dir, mut app, provider, task) = setup(Options::default()).await;
    let config = Arc::make_mut(&mut app.config);
    config.groq_key = String::new();
    config.openai_key = String::new();
    let entry = entry(&app, dir.path(), "optional").await;
    pipeline::process(&app, &entry).await.unwrap();
    let detail = retrieval::get(&app, &entry).await.unwrap();
    assert_eq!(detail["stage"], "complete");
    let warnings: Vec<&str> = detail["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("Summary unavailable") && w.contains("GROQ_API_KEY"))
    );
    assert!(
        !warnings.iter().any(|w| w.starts_with("Semantic search")),
        "{warnings:?}"
    );
    assert!(provider.chats.lock().unwrap().is_empty());
    let result = retrieval::search(
        &app,
        retrieval::Search {
            query: "cedar".into(),
            filters: Default::default(),
        },
    )
    .await
    .unwrap();
    assert_eq!(result["mode"], "keyword");
    assert_eq!(result["partial_results"], false);
    assert_eq!(result["results"][0]["entry_id"], entry);
    task.abort();
}

#[tokio::test]
async fn pause_waits_for_the_job_id_and_resume_polls_the_same_job() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (dir, app, provider, task) = setup(Options {
        gate: Some(gate.clone()),
        pending_polls: 1,
        ..Default::default()
    })
    .await;
    let entry = entry(&app, dir.path(), "pause").await;
    let (work_app, work_entry) = (app.clone(), entry.clone());
    let process = tokio::spawn(async move { pipeline::process(&work_app, &work_entry).await });
    until(|| provider.submits() == 1).await;
    // Audio is playable while the transcription is still running.
    assert_eq!(
        retrieval::get(&app, &entry).await.unwrap()["audio_available"],
        true
    );
    sqlx::query("UPDATE entries SET pause_requested=1 WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    assert!(
        !process.is_finished(),
        "A pause must not discard an in-flight transcription request"
    );
    gate.notify_one();
    process.await.unwrap().unwrap();
    let (stage_name, requested): (String, bool) =
        sqlx::query_as("SELECT stage,pause_requested FROM entries WHERE id=?")
            .bind(&entry)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(stage_name, "paused");
    assert!(!requested);
    assert_eq!(
        text(
            &app,
            "SELECT remote_id FROM transcriptions WHERE entry_id=?",
            &entry
        )
        .await,
        "job-0"
    );
    // Clean restarts keep a deliberately paused entry paused.
    let shutdown = tokio_util::sync::CancellationToken::new();
    shutdown.cancel();
    pipeline::worker(app.clone(), shutdown).await;
    assert_eq!(stage(&app, &entry).await, "paused");
    sqlx::query("UPDATE entries SET stage='queued' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(provider.submits(), 1);
    assert_eq!(stage(&app, &entry).await, "complete");
    task.abort();
}

#[tokio::test]
async fn speaker_reanalysis_preserves_transcription_and_manual_identities() {
    let (dir, app, provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "identity").await;
    pipeline::process(&app, &entry).await.unwrap();
    sqlx::query("INSERT INTO people(id,name)VALUES('manual-person','Synthetic person')")
        .execute(&app.db)
        .await
        .unwrap();
    voice::assign(&app, &entry, "A", Some("manual-person"))
        .await
        .unwrap();
    // The summary names speakers by placeholder, so it follows the assignment.
    let summary = |app: App, entry: String| async move {
        transcript_vault::retrieval::get(&app, &entry)
            .await
            .unwrap()["summary"]
            .clone()
    };
    assert_eq!(
        summary(app.clone(), entry.clone()).await,
        "Synthetic person plans the launch with Speaker 2."
    );
    sqlx::query("UPDATE entries SET summary='Old summary' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    let transcript: Vec<(String, String)> =
        sqlx::query_as("SELECT id,text FROM segments WHERE entry_id=? ORDER BY position")
            .bind(&entry)
            .fetch_all(&app.db)
            .await
            .unwrap();
    pipeline::reanalyze_speakers(&app, &entry).await.unwrap();
    assert_eq!(provider.submits(), 1);
    let after: Vec<(String, String)> =
        sqlx::query_as("SELECT id,text FROM segments WHERE entry_id=? ORDER BY position")
            .bind(&entry)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(transcript, after);
    let identity: (String, String) =
        sqlx::query_as("SELECT person_id,state FROM speakers WHERE entry_id=? AND label='A'")
            .bind(&entry)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(identity, ("manual-person".into(), "manual".into()));
    // Reanalyze writes fresh notes, e.g. for summaries from before placeholders.
    assert_eq!(
        summary(app.clone(), entry.clone()).await,
        "Synthetic person plans the launch with Speaker 2."
    );
    task.abort();
}

#[tokio::test]
async fn retranscribing_a_legacy_entry_replaces_its_part_labels() {
    let (dir, app, _provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "legacy").await;
    // A transcript from the former ten-minute-part pipeline, with a person on one part label.
    sqlx::query("INSERT INTO people(id,name)VALUES('p','Synthetic')")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO speakers(entry_id,label,person_id,state)VALUES(?,'1:A','p','manual')")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    for position in 0..5 {
        sqlx::query("INSERT INTO segments(id,entry_id,position,speaker,start_ms,end_ms,text)VALUES(?,?,?,'1:A',0,1,'old')")
            .bind(format!("{entry}:{position}"))
            .bind(&entry)
            .bind(position)
            .execute(&app.db)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO parts(entry_id,part,state,raw_json)VALUES(?,0,'complete','{\"segments\":[]}')",
    )
    .bind(&entry)
    .execute(&app.db)
    .await
    .unwrap();
    // Its notes describe the old labels and text; they must not survive the new transcript.
    tokio::fs::create_dir_all(app.entry_dir(&entry))
        .await
        .unwrap();
    tokio::fs::write(
        app.entry_dir(&entry).join("enrichment.json"),
        r#"{"title":"Old","summary":"Old summary","names":[{"speaker":"1:A","name":"Old","quote":"old"}]}"#,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE entries SET summary='Old summary' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(
        text(&app, "SELECT summary FROM entries WHERE id=?", &entry).await,
        "{A} plans the launch with {B}."
    );
    assert!(
        text(
            &app,
            "SELECT evidence FROM speakers WHERE entry_id=? AND label='A'",
            &entry
        )
        .await
        .contains("Morgan")
    );
    let speakers: Vec<String> =
        sqlx::query_scalar("SELECT label FROM speakers WHERE entry_id=? ORDER BY label")
            .bind(&entry)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(speakers, vec!["A", "B"]);
    let segments: i64 = sqlx::query_scalar("SELECT count(*) FROM segments WHERE entry_id=?")
        .bind(&entry)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(segments, 3);
    task.abort();
}

#[tokio::test]
async fn a_lost_create_answer_is_found_by_its_upload_url_and_paid_once() {
    let (dir, app, provider, task) = setup(Options {
        lost_answers: 1,
        ..Default::default()
    })
    .await;
    let entry = entry(&app, dir.path(), "lost").await;
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(stage(&app, &entry).await, "complete");
    assert_eq!(provider.submits(), 1);
    // The vault keeps the result; the job and its audio are removed at AssemblyAI.
    assert_eq!(provider.deletes(), 1);
    task.abort();
}

#[tokio::test]
async fn a_restart_after_an_unanswered_submit_resumes_the_job_it_finds() {
    let (dir, app, provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "restart").await;
    provider
        .jobs
        .lock()
        .unwrap()
        .push(("job-5".into(), "https://cdn.example/earlier".into()));
    sqlx::query("INSERT INTO transcriptions(entry_id,state,audio_url,submitted_at)VALUES(?,'submitted','https://cdn.example/earlier',?)")
        .bind(&entry).bind(now()).execute(&app.db).await.unwrap();
    sqlx::query("UPDATE entries SET stage='transcribing' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    pipeline::worker(app.clone(), stop).await;
    assert_eq!(stage(&app, &entry).await, "queued");
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(stage(&app, &entry).await, "complete");
    assert_eq!(provider.submits(), 0);
    task.abort();
}

#[tokio::test]
async fn a_job_gone_at_assemblyai_fails_cleanly_so_retry_submits_again() {
    let (dir, app, provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "gone").await;
    sqlx::query("INSERT INTO transcriptions(entry_id,state,remote_id,audio_url,submitted_at)VALUES(?,'processing','gone','https://cdn.example/gone',?)")
        .bind(&entry).bind(now()).execute(&app.db).await.unwrap();
    let error = pipeline::process(&app, &entry).await.unwrap_err();
    assert!(error.to_string().contains("no longer has"), "{error}");
    assert_eq!(stage(&app, &entry).await, "failed");
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(provider.submits(), 1);
    assert_eq!(stage(&app, &entry).await, "complete");
    task.abort();
}

#[tokio::test]
async fn recordings_over_assemblyais_limit_are_refused_before_upload() {
    let (dir, app, provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "long").await;
    sqlx::query("UPDATE entries SET duration_ms=? WHERE id=?")
        .bind(10 * 60 * 60 * 1000 + 1)
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    let error = pipeline::process(&app, &entry).await.unwrap_err();
    assert!(error.to_string().contains("10 hours"), "{error}");
    assert_eq!(
        provider.uploads.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    task.abort();
}

#[tokio::test]
async fn read_tokens_cannot_list_trash() {
    use tower::ServiceExt;
    let (_dir, app, _provider, task) = setup(Options::default()).await;
    let router =
        transcript_vault::api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    for (scope, expected) in [("read", 403), ("admin", 200)] {
        let token = transcript_vault::auth::issue(&app, "test", scope)
            .await
            .unwrap();
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::get("/api/v1/entries?trash=true")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{scope}");
    }
    task.abort();
}
