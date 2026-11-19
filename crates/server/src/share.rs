//! Share links: one recording, read-only, for whoever holds its link (`/s/{token}`).
//!
//! The owner's routes sit in the library (`api::library_routes`) behind owner access. The
//! recipient's routes are public, nested at `/s`. The page itself is the web app; everything it
//! reads (the recording, transcript, audio and video) looks the token up on every request, seeks
//! included: a link stops working at the next request once it expires, the owner stops sharing,
//! or the recording goes to Trash (which deletes the link). A recipient gets what the share page
//! shows (player, summary, transcript), never the recording's id, labels, sources or speaker
//! evidence.
use crate::{
    api,
    auth::{self, Access},
    config::{App, now},
    error::{Error, Result},
    retrieval,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::{MethodRouter, get},
};
use serde::Deserialize;
use serde_json::{Value, json};

/// The length of `auth::secret()`: 32 random bytes, base64url without padding.
const TOKEN_LEN: usize = 43;

fn url(app: &App, token: &str) -> String {
    // The public URL is an origin, so its path is "/".
    format!("{}s/{token}", app.config.public_url)
}

/// The recording's link while it works, or null.
pub async fn show(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    a.admin()?;
    let now = now();
    let row = sqlx::query!(
        "SELECT token,created_at,expires_at FROM shares WHERE entry_id=? AND (expires_at IS NULL OR expires_at>?)",
        id,
        now
    )
    .fetch_optional(&app.db)
    .await?;
    Ok(Json(row.map_or(
        Value::Null,
        |r| json!({"url":url(&app,&r.token),"created_at":r.created_at,"expires_at":r.expires_at}),
    )))
}

#[derive(Deserialize)]
pub struct Expiry {
    expires: String,
}

/// When a link set to `expires` from now stops working (None: never).
fn expiry(expires: &str) -> Result<Option<String>> {
    let days = match expires {
        "1d" => Some(1),
        "7d" => Some(7),
        "30d" => Some(30),
        "never" => None,
        _ => return Err(Error::bad("Choose 1d, 7d, 30d or never")),
    };
    Ok(days.map(|days| {
        (chrono::Utc::now() + chrono::Duration::days(days))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }))
}

/// Shares the recording. A new link works for `expires` from now. A link that still works is
/// returned as it is, its expiry too (only `extend` changes that), so a dialog that had not yet
/// seen a link made elsewhere cannot shorten or lengthen it; an expired one is replaced by a new
/// URL, so whoever had the old one does not get it back.
pub async fn save(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
    Json(p): Json<Expiry>,
) -> Result<Json<Value>> {
    a.admin()?;
    let expires_at = expiry(&p.expires)?;
    let now = now();
    let token = auth::secret();
    // One statement that also returns what it wrote, so Stop sharing and Trash (which deletes the
    // link) cannot land between a read and this write and be undone by it, nor change the answer.
    // It reads the clock itself, once it may write: a link that expired while this waited for
    // another writer gets a new URL too.
    let saved = sqlx::query!(
        "INSERT INTO shares(entry_id,token,created_at,expires_at) SELECT id,?2,?3,?4 FROM entries WHERE id=?1 AND trashed_at IS NULL
         ON CONFLICT(entry_id) DO UPDATE SET
           expires_at=CASE WHEN shares.expires_at IS NULL OR shares.expires_at>strftime('%Y-%m-%dT%H:%M:%fZ','now') THEN shares.expires_at ELSE excluded.expires_at END,
           token=CASE WHEN shares.expires_at IS NULL OR shares.expires_at>strftime('%Y-%m-%dT%H:%M:%fZ','now') THEN shares.token ELSE excluded.token END,
           created_at=CASE WHEN shares.expires_at IS NULL OR shares.expires_at>strftime('%Y-%m-%dT%H:%M:%fZ','now') THEN shares.created_at ELSE excluded.created_at END
         RETURNING token AS \"token!\",created_at AS \"created_at!\",expires_at",
        id,
        token,
        now,
        expires_at
    )
    // Read to the end, not just the first row: the change commits after the row is returned, and
    // a commit that fails must not be answered as saved.
    .fetch_all(&app.db)
    .await?
    .pop();
    let Some(r) = saved else {
        retrieval::get_including_trash(&app, &id).await?;
        return Err(Error::conflict(
            "Restore the recording from Trash before sharing it",
        ));
    };
    Ok(Json(
        json!({"url":url(&app,&r.token),"created_at":r.created_at,"expires_at":r.expires_at}),
    ))
}

#[derive(Deserialize)]
pub struct Change {
    expires: String,
    token: String,
}

/// Sets how long the named link keeps working, from now, keeping its URL. It never makes a link,
/// and touches no other: a link that expired, or that Stop sharing removed (perhaps from another
/// tab while this was on its way), stays gone, a link that replaced it stays as it is, and the
/// owner hears 404.
pub async fn extend(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
    Json(p): Json<Change>,
) -> Result<Json<Value>> {
    a.admin()?;
    let expires_at = expiry(&p.expires)?;
    // Trash deletes the link, so a row here is a recording outside Trash.
    let r = sqlx::query!(
        "UPDATE shares SET expires_at=?3 WHERE entry_id=?1 AND token=?2 AND (expires_at IS NULL OR expires_at>strftime('%Y-%m-%dT%H:%M:%fZ','now'))
         RETURNING token AS \"token!\",created_at AS \"created_at!\",expires_at",
        id,
        p.token,
        expires_at
    )
    // To the end, as in `save`, so a failed commit is not answered as saved.
    .fetch_all(&app.db)
    .await?
    .pop()
    .ok_or_else(Error::not_found)?;
    Ok(Json(
        json!({"url":url(&app,&r.token),"created_at":r.created_at,"expires_at":r.expires_at}),
    ))
}

