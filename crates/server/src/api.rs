use crate::{
    auth::{self, Access},
    config::{App, id, now},
    error::{Error, Result},
    mcp::VaultMcp,
    oauth, retrieval, settings, uploads,
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tower::ServiceExt;
use tower_http::services::ServeFile;

pub fn router(app: App, shutdown: tokio_util::sync::CancellationToken) -> Router {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    let mcp_app = app.clone();
    let origin = app.config.public_url.origin().ascii_serialization();
    let service = StreamableHttpService::new(
        move || Ok(VaultMcp::new(mcp_app.clone())),
        std::sync::Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_allowed_origins([origin])
            .with_allowed_hosts([
                app.config.public_url.host_str().unwrap_or("localhost"),
                "localhost",
                "127.0.0.1",
            ])
            .with_cancellation_token(shutdown),
    );
    let mcp = Router::new()
        .nest_service("/mcp", service)
        .route_layer(middleware::from_fn(read_only))
        .route_layer(middleware::from_fn_with_state(
            app.clone(),
            oauth::require_mcp,
        ));
    let transfers = transfer_routes("/api/v1")
        .route_layer(middleware::from_fn_with_state(app.clone(), auth::require));
    let protected = library_routes("/api/v1")
        .route(
            "/api/v1/oauth/authorize",
            get(oauth::authorization).post(oauth::consent),
        )
        .route("/api/v1/oauth/connections", get(oauth::connections))
        .route(
            "/api/v1/oauth/connections/{id}",
            axum::routing::delete(oauth::disconnect),
        )
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/tokens", get(tokens).post(create_token))
        .route("/api/v1/tokens/{id}", axum::routing::delete(revoke))
        .route_layer(middleware::from_fn_with_state(
            app.clone(),
            auth::require_browser,
        ));
    // Installed clients use an OAuth-only namespace. Proxy trust and legacy
    // upload/read credentials never authenticate these owner-capability routes.
    let native = library_routes("/v1")
        .merge(transfer_routes("/v1"))
        .fallback(|| async { Error::not_found() })
        .layer(middleware::from_fn_with_state(
            app.clone(),
            oauth::require_native,
        ));
    let root = app.config.web.clone();
    let web = std::sync::Arc::new(web_files(&root));
    Router::new()
        .route("/healthz", get(|| async { Json(json!({"ok":true})) }))
        .route("/api/login", post(auth::login))
        .merge(oauth::public_router())
        .merge(protected)
        .merge(transfers)
        .merge(mcp)
        .nest("/api/native", native)
        .fallback(move |request: Request| {
            // Debug builds pick up a fresh `pnpm build` without a restart.
            let web = if cfg!(debug_assertions) {
                std::sync::Arc::new(web_files(&root))
            } else {
                web.clone()
            };
            static_file(web, request)
        })
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
        .layer(middleware::from_fn(security_headers))
        .with_state(app)
}
/// The library and its management, shared by the browser and the native namespace.
fn library_routes(prefix: &str) -> Router<App> {
    let at = |path: &str| format!("{prefix}{path}");
    Router::new()
        .route(&at("/me"), get(me))
        .route(&at("/settings"), get(settings::get).patch(settings::update))
        .route(&at("/entries"), get(entries))
        .route(
            &at("/entries/{id}"),
            get(entry).patch(update_entry).delete(delete_entry),
        )
        .route(&at("/entries/{id}/transcript"), get(read))
        .route(&at("/entries/{id}/audio"), get(audio_file).head(audio_file))
        .route(&at("/entries/{id}/trash"), post(trash_entry))
        .route(&at("/entries/{id}/restore"), post(restore_entry))
        .route(&at("/entries/{id}/retry"), post(retry))
        .route(&at("/entries/{id}/pause"), post(pause))
        .route(&at("/entries/{id}/reanalyze"), post(reanalyze))
        .route(&at("/entries/{id}/summarize"), post(summarize))
        .route(&at("/entries/{id}/speakers/{label}"), patch(assign_speaker))
        .route(&at("/source-history"), get(source_history))
        .route(&at("/people"), get(people).post(create_person))
        .route(
            &at("/people/{id}"),
            patch(rename_person).delete(delete_person),
        )
        .route(
            &at("/people/{id}/voice"),
            axum::routing::delete(remove_voice),
        )
        .route(&at("/labels"), get(labels))
        .route(&at("/search"), get(search))
}
/// Resumable uploads and the imports they belong to.
fn transfer_routes(prefix: &str) -> Router<App> {
    let at = |path: &str| format!("{prefix}{path}");
    Router::new()
        .route(&at("/imports"), post(uploads::create_import))
        .route(&at("/imports/{id}"), get(uploads::status))
        .route(&at("/imports/{id}/complete"), post(uploads::complete))
        .route(
            &at("/uploads"),
            post(uploads::create).options(uploads::options),
        )
        .route(
            &at("/uploads/{id}"),
            axum::routing::head(uploads::head)
                .patch(uploads::patch)
                .delete(uploads::terminate)
                .options(uploads::options),
        )
}
async fn read_only(Extension(a): Extension<Access>, r: Request, n: middleware::Next) -> Response {
    if let Err(e) = a.read() {
        return e.into_response();
    }
    n.run(r).await
}
async fn security_headers(request: Request, next: middleware::Next) -> Response {
    let mut r = next.run(request).await;
    let h = r.headers_mut();
    for (k, v) in [
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "same-origin"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()",
        ),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; media-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'",
        ),
    ] {
        h.insert(k, v.parse().unwrap());
    }
    r
}
async fn me(State(app): State<App>, Extension(a): Extension<Access>) -> Json<Value> {
    Json(
        json!({"scope":a.scope,"auth_mode":if app.config.proxy_auth {"proxy"} else {"password"},"recorder_timezone":app.config.recorder_timezone,"voice_configured":app.config.voice_model.is_some(),"public_url":app.config.public_url.as_str()}),
    )
}
async fn logout(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    h: HeaderMap,
) -> Result<Response> {
    a.admin()?;
    if let Some(token) = h
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split(';')
                .find_map(|s| s.trim().strip_prefix("vault_session="))
        })
    {
        let token_hash = auth::hash(token);
        sqlx::query!(
            "DELETE FROM credentials WHERE hash=? AND scope='session'",
            token_hash
        )
        .execute(&app.db)
        .await?;
    }
    Ok((
        [(
            header::SET_COOKIE,
            "vault_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
        )],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}
#[derive(Deserialize)]
struct TokenRequest {
    name: String,
    scope: String,
}
async fn create_token(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Json(p): Json<TokenRequest>,
) -> Result<Json<Value>> {
    a.admin()?;
    if !["upload", "read"].contains(&p.scope.as_str())
        || p.name.trim().is_empty()
        || p.name.len() > 100
    {
        return Err(Error::bad("Choose a name and an upload or read scope"));
    }
    Ok(Json(
        json!({"token":auth::issue(&app,p.name.trim(),&p.scope).await?}),
    ))
}
async fn tokens(State(app): State<App>, Extension(a): Extension<Access>) -> Result<Json<Value>> {
    a.admin()?;
    let rows=sqlx::query!("SELECT hash AS \"hash!\",name,scope,created_at,last_used_at FROM credentials WHERE scope!='session' ORDER BY created_at DESC").fetch_all(&app.db).await?;
    Ok(Json(
        json!({"tokens":rows.iter().map(|r|json!({"id":r.hash.clone(),"name":r.name.clone(),"scope":r.scope.clone(),"created_at":r.created_at.clone(),"last_used_at":r.last_used_at.clone()})).collect::<Vec<_>>()}),
    ))
}
async fn revoke(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    a.admin()?;
    sqlx::query!(
        "DELETE FROM credentials WHERE hash=? AND scope!='session'",
        id
    )
    .execute(&app.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn entries(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Query(f): Query<retrieval::Filters>,
) -> Result<Json<Value>> {
    a.read()?;
    if f.trash == Some(true) {
        a.admin()?;
    }
    Ok(Json(retrieval::list(&app, f).await?))
}
async fn entry(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    a.read()?;
    Ok(Json(retrieval::get(&app, &id).await?))
}
#[derive(Deserialize, Default)]
struct ReadQuery {
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    cursor: Option<u32>,
    limit: Option<u32>,
}
async fn read(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
    Query(p): Query<ReadQuery>,
) -> Result<Json<Value>> {
    a.read()?;
    Ok(Json(
        retrieval::transcript(
            &app,
            retrieval::Read {
                entry_id: id,
                from_ms: p.from_ms,
                to_ms: p.to_ms,
                cursor: p.cursor,
                limit: p.limit,
            },
            2000,
        )
        .await?,
    ))
}
async fn labels(State(app): State<App>, Extension(a): Extension<Access>) -> Result<Json<Value>> {
    a.read()?;
    Ok(Json(retrieval::labels(&app).await?))
}
#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    limit: Option<u32>,
    cursor: Option<u32>,
    #[serde(flatten)]
    filters: retrieval::Filters,
}
async fn search(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Query(p): Query<SearchQuery>,
) -> Result<Json<Value>> {
    a.read()?;
    if p.filters.trash == Some(true) {
        a.admin()?;
    }
    Ok(Json(
        retrieval::search(
            &app,
            retrieval::Search {
                query: p.q,
                filters: retrieval::Filters {
                    limit: p.limit,
                    cursor: p.cursor,
                    ..p.filters
                },
            },
        )
        .await?,
    ))
}
async fn audio_file(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
    r: Request,
) -> Result<Response> {
    a.read()?;
    let exists: Option<String> = sqlx::query_scalar!(
        "SELECT audio_path FROM entries WHERE id=? AND trashed_at IS NULL",
        id
    )
    .fetch_optional(&app.db)
    .await?
    .flatten();
    if exists.is_none() {
        return Err(Error::not_found());
    }
    let response = ServeFile::new(app.entry_dir(&id).join("playback.m4a"))
        .oneshot(r)
        .await
        .unwrap();
    Ok(response.into_response())
}
#[derive(Deserialize)]
struct EditEntry {
    title: Option<String>,
    labels: Option<Vec<String>>,
    recorded_at: Option<String>,
}
async fn update_entry(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
    Json(p): Json<EditEntry>,
) -> Result<Json<Value>> {
    a.admin()?;
    retrieval::get(&app, &id).await?;
    let mut tx = app.db.begin().await?;
    if let Some(title) = p.title {
        let title = title.trim();
        if title.is_empty() || title.len() > 240 {
            return Err(Error::bad("Title must be 1–240 characters"));
        }
        let timestamp = now();
        sqlx::query!(
            "UPDATE entries SET title=?,title_manual=1,updated_at=? WHERE id=?",
            title,
            timestamp,
            id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("UPDATE passage_fts SET title=? WHERE entry_id=?", title, id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(labels) = p.labels {
        if labels.len() > 20 {
            return Err(Error::bad("Use at most 20 labels"));
        }
        sqlx::query!("DELETE FROM entry_labels WHERE entry_id=?", id)
            .execute(&mut *tx)
            .await?;
        for label in labels {
            let label = label.trim();
            if label.is_empty() || label.len() > 64 || label.contains(',') {
                return Err(Error::bad("Labels must be 1–64 characters without commas"));
            }
            sqlx::query!("INSERT OR IGNORE INTO labels(name)VALUES(?)", label)
                .execute(&mut *tx)
                .await?;
            sqlx::query!(
                "INSERT OR IGNORE INTO entry_labels(entry_id,label)VALUES(?,?)",
                id,
                label
            )
            .execute(&mut *tx)
            .await?;
        }
    }
    if let Some(time) = p.recorded_at {
        let time = chrono::DateTime::parse_from_rfc3339(&time)
            .map_err(|_| Error::bad("Recording time must include a UTC offset"))?
            .with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        sqlx::query!("UPDATE entries SET recorded_at=? WHERE id=?", time, id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(retrieval::get(&app, &id).await?))
}
async fn retry(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    a.admin()?;
    let _lock = app.mutations.lock().await;
    let state: Option<String> = sqlx::query_scalar!(
        "SELECT stage FROM entries WHERE id=? AND trashed_at IS NULL",
        id
    )
    .fetch_optional(&app.db)
    .await?;
    let state = state.ok_or_else(Error::not_found)?;
    if !["failed", "complete", "paused"].contains(&state.as_str()) {
        return Err(Error::conflict("Recording is already processing"));
    }
    let mut tx = app.db.begin().await?;
    // Retry is the owner's consent to pay again only for a request saved without its upload URL;
    // a saved job ID is polled and a saved upload URL finds its job, never resubmitting.
    sqlx::query!(
        "DELETE FROM transcriptions WHERE entry_id=? AND remote_id IS NULL AND audio_url IS NULL",
        id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("UPDATE entries SET stage='queued',failure=NULL,pause_requested=0,requested_work=CASE WHEN stage='complete' THEN NULL ELSE requested_work END WHERE id=?", id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    app.wake.notify_one();
    Ok(Json(json!({"stage":"queued"})))
}
async fn pause(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    access.admin()?;
    let _lock = app.mutations.lock().await;
    let stage: Option<String> = sqlx::query_scalar!(
        "SELECT stage FROM entries WHERE id=? AND trashed_at IS NULL",
        id
    )
    .fetch_optional(&app.db)
    .await?;
    let stage = stage.ok_or_else(Error::not_found)?;
    if stage == "paused" {
        return Ok(Json(json!({"stage":"paused","pause_requested":false})));
    }
    if ["complete", "failed"].contains(&stage.as_str()) {
        return Err(Error::conflict("Recording is not processing"));
    }
    let timestamp = now();
    let updated = sqlx::query!("UPDATE entries SET pause_requested=1,stage=CASE WHEN stage='queued' THEN 'paused' ELSE stage END,updated_at=? WHERE id=? AND stage NOT IN ('complete','failed','paused') RETURNING stage", timestamp, id).fetch_optional(&app.db).await?;
    let stage = if let Some(row) = updated {
        row.stage.clone()
    } else {
        sqlx::query_scalar!(
            "SELECT stage FROM entries WHERE id=? AND trashed_at IS NULL",
            id
        )
        .fetch_one(&app.db)
        .await?
    };
    Ok(Json(
        json!({"pause_requested": !["paused","complete","failed"].contains(&stage.as_str()),"stage":stage}),
    ))
}
async fn reanalyze(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    queue_rework(&app, &access, &id, "speakers", "identity-analysis.json").await
}
/// A fresh title (unless set by hand) and summary; the speakers stay as they are.
async fn summarize(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    queue_rework(&app, &access, &id, "summary", "summary.json").await
}
/// Queues `work` on a transcribed recording and drops its cached notes, so they are written afresh.
async fn queue_rework(
    app: &App,
    access: &Access,
    id: &str,
    work: &str,
    cache: &str,
) -> Result<Json<Value>> {
    access.admin()?;
    let _lock = app.mutations.lock().await;
    let row = sqlx::query!("SELECT stage,(SELECT count(*) FROM segments WHERE entry_id=entries.id) AS \"segments!: i64\" FROM entries WHERE id=? AND trashed_at IS NULL", id).fetch_optional(&app.db).await?.ok_or_else(Error::not_found)?;
    if !["complete", "failed", "paused"].contains(&row.stage.clone().as_str()) {
        return Err(Error::conflict("Recording is already processing"));
    }
    if row.segments == 0 {
        return Err(Error::conflict("Transcribe the recording first"));
    }
    match tokio::fs::remove_file(app.entry_dir(id).join(cache)).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let timestamp = now();
    sqlx::query!("UPDATE entries SET stage='queued',active_stage=NULL,pause_requested=0,requested_work=?,failure=NULL,updated_at=? WHERE id=?", work, timestamp, id).execute(&app.db).await?;
    app.wake.notify_one();
    Ok(Json(json!({"stage":"queued","work":work})))
}
async fn remove_voice(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    access.admin()?;
    let exists: i64 = sqlx::query_scalar!("SELECT count(*) FROM people WHERE id=?", id)
        .fetch_one(&app.db)
        .await?;
    if exists == 0 {
        return Err(Error::not_found());
    }
    sqlx::query!("DELETE FROM voice_samples WHERE person_id=?", id)
        .execute(&app.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
struct HistoryQuery {
    limit: Option<u32>,
    cursor: Option<u32>,
}
async fn source_history(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Value>> {
    access.read()?;
    let limit = query.limit.unwrap_or(500).clamp(1, 1000);
    let cursor = query.cursor.unwrap_or(0);
    let query_limit = i64::from(limit) + 1;
    let query_cursor = i64::from(cursor);
    let rows = sqlx::query!("SELECT e.id AS \"id!\",e.stage,e.active_stage,e.failure,e.pause_requested,e.trim_start_frame,e.trim_end_frame,COALESCE((SELECT json_extract(i.session_plan,'$['||ie.session_index||'].source_ids') FROM import_entries ie JOIN imports i ON i.id=ie.import_id WHERE ie.entry_id=e.id AND i.session_plan IS NOT NULL ORDER BY i.created_at DESC,i.rowid DESC LIMIT 1),(SELECT json_group_array(source_id) FROM (SELECT u.source_id FROM entry_sources es JOIN uploads u ON u.id=es.upload_id WHERE es.entry_id=e.id ORDER BY es.position))) AS \"source_ids!: String\" FROM entries e WHERE e.trashed_at IS NULL AND (EXISTS(SELECT 1 FROM import_entries ie JOIN imports i ON i.id=ie.import_id WHERE ie.entry_id=e.id AND i.session_plan IS NOT NULL) OR (EXISTS(SELECT 1 FROM entry_sources es JOIN uploads u ON u.id=es.upload_id WHERE es.entry_id=e.id AND u.source_id IS NOT NULL) AND NOT EXISTS(SELECT 1 FROM entry_sources es JOIN uploads u ON u.id=es.upload_id WHERE es.entry_id=e.id AND u.source_id IS NULL))) ORDER BY COALESCE(e.recorded_at,e.created_at) DESC,e.id LIMIT ? OFFSET ?", query_limit, query_cursor).fetch_all(&app.db).await?;
    let more = rows.len() > limit as usize;
    Ok(Json(
        json!({"entries":rows.iter().take(limit as usize).map(|row|json!({"id":row.id.clone(),"stage":row.stage.clone(),"active_stage":row.active_stage.clone(),"failure":row.failure.clone(),"pause_requested":(row.pause_requested != 0) && row.stage.clone() != "paused","trim_start_frame":row.trim_start_frame,"trim_end_frame":row.trim_end_frame,"source_ids":serde_json::from_str::<Value>(&row.source_ids.clone()).unwrap_or(json!([]))})).collect::<Vec<_>>(),"next_cursor":if more {Some(cursor+limit)} else {None}}),
    ))
}

async fn trash_entry(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    access.admin()?;
    let _lock = app.mutations.lock().await;
    // Trashing also cancels: a queued entry pauses now, a running one finishes its current step
    // and pauses at the worker's next checkpoint, which stops at a trashed entry.
    let trashed_at = now();
    let updated_at = now();
    let updated=sqlx::query!("UPDATE entries SET trashed_at=COALESCE(trashed_at,?),pause_requested=CASE WHEN stage='queued' THEN 1 ELSE pause_requested END,stage=CASE WHEN stage='queued' THEN 'paused' ELSE stage END,updated_at=? WHERE id=?", trashed_at, updated_at, id).execute(&app.db).await?;
    if updated.rows_affected() == 0 {
        return Err(Error::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn restore_entry(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    access.admin()?;
    let timestamp = now();
    let updated = sqlx::query!(
        "UPDATE entries SET trashed_at=NULL,updated_at=? WHERE id=?",
        timestamp,
        id
    )
    .execute(&app.db)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(Error::not_found());
    }
    Ok(Json(retrieval::get(&app, &id).await?))
}

async fn delete_entry(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    a.admin()?;
    let _lock = app.mutations.lock().await;
    let sources: Vec<String> =
        sqlx::query_scalar!("SELECT upload_id FROM entry_sources WHERE entry_id=?", id)
            .fetch_all(&app.db)
            .await?;
    // One statement, so the worker cannot claim the entry between the check and the delete.
    let deleted = sqlx::query!(
        "DELETE FROM entries WHERE id=? AND stage IN ('complete','failed','queued','paused')",
        id
    )
    .execute(&app.db)
    .await?;
    if deleted.rows_affected() == 0 {
        retrieval::get_including_trash(&app, &id).await?;
        return Err(Error::conflict(
            "Wait for processing to finish before deleting",
        ));
    }
    let _ = tokio::fs::remove_dir_all(app.entry_dir(&id)).await;
    for source in sources {
        let refs: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM entry_sources WHERE upload_id=?",
            source
        )
        .fetch_one(&app.db)
        .await?;
        if refs == 0 {
            sqlx::query!("DELETE FROM uploads WHERE id=?", source)
                .execute(&app.db)
                .await?;
            let _ = tokio::fs::remove_file(app.upload_path(&source)).await;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn people(State(app): State<App>, Extension(a): Extension<Access>) -> Result<Json<Value>> {
    a.read()?;
    let rows=sqlx::query!("SELECT p.id AS \"id!\",p.name,(SELECT COUNT(*) FROM voice_samples vs JOIN entries e ON e.id=vs.entry_id WHERE vs.person_id=p.id AND e.trashed_at IS NULL) AS samples FROM people p ORDER BY p.name").fetch_all(&app.db).await?;
    Ok(Json(
        json!({"people":rows.iter().map(|r|json!({"id":r.id.clone(),"name":r.name.clone(),"samples":r.samples})).collect::<Vec<_>>()}),
    ))
}
#[derive(Deserialize)]
struct Name {
    name: String,
}
async fn create_person(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Json(p): Json<Name>,
) -> Result<Json<Value>> {
    a.admin()?;
    let name = p.name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err(Error::bad("Name must be 1–120 characters"));
    }
    let id = id();
    sqlx::query!("INSERT INTO people(id,name)VALUES(?,?)", id, name)
        .execute(&app.db)
        .await?;
    Ok(Json(json!({"id":id,"name":name})))
}
async fn rename_person(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
    Json(p): Json<Name>,
) -> Result<StatusCode> {
    a.admin()?;
    if p.name.trim().is_empty() || p.name.len() > 120 {
        return Err(Error::bad("Name must be 1–120 characters"));
    }
    let trimmed_name = p.name.trim();
    sqlx::query!("UPDATE people SET name=? WHERE id=?", trimmed_name, id)
        .execute(&app.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete_person(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    a.admin()?;
    let _lock = app.mutations.lock().await;
    let mut tx = app.db.begin().await?;
    sqlx::query!(
        "UPDATE speakers SET person_id=NULL,state='unknown',evidence='[]' WHERE person_id=?",
        id
    )
    .execute(&mut *tx)
    .await?;
    let removed = sqlx::query!("DELETE FROM people WHERE id=?", id)
        .execute(&mut *tx)
        .await?;
    if removed.rows_affected() == 0 {
        return Err(Error::not_found());
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Assignment {
    person_id: Option<String>,
    locked: Option<bool>,
}
async fn assign_speaker(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path((id, label)): Path<(String, String)>,
    Json(p): Json<Assignment>,
) -> Result<StatusCode> {
    a.admin()?;
    if p.locked == Some(false) {
        if p.person_id.is_some() {
            return Err(Error::bad(
                "Resetting automation cannot also assign a person",
            ));
        }
        let mut tx = app.db.begin().await?;
        let updated = sqlx::query!("UPDATE speakers SET person_id=NULL,state='unknown',evidence='[]' WHERE entry_id=? AND label=?", id, label).execute(&mut *tx).await?;
        if updated.rows_affected() == 0 {
            return Err(Error::not_found());
        }
        sqlx::query!(
            "DELETE FROM voice_samples WHERE entry_id=? AND speaker=?",
            id,
            label
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    } else {
        crate::voice::assign(&app, &id, &label, p.person_id.as_deref()).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
type WebFiles = std::collections::HashMap<String, axum::body::Bytes>;
/// The built web app, read once at startup so it is served like an embedded bundle.
fn web_files(root: &std::path::Path) -> WebFiles {
    let mut files = WebFiles::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        // ponytail: an unreadable folder serves 404s; `serve` refuses to start without index.html.
        for path in std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
        {
            if path.is_dir() {
                dirs.push(path);
            } else if let (Ok(name), Ok(data)) = (path.strip_prefix(root), std::fs::read(&path)) {
                files.insert(name.to_string_lossy().into(), data.into());
            }
        }
    }
    files
}
async fn static_file(web: std::sync::Arc<WebFiles>, request: Request) -> Response {
    let path = request.uri().path().trim_start_matches('/');
    if path.starts_with("api/") {
        return Error::not_found().into_response();
    }
    let resource = web
        .get(if path.is_empty() { "index.html" } else { path })
        .or_else(|| {
            if !path.contains('.') {
                web.get("index.html")
            } else {
                None
            }
        });
    match resource {
        Some(asset) => {
            let mime = mime_guess::from_path(if path.contains('.') {
                path
            } else {
                "index.html"
            })
            .first_or_octet_stream()
            .to_string();
            // Vite fingerprints everything under assets/; the page itself must revalidate.
            let cache = if path.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            (
                [
                    (header::CONTENT_TYPE, mime.as_str()),
                    (header::CACHE_CONTROL, cache),
                ],
                asset.clone(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn web_files_serve_like_the_embedded_bundle() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("index.html"), "page").unwrap();
        std::fs::write(dir.path().join("assets/a.js"), "code").unwrap();
        let web = std::sync::Arc::new(super::web_files(dir.path()));
        let immutable = "public, max-age=31536000, immutable";
        for (path, status, body, kind, cache) in [
            ("/", 200, "page", "text/html", "no-cache"),
            ("/library/entry", 200, "page", "text/html", "no-cache"),
            ("/assets/a.js", 200, "code", "text/javascript", immutable),
            ("/assets/missing.js", 404, "", "", ""),
            ("/api/missing", 404, "", "application/json", ""),
        ] {
            let request = super::Request::get(path)
                .body(axum::body::Body::empty())
                .unwrap();
            let response = super::static_file(web.clone(), request).await;
            let header = |name| {
                response
                    .headers()
                    .get(name)
                    .map_or("", |v| v.to_str().unwrap())
                    .to_string()
            };
            assert_eq!(response.status(), status, "{path}");
            assert_eq!(header(super::header::CONTENT_TYPE), kind, "{path}");
            assert_eq!(header(super::header::CACHE_CONTROL), cache, "{path}");
            if status == 200 {
                let bytes = axum::body::to_bytes(response.into_body(), 1024).await;
                assert_eq!(bytes.unwrap(), body, "{path}");
            }
        }
    }
}
