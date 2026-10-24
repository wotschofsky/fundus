use axum::{
    Json, Router,
    body::Bytes,
    http::{Request, StatusCode},
    routing::post,
};
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;
use transcript_vault::{
    auth,
    config::{App, Config, now},
    grouping::{self, SessionPlan},
    pipeline,
};

async fn setup() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::from_env().unwrap();
    config.data = dir.path().join("vault");
    config.proxy_auth = true;
    config.public_url = "http://localhost:3000".parse().unwrap();
    config.voice_model = None;
    let app = App::new(config).await.unwrap();
    (dir, app)
}
fn wav(path: &std::path::Path, value: i16) {
    let mut out = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..8000 {
        out.write_sample(value).unwrap();
    }
    out.finalize().unwrap();
}
async fn add_batch(
    app: &App,
    batch: &str,
    files: &[(&str, &std::path::Path)],
    plans: &[SessionPlan],
) {
    sqlx::query("INSERT INTO imports(id,state,recorder_timezone,created_at,session_plan)VALUES(?,'verifying','UTC',?,?)")
        .bind(batch).bind(now()).bind(serde_json::to_string(plans).unwrap()).execute(&app.db).await.unwrap();
    for (index, (source, path)) in files.iter().enumerate() {
        let uid = format!("{batch}-{index}");
        tokio::fs::copy(path, app.upload_path(&uid)).await.unwrap();
        let size = std::fs::metadata(path).unwrap().len() as i64;
        let name = format!("TX01_MIC00{}_20261001_12000{}_orig.wav", index + 1, index);
        sqlx::query("INSERT INTO uploads(id,import_id,filename,original_name,relative_path,length,offset,state,source_id)VALUES(?,?,?,?,?,?,?,'received',?)")
            .bind(uid).bind(batch).bind(&name).bind(&name).bind(&name).bind(size).bind(size).bind(source).execute(&app.db).await.unwrap();
    }
}
fn plan(ids: &[&str], start: Option<u64>, end: Option<u64>) -> SessionPlan {
    SessionPlan {
        source_ids: ids.iter().map(|id| (*id).into()).collect(),
        trim_start_frame: start,
        trim_end_frame: end,
    }
}
async fn count(app: &App) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM entries")
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn explicit_sessions_preserve_manual_merges_splits_trims_and_duplicate_links() {
    let (dir, app) = setup().await;
    let a = dir.path().join("a.wav");
    let b = dir.path().join("b.wav");
    wav(&a, 1000);
    wav(&b, 2000);
    let files = [("source-a", a.as_path()), ("source-b", b.as_path())];
    let trimmed = [plan(&["source-a", "source-b"], Some(4000), Some(12000))];
    add_batch(&app, "first", &files, &trimmed).await;
    pipeline::finalize(&app, "first").await.unwrap();
    let (entry, duration, start, end): (String, i64, i64, i64) =
        sqlx::query_as("SELECT id,duration_ms,trim_start_frame,trim_end_frame FROM entries")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((duration, start, end), (1000, 4000, 12000));
    let recorded: String = sqlx::query_scalar("SELECT recorded_at FROM entries")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(recorded, "2026-10-01T12:00:00.500Z");
    assert_eq!(count(&app).await, 1);
    add_batch(&app, "duplicate", &files, &trimmed).await;
    pipeline::finalize(&app, "duplicate").await.unwrap();
    assert_eq!(count(&app).await, 1);
    let linked: String =
        sqlx::query_scalar("SELECT entry_id FROM import_entries WHERE import_id='duplicate'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(linked, entry);
    let duplicate_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM uploads WHERE import_id='duplicate' AND state='duplicate'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(duplicate_count, 2);
    // The same PCM can intentionally be retrimmed or regrouped into new entries.
    add_batch(
        &app,
        "retrim",
        &files,
        &[plan(&["source-a", "source-b"], Some(0), Some(8000))],
    )
    .await;
    pipeline::finalize(&app, "retrim").await.unwrap();
    assert_eq!(count(&app).await, 2);
    add_batch(
        &app,
        "split",
        &files,
        &[
            plan(&["source-a"], None, None),
            plan(&["source-b"], None, None),
        ],
    )
    .await;
    pipeline::finalize(&app, "split").await.unwrap();
    assert_eq!(count(&app).await, 4);
    add_batch(
        &app,
        "merge",
        &files,
        &[plan(&["source-a", "source-b"], None, None)],
    )
    .await;
    pipeline::finalize(&app, "merge").await.unwrap();
    assert_eq!(count(&app).await, 5);
    add_batch(
        &app,
        "gapped",
        &files,
        &[plan(&["source-a", "source-b"], Some(10000), Some(14000))],
    )
    .await;
    sqlx::query("UPDATE uploads SET original_name='TX01_MIC002_20261001_121000_orig.wav' WHERE import_id='gapped' AND source_id='source-b'").execute(&app.db).await.unwrap();
    pipeline::finalize(&app, "gapped").await.unwrap();
    let recorded:String=sqlx::query_scalar("SELECT e.recorded_at FROM import_entries ie JOIN entries e ON e.id=ie.entry_id WHERE ie.import_id='gapped'").fetch_one(&app.db).await.unwrap();
    assert_eq!(recorded, "2026-10-01T12:10:00.250Z");
}

#[tokio::test]
async fn remote_trim_is_sample_exact_across_the_source_boundary() {
    let (dir, mut app) = setup().await;
    let a = dir.path().join("a.wav");
    let b = dir.path().join("b.wav");
    wav(&a, 1000);
    wav(&b, 2000);
    add_batch(
        &app,
        "trim",
        &[("a", a.as_path()), ("b", b.as_path())],
        &[plan(&["a", "b"], Some(4000), Some(12000))],
    )
    .await;
    pipeline::finalize(&app, "trim").await.unwrap();
    let entry: String = sqlx::query_scalar("SELECT id FROM entries")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let count = Arc::new(AtomicUsize::new(0));
    let (notify, gate, calls) = (started.clone(), release.clone(), count.clone());
    let provider = Router::new()
        .route("/audio/transcriptions",post(move |_:Bytes| { let (notify,gate,calls)=(notify.clone(),gate.clone(),calls.clone()); async move {
            calls.fetch_add(1,Ordering::SeqCst); notify.notify_one(); gate.notified().await;
            Json(json!({"segments":[{"speaker":"A","start":0.,"end":1.,"text":"Synthetic trim test"}]}))
        }}))
        .route("/responses",post(|| async { Json(json!({"output":[{"content":[{"type":"output_text","text":"{\"title\":\"Trim fixture\",\"summary\":\"Synthetic audio\",\"names\":[]}"}]}]})) }))
        .route("/embeddings",post(|Json(body):Json<Value>| async move { Json(json!({"data":body["input"].as_array().unwrap().iter().enumerate().map(|(index,_)|json!({"index":index,"embedding":[1.,0.]})).collect::<Vec<_>>()})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let provider_task = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
    Arc::make_mut(&mut app.config).openai_url = format!("http://{address}");
    let work_app = app.clone();
    let work_entry = entry.clone();
    let process = tokio::spawn(async move { pipeline::process(&work_app, &work_entry).await });
    tokio::time::timeout(std::time::Duration::from_secs(15), started.notified())
        .await
        .unwrap();
    let mut working = hound::WavReader::open(app.entry_dir(&entry).join("working.wav")).unwrap();
    assert_eq!(working.spec().sample_rate, 8000);
    let samples: Vec<f32> = working.samples::<f32>().map(Result::unwrap).collect();
    assert_eq!(samples.len(), 8000);
    assert!(
        samples[..4000]
            .iter()
            .all(|sample| (*sample - 1000. / 32768.).abs() < 0.00001)
    );
    assert!(
        samples[4000..]
            .iter()
            .all(|sample| (*sample - 2000. / 32768.).abs() < 0.00001)
    );
    release.notify_one();
    process.await.unwrap().unwrap();
    // A processing retry reuses the transcript checkpoint for the exact trimmed audio.
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    provider_task.abort();
}

#[tokio::test]
async fn invalid_manifest_never_queues_partial_entries() {
    let (dir, app) = setup().await;
    let a = dir.path().join("a.wav");
    wav(&a, 1000);
    add_batch(
        &app,
        "bad",
        &[("a", a.as_path())],
        &[plan(&["a"], None, None), plan(&["missing"], None, None)],
    )
    .await;
    assert!(pipeline::finalize(&app, "bad").await.is_err());
    assert_eq!(count(&app).await, 0);
    assert!(
        grouping::validate_plan(&[plan(&["a"], None, None), plan(&["a"], None, None)]).is_err()
    );
}

async fn request(
    router: &axum::Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
    metadata: Option<String>,
) -> axum::response::Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json");
    if let Some(metadata) = metadata {
        request = request
            .header("Tus-Resumable", "1.0.0")
            .header("Upload-Length", "10")
            .header("Upload-Metadata", metadata);
    }
    router
        .clone()
        .oneshot(
            request
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn manifest_http_validation_and_tus_creation_are_resumable() {
    let (_dir, app) = setup().await;
    let token = auth::issue(&app, "test uploader", "upload").await.unwrap();
    let router =
        transcript_vault::api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    let bad = request(
        &router,
        "POST",
        "/api/v1/imports",
        &token,
        json!({"sessions":[{"source_ids":["same"]},{"source_ids":["same"]}]}),
        None,
    )
    .await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let response = request(
        &router,
        "POST",
        "/api/v1/imports",
        &token,
        json!({"sessions":[{"source_ids":["source"]}]}),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let batch = response["id"].as_str().unwrap();
    let metadata = |source: &str| {
        [
            ("import_id", batch),
            ("source_id", source),
            ("filename", "prepared.flac"),
            ("original_name", "recording.wav"),
        ]
        .iter()
        .map(|(key, value)| {
            format!(
                "{key} {}",
                base64::engine::general_purpose::STANDARD.encode(value)
            )
        })
        .collect::<Vec<_>>()
        .join(",")
    };
    let unknown = request(
        &router,
        "POST",
        "/api/v1/uploads",
        &token,
        Value::Null,
        Some(metadata("wrong")),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
    let first = request(
        &router,
        "POST",
        "/api/v1/uploads",
        &token,
        Value::Null,
        Some(metadata("source")),
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let location = first.headers()["location"].clone();
    let repeated = request(
        &router,
        "POST",
        "/api/v1/uploads",
        &token,
        Value::Null,
        Some(metadata("source")),
    )
    .await;
    assert_eq!(repeated.status(), StatusCode::CREATED);
    assert_eq!(repeated.headers()["location"], location);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM uploads")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let missing = request(
        &router,
        "POST",
        &format!("/api/v1/imports/{batch}/complete"),
        &token,
        json!({}),
        None,
    )
    .await;
    assert_eq!(missing.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn imported_history_and_identity_controls_preserve_their_distinctions() {
    let (dir, app) = setup().await;
    let audio = dir.path().join("one.wav");
    wav(&audio, 1000);
    add_batch(
        &app,
        "history",
        &[("opaque-source", audio.as_path())],
        &[plan(&["opaque-source"], None, None)],
    )
    .await;
    pipeline::finalize(&app, "history").await.unwrap();
    let entry: String = sqlx::query_scalar("SELECT id FROM entries")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query("UPDATE entries SET stage='complete' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO speakers(entry_id,label)VALUES(?,'0:A')")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO people(id,name)VALUES('person','Synthetic person')")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO voice_probes(entry_id,speaker,model,vector_json)VALUES(?,'0:A',?,'[1,0,0]')",
    )
    .bind(&entry)
    .bind(transcript_vault::voice::MODEL)
    .execute(&app.db)
    .await
    .unwrap();
    let admin = auth::issue(&app, "test owner", "admin").await.unwrap();
    let read = auth::issue(&app, "test reader", "read").await.unwrap();
    let router =
        transcript_vault::api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    let history = request(
        &router,
        "GET",
        "/api/v1/source-history",
        &read,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(history.status(), StatusCode::OK);
    let data: Value =
        serde_json::from_slice(&history.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["entries"][0]["source_ids"], json!(["opaque-source"]));
    assert_eq!(data["entries"][0]["id"], entry);
    let endpoint = format!("/api/v1/entries/{entry}/speakers/0:A");
    let locked = request(
        &router,
        "PATCH",
        &endpoint,
        &admin,
        json!({"person_id":null}),
        None,
    )
    .await;
    assert_eq!(locked.status(), StatusCode::NO_CONTENT);
    let state: String = sqlx::query_scalar("SELECT state FROM speakers WHERE entry_id=?")
        .bind(&entry)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(state, "manual_unknown");
    let reset = request(
        &router,
        "PATCH",
        &endpoint,
        &admin,
        json!({"person_id":null,"locked":false}),
        None,
    )
    .await;
    assert_eq!(reset.status(), StatusCode::NO_CONTENT);
    let state: String = sqlx::query_scalar("SELECT state FROM speakers WHERE entry_id=?")
        .bind(&entry)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(state, "unknown");
    let assigned = request(
        &router,
        "PATCH",
        &endpoint,
        &admin,
        json!({"person_id":"person"}),
        None,
    )
    .await;
    assert_eq!(assigned.status(), StatusCode::NO_CONTENT);
    let samples: i64 =
        sqlx::query_scalar("SELECT count(*) FROM voice_samples WHERE person_id='person'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(samples, 1);
    let removed = request(
        &router,
        "DELETE",
        "/api/v1/people/person/voice",
        &admin,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    let person: i64 = sqlx::query_scalar("SELECT count(*) FROM people WHERE id='person'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let samples: i64 =
        sqlx::query_scalar("SELECT count(*) FROM voice_samples WHERE person_id='person'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((person, samples), (1, 0));
    sqlx::query(
        "INSERT INTO speakers(entry_id,label,state)VALUES(?,'deliberate-unknown','manual_unknown')",
    )
    .bind(&entry)
    .execute(&app.db)
    .await
    .unwrap();
    let deleted = request(
        &router,
        "DELETE",
        "/api/v1/people/person",
        &admin,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let assigned: (Option<String>, String) =
        sqlx::query_as("SELECT person_id,state FROM speakers WHERE entry_id=? AND label='0:A'")
            .bind(&entry)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(assigned, (None, "unknown".into()));
    let deliberate: String = sqlx::query_scalar(
        "SELECT state FROM speakers WHERE entry_id=? AND label='deliberate-unknown'",
    )
    .bind(&entry)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(deliberate, "manual_unknown");
    let rejected = request(
        &router,
        "POST",
        &format!("/api/v1/entries/{entry}/reanalyze"),
        &read,
        json!({}),
        None,
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    // Old API credentials never gain the native owner's OAuth audience.
    let native = request(
        &router,
        "GET",
        "/api/native/v1/source-history",
        &admin,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(native.status(), StatusCode::UNAUTHORIZED);
    let unknown = request(
        &router,
        "GET",
        "/api/native/unknown-route",
        &admin,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED);
    assert!(
        unknown.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
}

#[tokio::test]
async fn trash_is_recoverable_hidden_by_default_and_reimport_restores_it() {
    let (dir, app) = setup().await;
    let audio = dir.path().join("trash.wav");
    wav(&audio, 1000);
    add_batch(
        &app,
        "trash-original",
        &[("old-source", audio.as_path())],
        &[plan(&["old-source"], None, None)],
    )
    .await;
    pipeline::finalize(&app, "trash-original").await.unwrap();
    let entry: String = sqlx::query_scalar("SELECT id FROM entries")
        .fetch_one(&app.db)
        .await
        .unwrap();
    sqlx::query("UPDATE entries SET stage='complete',audio_path='playback.m4a' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    tokio::fs::create_dir_all(app.entry_dir(&entry))
        .await
        .unwrap();
    tokio::fs::write(
        app.entry_dir(&entry).join("playback.m4a"),
        b"synthetic retained audio",
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO parts(entry_id,part,state,raw_json)VALUES(?,0,'complete','{\"segments\":[]}')",
    )
    .bind(&entry)
    .execute(&app.db)
    .await
    .unwrap();
    let admin = auth::issue(&app, "test owner", "admin").await.unwrap();
    let router =
        transcript_vault::api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    let trash = request(
        &router,
        "POST",
        &format!("/api/v1/entries/{entry}/trash"),
        &admin,
        json!({}),
        None,
    )
    .await;
    assert_eq!(trash.status(), StatusCode::NO_CONTENT);
    assert!(
        transcript_vault::retrieval::get(&app, &entry)
            .await
            .is_err()
    );
    let normal = transcript_vault::retrieval::list(&app, Default::default())
        .await
        .unwrap();
    assert!(normal["entries"].as_array().unwrap().is_empty());
    let trash = transcript_vault::retrieval::list(
        &app,
        transcript_vault::retrieval::Filters {
            trash: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(trash["entries"][0]["id"], entry);
    assert!(app.entry_dir(&entry).join("playback.m4a").exists());
    let parts: i64 = sqlx::query_scalar("SELECT count(*) FROM parts WHERE entry_id=?")
        .bind(&entry)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(parts, 1);
    let blocked = request(
        &router,
        "GET",
        &format!("/api/v1/entries/{entry}/audio"),
        &admin,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::NOT_FOUND);
    let history = request(
        &router,
        "GET",
        "/api/v1/source-history",
        &admin,
        Value::Null,
        None,
    )
    .await;
    let history: Value =
        serde_json::from_slice(&history.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(history["entries"].as_array().unwrap().is_empty());
    let restored = request(
        &router,
        "POST",
        &format!("/api/v1/entries/{entry}/restore"),
        &admin,
        json!({}),
        None,
    )
    .await;
    assert_eq!(restored.status(), StatusCode::OK);
    assert!(transcript_vault::retrieval::get(&app, &entry).await.is_ok());
    let _ = request(
        &router,
        "POST",
        &format!("/api/v1/entries/{entry}/trash"),
        &admin,
        json!({}),
        None,
    )
    .await;
    // Reimporting the exact audio restores its prior transcript/checkpoints, including
    // source aliases needed for imported badges on a fresh native client.
    add_batch(
        &app,
        "trash-reimport",
        &[("new-source-alias", audio.as_path())],
        &[plan(&["new-source-alias"], None, None)],
    )
    .await;
    pipeline::finalize(&app, "trash-reimport").await.unwrap();
    assert_eq!(count(&app).await, 1);
    assert!(transcript_vault::retrieval::get(&app, &entry).await.is_ok());
    let history = request(
        &router,
        "GET",
        "/api/v1/source-history",
        &admin,
        Value::Null,
        None,
    )
    .await;
    let history: Value =
        serde_json::from_slice(&history.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(
        history["entries"][0]["source_ids"],
        json!(["new-source-alias"])
    );
}

#[tokio::test]
async fn settings_are_validated_persisted_and_applied_to_enrichment_only() {
    let (_dir, mut app) = setup().await;
    let admin = auth::issue(&app, "test owner", "admin").await.unwrap();
    let read = auth::issue(&app, "test reader", "read").await.unwrap();
    let router =
        transcript_vault::api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    let blocked = request(&router, "GET", "/api/v1/settings", &read, Value::Null, None).await;
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
    let bad = request(
        &router,
        "PATCH",
        "/api/v1/settings",
        &admin,
        json!({"voice_thresholds":{"strong":1.5,"margin":0.1}}),
        None,
    )
    .await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let good=request(&router,"PATCH","/api/v1/settings",&admin,json!({"recording_context":"Synthetic vocabulary context","voice_thresholds":{"strong":0.9,"margin":0.2}}),None).await;
    assert_eq!(good.status(), StatusCode::OK);
    let reloaded = App::new((*app.config).clone()).await.unwrap();
    let saved = transcript_vault::settings::load(&reloaded).await.unwrap();
    assert_eq!(saved.recording_context, "Synthetic vocabulary context");
    assert!((saved.voice_thresholds.strong - 0.9).abs() < 0.00001);
    let captured = Arc::new(tokio::sync::Mutex::new(Value::Null));
    let record = captured.clone();
    let provider=Router::new().route("/responses",post(move |Json(body):Json<Value>| {let record=record.clone();async move{
        *record.lock().await=body;
        Json(json!({"output":[{"content":[{"type":"output_text","text":"{\"title\":\"Fixture\",\"summary\":\"Synthetic\",\"names\":[]}"}]}]}))
    }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
    Arc::make_mut(&mut app.config).openai_url = format!("http://{address}");
    transcript_vault::provider::enrich(&app, "Synthetic transcript")
        .await
        .unwrap();
    let request = captured.lock().await;
    assert!(
        request["instructions"]
            .as_str()
            .unwrap()
            .contains("Synthetic vocabulary context")
    );
    assert!(request.get("prompt").is_none());
    server.abort();
}
