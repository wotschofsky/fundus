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
use sqlx::Row;
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
    let transfers = Router::new()
        .route("/api/v1/imports", post(uploads::create_import))
        .route("/api/v1/imports/{id}", get(uploads::status))
        .route("/api/v1/imports/{id}/complete", post(uploads::complete))
        .route(
            "/api/v1/uploads",
            post(uploads::create).options(uploads::options),
        )
        .route(
            "/api/v1/uploads/{id}",
            axum::routing::head(uploads::head)
                .patch(uploads::patch)
                .delete(uploads::terminate)
                .options(uploads::options),
        )
        .route_layer(middleware::from_fn_with_state(app.clone(), auth::require));
    let protected = Router::new()
        .route(
            "/api/v1/oauth/authorize",
            get(oauth::authorization).post(oauth::consent),
        )
        .route("/api/v1/oauth/connections", get(oauth::connections))
        .route(
            "/api/v1/oauth/connections/{id}",
            axum::routing::delete(oauth::disconnect),
        )
        .route("/api/v1/me", get(me))
        .route(
            "/api/v1/settings",
            get(settings::get).patch(settings::update),
        )
        .route("/api/v1/entries/{id}/trash", post(trash_entry))
        .route("/api/v1/entries/{id}/restore", post(restore_entry))
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/tokens", get(tokens).post(create_token))
        .route("/api/v1/tokens/{id}", axum::routing::delete(revoke))
        .route("/api/v1/entries", get(entries))
        .route(
            "/api/v1/entries/{id}",
            get(entry).patch(update_entry).delete(delete_entry),
        )
        .route("/api/v1/entries/{id}/transcript", get(read))
        .route(
            "/api/v1/entries/{id}/audio",
            get(audio_file).head(audio_file),
        )
        .route("/api/v1/entries/{id}/retry", post(retry))
        .route("/api/v1/entries/{id}/pause", post(pause))
        .route("/api/v1/entries/{id}/reanalyze", post(reanalyze))
        .route("/api/v1/source-history", get(source_history))
        .route(
            "/api/v1/people/{id}/voice",
            axum::routing::delete(remove_voice),
        )
        .route(
            "/api/v1/entries/{id}/speakers/{label}",
            patch(assign_speaker),
        )
        .route("/api/v1/people", get(people).post(create_person))
        .route(
            "/api/v1/people/{id}",
            patch(rename_person).delete(delete_person),
        )
        .route("/api/v1/labels", get(labels))
        .route("/api/v1/search", get(search))
        .route_layer(middleware::from_fn_with_state(
            app.clone(),
            auth::require_browser,
        ));
    // Installed clients use an OAuth-only namespace. Proxy trust and legacy
    // upload/read credentials never authenticate these owner-capability routes.
    let native = Router::new()
        .route("/v1/me", get(me))
        .route("/v1/settings", get(settings::get).patch(settings::update))
        .route("/v1/entries/{id}/trash", post(trash_entry))
        .route("/v1/entries/{id}/restore", post(restore_entry))
        .route("/v1/imports", post(uploads::create_import))
        .route("/v1/imports/{id}", get(uploads::status))
        .route("/v1/imports/{id}/complete", post(uploads::complete))
        .route(
            "/v1/uploads",
            post(uploads::create).options(uploads::options),
        )
        .route(
            "/v1/uploads/{id}",
            axum::routing::head(uploads::head)
                .patch(uploads::patch)
                .delete(uploads::terminate)
                .options(uploads::options),
        )
        .route("/v1/entries", get(entries))
        .route(
            "/v1/entries/{id}",
            get(entry).patch(update_entry).delete(delete_entry),
        )
        .route("/v1/entries/{id}/transcript", get(read))
        .route("/v1/entries/{id}/audio", get(audio_file).head(audio_file))
        .route("/v1/entries/{id}/retry", post(retry))
        .route("/v1/entries/{id}/pause", post(pause))
        .route("/v1/entries/{id}/reanalyze", post(reanalyze))
        .route("/v1/source-history", get(source_history))
        .route("/v1/people/{id}/voice", axum::routing::delete(remove_voice))
        .route("/v1/entries/{id}/speakers/{label}", patch(assign_speaker))
        .route("/v1/people", get(people).post(create_person))
        .route(
            "/v1/people/{id}",
            patch(rename_person).delete(delete_person),
        )
        .route("/v1/labels", get(labels))
        .route("/v1/search", get(search))
        .fallback(|| async { Error::not_found() })
        .layer(middleware::from_fn_with_state(
            app.clone(),
            oauth::require_native,
        ));
    Router::new()
        .route("/healthz", get(|| async { Json(json!({"ok":true})) }))
        .route("/api/login", post(auth::login))
        .merge(oauth::public_router())
        .merge(protected)
        .merge(transfers)
        .merge(mcp)
        .nest("/api/native", native)
        .fallback(static_file)
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
        .layer(middleware::from_fn(security_headers))
        .with_state(app)
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
        sqlx::query("DELETE FROM credentials WHERE hash=? AND scope='session'")
            .bind(auth::hash(token))
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
    let rows=sqlx::query("SELECT hash,name,scope,created_at FROM credentials WHERE scope!='session' ORDER BY created_at DESC").fetch_all(&app.db).await?;
    Ok(Json(
        json!({"tokens":rows.iter().map(|r|json!({"id":r.get::<String,_>("hash"),"name":r.get::<String,_>("name"),"scope":r.get::<String,_>("scope"),"created_at":r.get::<String,_>("created_at")})).collect::<Vec<_>>()}),
    ))
}
async fn revoke(
    State(app): State<App>,
    Extension(a): Extension<Access>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    a.admin()?;
    sqlx::query("DELETE FROM credentials WHERE hash=? AND scope!='session'")
        .bind(id)
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
    let exists: Option<String> =
        sqlx::query_scalar("SELECT audio_path FROM entries WHERE id=? AND trashed_at IS NULL")
            .bind(&id)
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
        sqlx::query("UPDATE entries SET title=?,title_manual=1,updated_at=? WHERE id=?")
            .bind(title)
            .bind(now())
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE passage_fts SET title=? WHERE entry_id=?")
            .bind(title)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(labels) = p.labels {
        if labels.len() > 20 {
            return Err(Error::bad("Use at most 20 labels"));
        }
        sqlx::query("DELETE FROM entry_labels WHERE entry_id=?")
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        for label in labels {
            let label = label.trim();
            if label.is_empty() || label.len() > 64 || label.contains(',') {
                return Err(Error::bad("Labels must be 1–64 characters without commas"));
            }
            sqlx::query("INSERT OR IGNORE INTO labels(name)VALUES(?)")
                .bind(label)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT OR IGNORE INTO entry_labels(entry_id,label)VALUES(?,?)")
                .bind(&id)
                .bind(label)
                .execute(&mut *tx)
                .await?;
        }
    }
    if let Some(time) = p.recorded_at {
        let time = chrono::DateTime::parse_from_rfc3339(&time)
            .map_err(|_| Error::bad("Recording time must include a UTC offset"))?
            .with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        sqlx::query("UPDATE entries SET recorded_at=? WHERE id=?")
            .bind(time)
            .bind(&id)
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
    let state: Option<String> =
        sqlx::query_scalar("SELECT stage FROM entries WHERE id=? AND trashed_at IS NULL")
            .bind(&id)
            .fetch_optional(&app.db)
            .await?;
    let state = state.ok_or_else(Error::not_found)?;
    if !["failed", "complete", "paused"].contains(&state.as_str()) {
        return Err(Error::conflict("Recording is already processing"));
    }
    let mut tx = app.db.begin().await?;
    sqlx::query("DELETE FROM parts WHERE entry_id=? AND raw_json IS NULL")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE entries SET stage='queued',failure=NULL,pause_requested=0,requested_work=CASE WHEN stage='complete' THEN NULL ELSE requested_work END WHERE id=?")
        .bind(&id)
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
    let stage: Option<String> =
        sqlx::query_scalar("SELECT stage FROM entries WHERE id=? AND trashed_at IS NULL")
            .bind(&id)
            .fetch_optional(&app.db)
            .await?;
    let stage = stage.ok_or_else(Error::not_found)?;
    if stage == "paused" {
        return Ok(Json(json!({"stage":"paused","pause_requested":false})));
    }
    if ["complete", "failed"].contains(&stage.as_str()) {
        return Err(Error::conflict("Recording is not processing"));
    }
    let updated = sqlx::query("UPDATE entries SET pause_requested=1,stage=CASE WHEN stage='queued' THEN 'paused' ELSE stage END,updated_at=? WHERE id=? AND stage NOT IN ('complete','failed','paused') RETURNING stage")
        .bind(now()).bind(&id).fetch_optional(&app.db).await?;
    let stage = if let Some(row) = updated {
        row.get::<String, _>("stage")
    } else {
        sqlx::query_scalar("SELECT stage FROM entries WHERE id=? AND trashed_at IS NULL")
            .bind(&id)
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
    access.admin()?;
    let _lock = app.mutations.lock().await;
    let row = sqlx::query("SELECT stage,(SELECT count(*) FROM segments WHERE entry_id=entries.id) AS segments FROM entries WHERE id=? AND trashed_at IS NULL").bind(&id).fetch_optional(&app.db).await?.ok_or_else(Error::not_found)?;
    if !["complete", "failed", "paused"].contains(&row.get::<String, _>("stage").as_str()) {
        return Err(Error::conflict("Recording is already processing"));
    }
    if row.get::<i64, _>("segments") == 0 {
        return Err(Error::conflict(
            "Transcribe the recording before reanalyzing its speakers",
        ));
    }
    match tokio::fs::remove_file(app.entry_dir(&id).join("identity-analysis.json")).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    sqlx::query("UPDATE entries SET stage='queued',active_stage=NULL,pause_requested=0,requested_work='speakers',failure=NULL,updated_at=? WHERE id=?")
        .bind(now()).bind(id).execute(&app.db).await?;
    app.wake.notify_one();
    Ok(Json(json!({"stage":"queued","work":"speakers"})))
}
async fn remove_voice(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    access.admin()?;
    let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM people WHERE id=?")
        .bind(&id)
        .fetch_one(&app.db)
        .await?;
    if exists == 0 {
        return Err(Error::not_found());
    }
    sqlx::query("DELETE FROM voice_samples WHERE person_id=?")
        .bind(id)
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
    let rows = sqlx::query("SELECT e.id,e.stage,e.active_stage,e.failure,e.pause_requested,e.trim_start_frame,e.trim_end_frame,COALESCE((SELECT json_extract(i.session_plan,'$['||ie.session_index||'].source_ids') FROM import_entries ie JOIN imports i ON i.id=ie.import_id WHERE ie.entry_id=e.id AND i.session_plan IS NOT NULL ORDER BY i.created_at DESC,i.rowid DESC LIMIT 1),(SELECT json_group_array(source_id) FROM (SELECT u.source_id FROM entry_sources es JOIN uploads u ON u.id=es.upload_id WHERE es.entry_id=e.id ORDER BY es.position))) AS source_ids FROM entries e WHERE e.trashed_at IS NULL AND (EXISTS(SELECT 1 FROM import_entries ie JOIN imports i ON i.id=ie.import_id WHERE ie.entry_id=e.id AND i.session_plan IS NOT NULL) OR (EXISTS(SELECT 1 FROM entry_sources es JOIN uploads u ON u.id=es.upload_id WHERE es.entry_id=e.id AND u.source_id IS NOT NULL) AND NOT EXISTS(SELECT 1 FROM entry_sources es JOIN uploads u ON u.id=es.upload_id WHERE es.entry_id=e.id AND u.source_id IS NULL))) ORDER BY COALESCE(e.recorded_at,e.created_at) DESC,e.id LIMIT ? OFFSET ?")
        .bind(i64::from(limit)+1).bind(i64::from(cursor)).fetch_all(&app.db).await?;
    let more = rows.len() > limit as usize;
    Ok(Json(
        json!({"entries":rows.iter().take(limit as usize).map(|row|json!({"id":row.get::<String,_>("id"),"stage":row.get::<String,_>("stage"),"active_stage":row.get::<Option<String>,_>("active_stage"),"failure":row.get::<Option<String>,_>("failure"),"pause_requested":row.get::<bool,_>("pause_requested") && row.get::<String,_>("stage") != "paused","trim_start_frame":row.get::<Option<i64>,_>("trim_start_frame"),"trim_end_frame":row.get::<Option<i64>,_>("trim_end_frame"),"source_ids":serde_json::from_str::<Value>(&row.get::<String,_>("source_ids")).unwrap_or(json!([]))})).collect::<Vec<_>>(),"next_cursor":if more {Some(cursor+limit)} else {None}}),
    ))
}

async fn trash_entry(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    access.admin()?;
    let _lock = app.mutations.lock().await;
    let stage: Option<String> = sqlx::query_scalar("SELECT stage FROM entries WHERE id=?")
        .bind(&id)
        .fetch_optional(&app.db)
        .await?;
    let stage = stage.ok_or_else(Error::not_found)?;
    if !["complete", "failed", "queued", "paused"].contains(&stage.as_str()) {
        return Err(Error::conflict(
            "Pause or finish processing before moving the recording to Trash",
        ));
    }
    let updated=sqlx::query("UPDATE entries SET trashed_at=COALESCE(trashed_at,?),pause_requested=CASE WHEN stage='queued' THEN 1 ELSE pause_requested END,stage=CASE WHEN stage='queued' THEN 'paused' ELSE stage END,updated_at=? WHERE id=? AND stage IN ('complete','failed','queued','paused')")
        .bind(now()).bind(now()).bind(id).execute(&app.db).await?;
    if updated.rows_affected() == 0 {
        return Err(Error::conflict(
            "Processing started; pause it before moving the recording to Trash",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn restore_entry(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    access.admin()?;
    let updated = sqlx::query("UPDATE entries SET trashed_at=NULL,updated_at=? WHERE id=?")
        .bind(now())
        .bind(&id)
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
    let v = retrieval::get_including_trash(&app, &id).await?;
    if !["complete", "failed", "queued", "paused"].contains(&v["stage"].as_str().unwrap_or("")) {
        return Err(Error::conflict(
            "Wait for processing to finish before deleting",
        ));
    }
    let sources: Vec<String> =
        sqlx::query_scalar("SELECT upload_id FROM entry_sources WHERE entry_id=?")
            .bind(&id)
            .fetch_all(&app.db)
            .await?;
    sqlx::query("DELETE FROM entries WHERE id=?")
        .bind(&id)
        .execute(&app.db)
        .await?;
    let _ = tokio::fs::remove_dir_all(app.entry_dir(&id)).await;
    for source in sources {
        let refs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entry_sources WHERE upload_id=?")
            .bind(&source)
            .fetch_one(&app.db)
            .await?;
        if refs == 0 {
            sqlx::query("DELETE FROM uploads WHERE id=?")
                .bind(&source)
                .execute(&app.db)
                .await?;
            let _ = tokio::fs::remove_file(app.upload_path(&source)).await;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn people(State(app): State<App>, Extension(a): Extension<Access>) -> Result<Json<Value>> {
    a.read()?;
    let rows=sqlx::query("SELECT p.id,p.name,(SELECT COUNT(*) FROM voice_samples vs JOIN entries e ON e.id=vs.entry_id WHERE vs.person_id=p.id AND e.trashed_at IS NULL) AS samples FROM people p ORDER BY p.name").fetch_all(&app.db).await?;
    Ok(Json(
        json!({"people":rows.iter().map(|r|json!({"id":r.get::<String,_>("id"),"name":r.get::<String,_>("name"),"samples":r.get::<i64,_>("samples")})).collect::<Vec<_>>()}),
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
    sqlx::query("INSERT INTO people(id,name)VALUES(?,?)")
        .bind(&id)
        .bind(name)
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
    sqlx::query("UPDATE people SET name=? WHERE id=?")
        .bind(p.name.trim())
        .bind(id)
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
    sqlx::query(
        "UPDATE speakers SET person_id=NULL,state='unknown',evidence='[]' WHERE person_id=?",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    let removed = sqlx::query("DELETE FROM people WHERE id=?")
        .bind(&id)
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
        let updated = sqlx::query("UPDATE speakers SET person_id=NULL,state='unknown',evidence='[]' WHERE entry_id=? AND label=?").bind(&id).bind(&label).execute(&mut *tx).await?;
        if updated.rows_affected() == 0 {
            return Err(Error::not_found());
        }
        sqlx::query("DELETE FROM voice_samples WHERE entry_id=? AND speaker=?")
            .bind(&id)
            .bind(&label)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    } else {
        crate::voice::assign(&app, &id, &label, p.person_id.as_deref()).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
#[derive(rust_embed::Embed)]
#[folder = "../../web/dist/"]
struct Assets;
async fn static_file(request: Request) -> Response {
    let path = request.uri().path().trim_start_matches('/');
    if path.starts_with("api/") {
        return Error::not_found().into_response();
    }
    let resource = Assets::get(if path.is_empty() { "index.html" } else { path }).or_else(|| {
        if !path.contains('.') {
            Assets::get("index.html")
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
            ([(header::CONTENT_TYPE, mime)], asset.data.into_owned()).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
