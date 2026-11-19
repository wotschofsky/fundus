//! Local stand-ins for AssemblyAI and OpenAI's chat completions and embeddings. No real provider is called.
#![allow(dead_code)]
use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use fundus::config::App;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default, Clone)]
pub struct Options {
    /// Holds every transcript creation request until notified.
    pub gate: Option<Arc<tokio::sync::Notify>>,
    pub first_embedding_gate: Option<Arc<tokio::sync::Notify>>,
    /// Polls answered "processing" before a job completes.
    pub pending_polls: usize,
    /// The first this many jobs end in AssemblyAI's error state.
    pub failing_jobs: usize,
    /// The first this many creation requests make a job but lose their answer (HTTP 503).
    pub lost_answers: usize,
    /// Holds every chat completion (titles and summaries) until notified.
    pub chat_gate: Option<Arc<tokio::sync::Notify>>,
}

#[derive(Default)]
pub struct Provider {
    pub submits: AtomicUsize,
    pub embeddings: AtomicUsize,
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
    let embedding = state.clone();
    let embedding_gate = options.first_embedding_gate.clone();
    let chat_gate = options.chat_gate.clone();
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
                let (state, gate) = (chat.clone(), chat_gate.clone());
                async move {
                    state.chats.lock().unwrap().push(body);
                    if let Some(gate) = gate {
                        gate.notified().await;
                    }
                    let notes = json!({"language": "English", "title": "Cedar launch planning", "summary": "{A} plans the launch with {B}.",
                        "names": [{"speaker": "[A]", "name": "Morgan", "quote": "I'm Morgan."}]});
                    Json(json!({"choices": [{"message": {"role": "assistant", "content": notes.to_string()}}]}))
                }
            }),
        )
        .route(
            "/embeddings",
            post(move |Json(body): Json<Value>| {
                let state = embedding.clone();
                let gate = embedding_gate.clone();
                async move {
                    if state.embeddings.fetch_add(1, Ordering::SeqCst) == 0
                        && let Some(gate) = gate
                    {
                        gate.notified().await;
                    }
                    let n = body["input"].as_array().map(Vec::len).unwrap_or(1);
                    Json(json!({"data": (0..n).map(|i| json!({"index": i, "embedding": [1., 0., 0.]})).collect::<Vec<_>>()}))
                }
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

/// Cloudflare Access as tests see it. The team's address refuses connections, so no test reads
/// keys from the internet; `signed_in` hands the server the test key instead.
pub const ACCESS_TEAM: &str = "http://127.0.0.1:9";
pub const ACCESS_AUD: &str = "fundus-test-audience";
/// A throwaway RSA key (PKCS#8, base64) made for these tests only; it signs nothing real.
const ACCESS_KEY: &str = concat!(
    "MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQDaTdrhG1wbQVWRY+C4aubpP19RIIG9MLDjy9f/U6pMhB5A",
    "VxqS5cbXzRe4fC9fg7UAamU/+M18YsentoOLTl0i2MeMoTHHME7303REHrr3nMn/Y8fYuDlC/NYaCC6wy15InQnTuDu8g4/Z",
    "GYdWIWSHB8bsJlhUyinGzq1HGp8SmgvSUdUHz4dLMhX+jN7GjCQsKwi5CF2+Cb+76Re5+A+3F0W/7THy0f8kTHbF/o+6/Jpf",
    "aNB2IWHNmFTSvbpVjk09N8br7OoYZUJjs5m8nxfWYkAq7FL2mS4KjrTCxwbrq1aoPy9lR7ZszZFiMs2SgKGSaaE87QnCvhTR",
    "nm8YnEmFAgMBAAECggEAIbpARDct69K+8vozZfVePPswMXXhlZFIQxQ4suys4FV+jFeTdiP6AgK11NLwSW7X2tCVWWTfRTBz",
    "GFOieeC0PbCwY+cjrFqnrgewFuDd52kElxNGH72WsJAk+VtPm2ZQPXfFuj2PPSOLvfGrxIA0P93REKXRT92hcWMiclyH5x+M",
    "kSKSg/cYBin4NYKIQGqj9r1tstZbpXhGGnsaku4jhl6rQpDnIo6RreXCCRGTu04jtw5fyitRKi+F1b+X6LNGWizHSc7Sx9In",
    "F8gnXnO0jxwhc9WrXoI+bkQrQ8jUtbJFRk9jexBWZa/wq8MiLTg/FY58/nyAILQKybwjv+xxoQKBgQD5qJ2tPol8iRC+uVhm",
    "2vNCUA8lzdup5oNih/v6RKv224mVEgS3NfbqVrtt9WuboBgv3nQvp+bMpOyM+n0s0GhA6oROGk1BPpX7tLlSBfyE45D2qci5",
    "KLQACPIxNHOEoG7Qk5p7Db2NxLmO+6Mr7A6t1nnbX3AEB7akvsBn17B99wKBgQDf2VvgJ2vDw+Z0nKYhaUP0gC2S0KSMkrkc",
    "EkCXEbKMtXjiEXO6xKrVySGFTgWVdNpT2oGRjfcCYK5XltnLRVrLuuxWvT4eiS75kleuOsVsHCEq5VmO6ULn3/g8bWNocPNu",
    "VWKWAfPWygiFk7RtqL5d6U4qiOdhoSsmgWYDk9hFYwKBgQDQ8I0FsJDf6NZayENz0UpE9Tu4J9DvIDxU+fmoOlCxCIrcd3f6",
    "3O8IsA2suPiWBiY36bTTLXG7U0ulsP2Rw9l8esi/lMOTFw3u2hE+1EPB87IevXl/fNEoNlR7WapUFPCpHt7qnVVGkJ27TF4D",
    "JEwksJrLPdRBizMQLZRZU+ohgQKBgQCDAJ/GNQvaS8GJKUwIbaR5aLJDI5vJMOzoQr5OKutFccbG9tO1z/bymnzlmN2ns/U5",
    "8by2RFD0sUQWuo/HaFVUuZYqxSGfmUVGhaCOtJJpTKUUh5i8DEM9lFOgeZwduUydm5bQl1/uqdoG4oRLUN/n8evu9KQE/N9V",
    "75CWM2X0ZQKBgQC6OKse5Xt8gO0woRguATV8/2UiscpadfhZxxK/L3xqakVDJONOqGQIOTOENwtqZXu7KUjsMJioZfDGQRxT",
    "Zb3x7bzseQ3xokts3CGUhv8hhh8RxKYVtjAwRgipTS9Btdcga+46ym3eIKhr00s7FLjLq3CTubHUhT2M3DniJ3gGlg==",
);

/// Proxy mode behind Cloudflare Access, as `VAULT_AUTH_MODE=proxy` sets it up.
pub fn proxy(config: &mut fundus::config::Config) {
    config.proxy_auth = true;
    config.access = Some(fundus::access::Settings::new(ACCESS_TEAM, ACCESS_AUD).unwrap());
}
fn access_key() -> ring::signature::RsaKeyPair {
    use base64::Engine;
    let der = base64::engine::general_purpose::STANDARD
        .decode(ACCESS_KEY)
        .unwrap();
    ring::signature::RsaKeyPair::from_pkcs8(&der).unwrap()
}
/// The test key as Access lists its signing keys (`/cdn-cgi/access/certs`).
pub fn access_jwks() -> Value {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let public = ring::rsa::PublicKeyComponents::<Vec<u8>>::from(access_key().public());
    json!({"keys": [{
        "kid": "test", "kty": "RSA", "alg": "RS256", "use": "sig",
        "n": URL_SAFE_NO_PAD.encode(public.n),
        "e": URL_SAFE_NO_PAD.encode(public.e),
    }]})
}
/// A token with `header` and `claims`, signed with the test key (RS256).
pub fn access_token_with(header: Value, claims: Value) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let signed = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let key = access_key();
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        signed.as_bytes(),
        &mut signature,
    )
    .unwrap();
    format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature))
}
/// What Access puts in the owner's token: their sign-in to this application, valid for a while.
pub fn owner_claims() -> Value {
    let now = chrono::Utc::now().timestamp();
    json!({
        "iss": ACCESS_TEAM, "aud": [ACCESS_AUD], "email": "owner@example.com", "type": "app",
        "iat": now, "nbf": now, "exp": now + 600, "sub": "owner",
    })
}
pub fn access_token(claims: Value) -> String {
    access_token_with(json!({"alg": "RS256", "kid": "test", "typ": "JWT"}), claims)
}
/// Every request through `router` comes signed in by Access as the owner, as behind the proxy,
/// unless it carries its own token.
pub fn signed_in(app: &App, router: Router) -> Router {
    let (app, token) = (app.clone(), access_token(owner_claims()));
    let jwks = access_jwks().to_string();
    router.layer(axum::middleware::map_request(
        move |mut request: axum::extract::Request| {
            let (app, token, jwks) = (app.clone(), token.clone(), jwks.clone());
            async move {
                app.access_keys.hold(jwks.as_bytes()).await.unwrap();
                request
                    .headers_mut()
                    .entry(fundus::access::HEADER)
                    .or_insert(token.parse().unwrap());
                request
            }
        },
    ))
}
/// `router` as the owner's browser uses it in password mode: every request carries a fresh session
/// cookie and comes from the vault's own pages, unless it sets its own.
pub async fn with_session(app: &App, router: Router) -> Router {
    let cookie = format!(
        "vault_session={}",
        fundus::auth::session(app).await.unwrap()
    );
    let origin = app.config.public_url.origin().ascii_serialization();
    router.layer(axum::middleware::map_request(
        move |mut request: axum::extract::Request| {
            let (cookie, origin) = (cookie.clone(), origin.clone());
            async move {
                let headers = request.headers_mut();
                headers
                    .entry(axum::http::header::COOKIE)
                    .or_insert(cookie.parse().unwrap());
                headers
                    .entry(axum::http::header::ORIGIN)
                    .or_insert(origin.parse().unwrap());
                request
            }
        },
    ))
}
/// An OAuth access token for `scope`: "mcp:read" for MCP, "vault:owner" for the native API. As if
/// the owner had approved a client in the browser; `tests/oauth.rs` runs that flow itself.
pub async fn grant(app: &App, scope: &str) -> String {
    let resource = if scope == "vault:owner" {
        fundus::oauth::native_resource(app)
    } else {
        fundus::oauth::resource(app)
    };
    let (client, grant) = (fundus::auth::secret(), fundus::auth::secret());
    sqlx::query("INSERT INTO oauth_clients(id,name,redirect_uris,auth_method,created_at,allowed_scope) VALUES(?,'Test client','[]','none',unixepoch(),?)")
        .bind(&client)
        .bind(scope)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO oauth_grants(id,client_id,scope,resource,created_at,expires_at) VALUES(?,?,?,?,unixepoch(),unixepoch() + 3600)")
        .bind(&grant)
        .bind(&client)
        .bind(scope)
        .bind(resource)
        .execute(&app.db)
        .await
        .unwrap();
    let token = format!("oa_{}", fundus::auth::secret());
    sqlx::query(
        "INSERT INTO oauth_access(hash,grant_id,expires_at) VALUES(?,?,unixepoch() + 3600)",
    )
    .bind(fundus::auth::hash(&token))
    .bind(&grant)
    .execute(&app.db)
    .await
    .unwrap();
    token
}
/// A row the removed static tokens left in `credentials` (`scope`: "upload", "read" or "admin"),
/// as before the migration that deletes them; its value. Tokens never expired; this one expires
/// in a day, so only its scope keeps it from passing for a browser session.
pub async fn leftover_token(app: &App, scope: &str) -> String {
    let value = fundus::auth::secret();
    sqlx::query("INSERT INTO credentials(hash,name,scope,created_at,expires_at) VALUES(?,'Leftover',?,'2026-01-01T00:00:00.000Z',?)")
        .bind(fundus::auth::hash(&value))
        .bind(scope)
        .bind(
            (chrono::Utc::now() + chrono::Duration::days(1))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )
        .execute(&app.db)
        .await
        .unwrap();
    value
}
