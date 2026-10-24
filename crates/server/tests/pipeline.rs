use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
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
async fn setup() -> (
    tempfile::TempDir,
    App,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    setup_with_gate(None).await
}
async fn setup_with_gate(
    gate: Option<Arc<tokio::sync::Notify>>,
) -> (
    tempfile::TempDir,
    App,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let provider=Router::new().route("/audio/transcriptions",post(move |_: axum::body::Bytes| {let c=count.clone();let gate=gate.clone();async move{c.fetch_add(1,Ordering::SeqCst);if let Some(gate)=gate { gate.notified().await; } Json(json!({"segments":[{"speaker":"A","start":0.,"end":5.,"text":"I'm Morgan. The cedar launch is on Tuesday."},{"speaker":"A","start":6.,"end":12.,"text":"We agreed to send the proposal tomorrow."}]}))}}))
    .route("/responses",post(||async{Json(json!({"output":[{"content":[{"type":"output_text","text":json!({"title":"Cedar launch planning","summary":"Discussed the launch and next proposal.","names":[{"speaker":"0:A","name":"Morgan","quote":"I'm Morgan."}]}).to_string()}]}]}))}))
    .route("/embeddings",post(|Json(body):Json<Value>|async move{let n=body["input"].as_array().map(|x|x.len()).unwrap_or(1);Json(json!({"data":(0..n).map(|i|json!({"index":i,"embedding":[1.,0.,0.]})).collect::<Vec<_>>()}))}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
    let mut config = Config::from_env().unwrap();
    config.data = dir.path().join("vault");
    config.openai_url = format!("http://{address}");
    config.openai_key = "synthetic-only".into();
    config.voice_model = None;
    let app = App::new(config).await.unwrap();
    (dir, app, calls, task)
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
#[tokio::test]
async fn audio_to_search_checkpoints_and_pcm_deduplication() {
    let (dir, app, calls, provider) = setup().await;
    let source = dir.path().join("source.wav");
    wav(&source);
    add(&app, &source, "batch-a", "upload-a").await;
    pipeline::finalize(&app, "batch-a").await.unwrap();
    let entry: String = sqlx::query_scalar("SELECT id FROM entries")
        .fetch_one(&app.db)
        .await
        .unwrap();
    // Interrupted preparation leaves only partial artifacts; a resume replaces them.
    std::fs::create_dir_all(app.entry_dir(&entry)).unwrap();
    for name in [
        "source-0.partial",
        "working.partial.wav",
        "playback.partial.m4a",
        "enrichment.partial.json",
    ] {
        std::fs::write(app.entry_dir(&entry).join(name), b"interrupted").unwrap();
    }
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        retrieval::list(
            &app,
            retrieval::Filters {
                stage: Some(String::new()),
                ..Default::default()
            }
        )
        .await
        .unwrap()["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let result = retrieval::search(
        &app,
        retrieval::Search {
            query: "cedar".into(),
            filters: Default::default(),
        },
    )
    .await
    .unwrap();
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
    )
    .await
    .unwrap();
    assert_eq!(transcript["segments"].as_array().unwrap().len(), 1);
    assert_eq!(transcript["next_cursor"], 1);
    // An explicit retry reuses saved provider output rather than billing transcription again.
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
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
    // Name evidence never enrolls a voice. A manual correction changes the sample owner.
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
        "INSERT INTO voice_probes(entry_id,speaker,model,vector_json)VALUES(?,'0:A',?,'[1.0,0.0]')",
    )
    .bind(&entry)
    .bind(voice::MODEL)
    .execute(&app.db)
    .await
    .unwrap();
    voice::assign(&app, &entry, "0:A", Some("morgan"))
        .await
        .unwrap();
    voice::assign(&app, &entry, "0:A", Some("avery"))
        .await
        .unwrap();
    let owner: String = sqlx::query_scalar("SELECT person_id FROM voice_samples")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(owner, "avery");
    provider.abort();
}

#[tokio::test]
async fn graceful_shutdown_preserves_in_flight_transcription_response() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (dir, app, calls, provider) = setup_with_gate(Some(gate.clone())).await;
    let source = dir.path().join("source.wav");
    wav(&source);
    add(&app, &source, "shutdown-batch", "shutdown-upload").await;
    pipeline::finalize(&app, "shutdown-batch").await.unwrap();
    let entry: String = sqlx::query_scalar("SELECT id FROM entries")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let mut worker = tokio::spawn(pipeline::worker(app.clone(), stop.clone()));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    sqlx::query("INSERT INTO entries(id,fingerprint,title,duration_ms,timezone,stage,created_at,updated_at) VALUES('next-job','next-job','Synthetic queued job',1,'UTC','queued',?,?)")
        .bind(now()).bind(now()).execute(&app.db).await.unwrap();
    stop.cancel();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut worker)
            .await
            .is_err(),
        "Shutdown discarded the in-flight provider request instead of draining it"
    );
    gate.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap();
    let (state, saved): (String, bool) =
        sqlx::query_as("SELECT state,raw_json IS NOT NULL FROM parts WHERE entry_id=? AND part=0")
            .bind(&entry)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(state, "complete");
    assert!(saved);
    let stage: String = sqlx::query_scalar("SELECT stage FROM entries WHERE id=?")
        .bind(&entry)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stage, "complete");
    let next: String = sqlx::query_scalar("SELECT stage FROM entries WHERE id='next-job'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(next, "queued", "Shutdown must not start the next job");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // A subsequent explicit processing pass must reuse the saved response.
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        pipeline::process(&app, &entry),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    provider.abort();
}

