//! Local stand-ins for AssemblyAI and OpenAI's chat completions and embeddings. No real provider is called.
#![allow(dead_code)]
use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use transcript_vault::config::App;

#[derive(Default, Clone)]
pub struct Options {
    /// Holds every transcript creation request until notified.
    pub gate: Option<Arc<tokio::sync::Notify>>,
    /// Polls answered "processing" before a job completes.
    pub pending_polls: usize,
    /// The first this many jobs end in AssemblyAI's error state.
    pub failing_jobs: usize,
    /// The first this many creation requests make a job but lose their answer (HTTP 503).
    pub lost_answers: usize,
}

#[derive(Default)]
pub struct Provider {
    pub submits: AtomicUsize,
    pub polls: AtomicUsize,
    pub uploads: AtomicUsize,
    pub deletes: AtomicUsize,
    /// Every job created, with the audio URL it reads.
    pub jobs: Mutex<Vec<(String, String)>>,
    /// Bodies of transcript creation requests.
    pub transcripts: Mutex<Vec<Value>>,
    /// Bodies of chat completion requests.
    pub chats: Mutex<Vec<Value>>,
}
impl Provider {
    pub fn submits(&self) -> usize {
        self.submits.load(Ordering::SeqCst)
    }
    pub fn deletes(&self) -> usize {
        self.deletes.load(Ordering::SeqCst)
    }
}

pub const UTTERANCES: &str = r#"[
 {"speaker":"A","start":0,"end":5000,"text":"I'm Morgan. The cedar launch is on Tuesday.","confidence":0.9,"words":[]},
 {"speaker":"B","start":5200,"end":8000,"text":"Great, I will tell the team.","confidence":0.9,"words":[]},
 {"speaker":"A","start":8200,"end":12000,"text":"We agreed to send the proposal tomorrow.","confidence":0.9,"words":[]}
]"#;

/// Starts the mock and points `app` at it. Returns the shared request counters.
pub async fn connect(
    app: &mut App,
    options: Options,
) -> (Arc<Provider>, tokio::task::JoinHandle<()>) {
    let state = Arc::new(Provider::default());
    let (submit, poll, upload, chat) = (state.clone(), state.clone(), state.clone(), state.clone());
    let (list, delete) = (state.clone(), state.clone());
    let gate = options.gate.clone();
    let router = Router::new()
        .route(
            "/v2/upload",
            post(move |body: axum::body::Bytes| async move {
                assert!(!body.is_empty(), "upload must carry the audio");
                let n = upload.uploads.fetch_add(1, Ordering::SeqCst);
                Json(json!({"upload_url": format!("https://cdn.example/upload-{n}")}))
            }),
        )
        .route(
            "/v2/transcript",
            post(move |Json(body): Json<Value>| {
                let (state, gate) = (submit.clone(), gate.clone());
                async move {
                    let n = state.submits.fetch_add(1, Ordering::SeqCst);
                    let job = format!("job-{n}");
                    let audio = body["audio_url"].as_str().unwrap_or_default().to_string();
                    state.jobs.lock().unwrap().push((job.clone(), audio));
                    state.transcripts.lock().unwrap().push(body);
                    if let Some(gate) = gate {
                        gate.notified().await;
                    }
                    if n < options.lost_answers {
                        return StatusCode::SERVICE_UNAVAILABLE.into_response();
                    }
                    Json(json!({"id": job, "status": "queued"})).into_response()
                }
            })
            .get(move || {
                let state = list.clone();
                async move {
                    let jobs = state.jobs.lock().unwrap().clone();
                    Json(json!({"transcripts": jobs.iter().rev().map(|(id, audio)| json!({"id": id, "status": "processing", "audio_url": audio})).collect::<Vec<_>>()}))
                }
            }),
        )
        .route(
            "/v2/transcript/{id}",
            get(move |Path(id): Path<String>| {
                let state = poll.clone();
                async move {
                    let n = state.polls.fetch_add(1, Ordering::SeqCst);
                    let Ok(job) = id.trim_start_matches("job-").parse::<usize>() else {
                        return (StatusCode::NOT_FOUND, Json(json!({"error": "Transcript not found"}))).into_response();
                    };
                    Json(if job < options.failing_jobs {
                        json!({"id": id, "status": "error", "error": "synthetic failure"})
                    } else if n % (options.pending_polls + 1) < options.pending_polls {
                        json!({"id": id, "status": "processing"})
                    } else {
                        json!({"id": id, "status": "completed", "utterances": serde_json::from_str::<Value>(UTTERANCES).unwrap(), "words": []})
                    }).into_response()
                }
            })
            .delete(move || {
                let state = delete.clone();
                async move {
                    state.deletes.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"status": "completed"}))
                }
            }),
        )
        .route(
            "/chat/completions",
            post(move |Json(body): Json<Value>| {
                let state = chat.clone();
                async move {
                    state.chats.lock().unwrap().push(body);
                    let notes = json!({"language": "English", "title": "Cedar launch planning", "summary": "{A} plans the launch with {B}.",
                        "names": [{"speaker": "[A]", "name": "Morgan", "quote": "I'm Morgan."}]});
                    Json(json!({"choices": [{"message": {"role": "assistant", "content": notes.to_string()}}]}))
                }
            }),
        )
        .route(
            "/embeddings",
            post(|Json(body): Json<Value>| async move {
                let n = body["input"].as_array().map(Vec::len).unwrap_or(1);
                Json(json!({"data": (0..n).map(|i| json!({"index": i, "embedding": [1., 0., 0.]})).collect::<Vec<_>>()}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let config = Arc::make_mut(&mut app.config);
    config.assemblyai_url = address.clone();
    config.assemblyai_key = "synthetic-only".into();
    config.openai_url = address;
    config.openai_key = "synthetic-only".into();
    config.poll_interval = std::time::Duration::from_millis(10);
    (state, task)
}

/// Waits until `check` holds, or fails the test after ten seconds.
pub async fn until(mut check: impl FnMut() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !check() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("condition not reached in time");
}