#[derive(Deserialize)]
pub struct Named {
    token: String,
}

/// Stop sharing the named link: it stops working at its next request. A link that replaced it
/// meanwhile stays; the owner hears 404, as for a link already gone.
pub async fn stop(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
    Query(p): Query<Named>,
) -> Result<StatusCode> {
    a.admin()?;
    let stopped = sqlx::query!(
        "DELETE FROM shares WHERE entry_id=? AND token=?",
        id,
        p.token
    )
    .execute(&app.db)
    .await?
    .rows_affected();
    if stopped == 0 {
        return Err(Error::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Every link that works, newest first.
pub async fn list(State(app): State<App>, Extension(a): Extension<Access>) -> Result<Json<Value>> {
    a.admin()?;
    let now = now();
    let rows = sqlx::query!(
        "SELECT s.entry_id,s.token,s.created_at,s.expires_at,e.title FROM shares s JOIN entries e ON e.id=s.entry_id WHERE (s.expires_at IS NULL OR s.expires_at>?) AND e.trashed_at IS NULL ORDER BY s.created_at DESC",
        now
    )
    .fetch_all(&app.db)
    .await?;
    Ok(Json(json!({"shares":rows.iter().map(|r| json!({
        "entry_id":r.entry_id,"title":r.title,"url":url(&app,&r.token),
        "created_at":r.created_at,"expires_at":r.expires_at,
    })).collect::<Vec<_>>()})))
}

/// The recipient's routes, nested at `/s` with `page` (the web app) at `/s/{token}`. Anything
/// else under a link is a 404 here rather than the web app, with the same headers; a bare `/s/`
/// is the app's own 404.
pub fn public(page: MethodRouter<App>) -> Router<App> {
    Router::new()
        .route("/{token}", page)
        .route("/{token}/entry", get(entry))
        .route("/{token}/audio", get(audio).head(audio))
        .route("/{token}/video", get(video).head(video))
        .fallback(|| async { Error::not_found() })
        .layer(middleware::from_fn(headers))
}

/// No cache keeps an owner's answer that holds a link: its token is the secret, and a kept
/// answer would hand the link out again after it was stopped.
pub async fn no_store(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Neither caches nor search engines keep what a link reads, its 404s included.
async fn headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert("x-robots-tag", HeaderValue::from_static("noindex"));
    response
}

/// The recording a link opens while it works. A link that never existed, has expired, or whose
/// recording is in Trash: the same 404.
async fn shared(app: &App, token: &str) -> Result<String> {
    if token.len() != TOKEN_LEN {
        return Err(Error::not_found());
    }
    let now = now();
    sqlx::query_scalar!(
        "SELECT s.entry_id AS \"entry_id!\" FROM shares s JOIN entries e ON e.id=s.entry_id WHERE s.token=? AND (s.expires_at IS NULL OR s.expires_at>?) AND e.trashed_at IS NULL",
        token,
        now
    )
    .fetch_optional(&app.db)
    .await?
    .ok_or_else(Error::not_found)
}

#[derive(Deserialize)]
struct EntryQuery {
    /// The `transcript_key` of the segments the reader holds: unchanged, they are not sent again.
    transcript_key: Option<String>,
}

/// The recording and its whole transcript in one read, so neither waits for the other.
async fn entry(
    State(app): State<App>,
    Path(token): Path<String>,
    Query(q): Query<EntryQuery>,
) -> Result<Json<Value>> {
    let id = shared(&app, &token).await?;
    let e = retrieval::get(&app, &id).await?;
    let segments = retrieval::segments(&app, &id).await?;
    // Copied field by field: what else the owner's entry holds stays with the owner.
    let speakers: Vec<Value> = e["speakers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| json!({"label":s["label"],"name":s["name"]}))
        .collect();
    let video = e["video"]
        .as_object()
        .map(|v| json!({"state":v["state"],"width":v["width"],"height":v["height"]}));
    // Positions stand in for segment ids, which contain the recording's id.
    let segments: Vec<Value> = segments
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            json!({"id":s["position"].to_string(),"position":s["position"],"speaker":s["speaker"],
                "speaker_name":s["speaker_name"],"start_ms":s["start_ms"],"end_ms":s["end_ms"],"text":s["text"]})
        })
        .collect();
    let mut v = json!({
        "title":e["title"],"recorded_at":e["recorded_at"],"duration_ms":e["duration_ms"],
        "stage":e["stage"],"summary":e["summary"],"summary_manual":e["summary_manual"],
        "audio_available":e["audio_available"],"video":video,"speakers":speakers,
    });
    retrieval::attach(&mut v, json!(segments), q.transcript_key.as_deref());
    Ok(Json(v))
}

async fn audio(
    State(app): State<App>,
    Path(token): Path<String>,
    request: Request,
) -> Result<Response> {
    let id = shared(&app, &token).await?;
    api::serve_media(&app, &id, request, false).await
}

async fn video(
    State(app): State<App>,
    Path(token): Path<String>,
    request: Request,
) -> Result<Response> {
    let id = shared(&app, &token).await?;
    api::serve_media(&app, &id, request, true).await
}