#[tokio::test]
async fn hard_crash_checkpoint_still_requires_explicit_retry() {
    let (_dir, app, calls, provider) = setup().await;
    sqlx::query("INSERT INTO entries(id,fingerprint,title,duration_ms,timezone,stage,active_stage,created_at,updated_at) VALUES('interrupted','interrupted','Synthetic interrupted job',1,'UTC','transcribing','transcribing',?,?)")
        .bind(now()).bind(now()).execute(&app.db).await.unwrap();
    sqlx::query("INSERT INTO parts(entry_id,part,state) VALUES('interrupted',0,'submitted')")
        .execute(&app.db)
        .await
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    pipeline::worker(app.clone(), stop).await;
    let (stage, failure): (String, String) =
        sqlx::query_as("SELECT stage,failure FROM entries WHERE id='interrupted'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(stage, "failed");
    assert!(failure.contains("Retry explicitly"));
    let missing_response: bool = sqlx::query_scalar(
        "SELECT raw_json IS NULL FROM parts WHERE entry_id='interrupted' AND part=0",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(missing_response);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    provider.abort();
}

#[tokio::test]
async fn pause_waits_for_the_provider_response_and_resume_reuses_it() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (dir, app, calls, provider) = setup_with_gate(Some(gate.clone())).await;
    let source = dir.path().join("pause.wav");
    wav(&source);
    add(&app, &source, "pause-batch", "pause-upload").await;
    pipeline::finalize(&app, "pause-batch").await.unwrap();
    let entry: String = sqlx::query_scalar("SELECT id FROM entries")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let work_app = app.clone();
    let work_entry = entry.clone();
    let process = tokio::spawn(async move { pipeline::process(&work_app, &work_entry).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
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
    let (stage, requested): (String, bool) =
        sqlx::query_as("SELECT stage,pause_requested FROM entries WHERE id=?")
            .bind(&entry)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(stage, "paused");
    assert!(!requested);
    let saved: bool =
        sqlx::query_scalar("SELECT raw_json IS NOT NULL FROM parts WHERE entry_id=? AND part=0")
            .bind(&entry)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(saved);
    // Clean restarts keep a deliberately paused entry paused.
    let shutdown = tokio_util::sync::CancellationToken::new();
    shutdown.cancel();
    pipeline::worker(app.clone(), shutdown).await;
    let stage: String = sqlx::query_scalar("SELECT stage FROM entries WHERE id=?")
        .bind(&entry)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stage, "paused");
    sqlx::query("UPDATE entries SET stage='queued' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let stage: String = sqlx::query_scalar("SELECT stage FROM entries WHERE id=?")
        .bind(&entry)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stage, "complete");
    provider.abort();
}

#[tokio::test]
async fn speaker_reanalysis_preserves_transcription_and_manual_identities() {
    let (dir, app, calls, provider) = setup().await;
    let source = dir.path().join("identity.wav");
    wav(&source);
    add(&app, &source, "identity-batch", "identity-upload").await;
    pipeline::finalize(&app, "identity-batch").await.unwrap();
    let entry: String = sqlx::query_scalar("SELECT id FROM entries")
        .fetch_one(&app.db)
        .await
        .unwrap();
    pipeline::process(&app, &entry).await.unwrap();
    sqlx::query("INSERT INTO people(id,name)VALUES('manual-person','Synthetic person')")
        .execute(&app.db)
        .await
        .unwrap();
    voice::assign(&app, &entry, "0:A", Some("manual-person"))
        .await
        .unwrap();
    let transcript: Vec<(String, String)> =
        sqlx::query_as("SELECT id,text FROM segments WHERE entry_id=? ORDER BY position")
            .bind(&entry)
            .fetch_all(&app.db)
            .await
            .unwrap();
    pipeline::reanalyze_speakers(&app, &entry).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let after: Vec<(String, String)> =
        sqlx::query_as("SELECT id,text FROM segments WHERE entry_id=? ORDER BY position")
            .bind(&entry)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(transcript, after);
    let identity: (String, String) =
        sqlx::query_as("SELECT person_id,state FROM speakers WHERE entry_id=? AND label='0:A'")
            .bind(&entry)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(identity, ("manual-person".into(), "manual".into()));
    provider.abort();
}
