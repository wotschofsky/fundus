use crate::{
    auth::Access,
    config::{App, id, now},
    error::{Error, Result},
    grouping::{self, SessionPlan},
};
use axum::{
    Extension, Json,
    extract::{OriginalUri, Path, Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::Engine;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
const MAX_FILE: i64 = 32 * 1024 * 1024 * 1024;
#[derive(Deserialize)]
pub struct NewImport {
    pub recorder_timezone: Option<String>,
    pub sessions: Option<Vec<SessionPlan>>,
}
pub async fn create_import(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Json(body): Json<NewImport>,
) -> Result<Json<Value>> {
    access.upload()?;
    if let Some(sessions) = &body.sessions {
        grouping::validate_plan(sessions).map_err(|error| Error::bad(error.to_string()))?;
    }
    let zone = body
        .recorder_timezone
        .unwrap_or_else(|| app.config.recorder_timezone.clone());
    zone.parse::<chrono_tz::Tz>()
        .map_err(|_| Error::bad("Unknown recorder timezone"))?;
    let id = id();
    sqlx::query("INSERT INTO imports(id,recorder_timezone,created_at,session_plan)VALUES(?,?,?,?)")
        .bind(&id)
        .bind(zone)
        .bind(now())
        .bind(
            body.sessions
                .as_ref()
                .map(|sessions| serde_json::to_string(sessions).unwrap()),
        )
        .execute(&app.db)
        .await?;
    Ok(Json(json!({"id":id,"state":"uploading"})))
}
pub async fn status(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    access.upload()?;
    let row = sqlx::query("SELECT * FROM imports WHERE id=?")
        .bind(&id)
        .fetch_optional(&app.db)
        .await?
        .ok_or_else(Error::not_found)?;
    let uploads=sqlx::query("SELECT id,source_id,original_name,length,offset,state,failure FROM uploads WHERE import_id=? ORDER BY rowid").bind(&id).fetch_all(&app.db).await?;
    let entries=sqlx::query("SELECT e.id,e.stage,e.active_stage,e.failure,e.pause_requested,e.duration_ms,e.recorded_at,ie.session_index FROM import_entries ie JOIN entries e ON e.id=ie.entry_id WHERE ie.import_id=? ORDER BY ie.session_index").bind(&id).fetch_all(&app.db).await?;
    Ok(Json(
        json!({"id":id,"state":row.get::<String,_>("state"),"entries":entries.iter().map(|r|json!({"id":r.get::<String,_>("id"),"session_index":r.get::<i64,_>("session_index"),"stage":r.get::<String,_>("stage"),"active_stage":r.get::<Option<String>,_>("active_stage"),"pause_requested":r.get::<bool,_>("pause_requested") && r.get::<String,_>("stage") != "paused","failure":r.get::<Option<String>,_>("failure"),"duration_ms":r.get::<i64,_>("duration_ms"),"recorded_at":r.get::<Option<String>,_>("recorded_at")})).collect::<Vec<_>>(),"files":uploads.into_iter().map(|r|json!({"id":r.get::<String,_>("id"),"source_id":r.get::<Option<String>,_>("source_id"),"filename":r.get::<String,_>("original_name"),"length":r.get::<i64,_>("length"),"offset":r.get::<i64,_>("offset"),"state":r.get::<String,_>("state"),"failure":r.get::<Option<String>,_>("failure")})).collect::<Vec<_>>() }),
    ))
}
fn tus(status: StatusCode, values: Vec<(&'static str, String)>) -> Response {
    let mut response = status.into_response();
    let h = response.headers_mut();
    h.insert("tus-resumable", "1.0.0".parse().unwrap());
    h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    for (k, v) in values {
        if let Ok(v) = v.parse() {
            h.insert(k, v);
        }
    }
    response
}
pub async fn options() -> Response {
    tus(
        StatusCode::NO_CONTENT,
        vec![
            ("tus-version", "1.0.0".into()),
            ("tus-extension", "creation,termination".into()),
            ("tus-max-size", MAX_FILE.to_string()),
        ],
    )
}
fn version(h: &HeaderMap) -> Result<()> {
    if h.get("tus-resumable").and_then(|v| v.to_str().ok()) != Some("1.0.0") {
        return Err(Error(
            StatusCode::PRECONDITION_FAILED,
            "Tus-Resumable: 1.0.0 required".into(),
        ));
    }
    Ok(())
}
fn metadata(h: &HeaderMap) -> Result<std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    let s = h
        .get("upload-metadata")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if s.len() > 8192 {
        return Err(Error::bad("Metadata too large"));
    }
    for item in s.split(',').filter(|s| !s.is_empty()) {
        let (k, v) = item
            .trim()
            .split_once(' ')
            .ok_or_else(|| Error::bad("Malformed upload metadata"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(v)
            .map_err(|_| Error::bad("Malformed metadata encoding"))?;
        out.insert(
            k.to_string(),
            String::from_utf8(bytes).map_err(|_| Error::bad("Metadata must be UTF-8"))?,
        );
    }
    Ok(out)
}
pub async fn create(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    OriginalUri(uri): OriginalUri,
    h: HeaderMap,
) -> Result<Response> {
    access.upload()?;
    version(&h)?;
    let length = h
        .get("upload-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|n| *n > 0 && *n <= MAX_FILE)
        .ok_or_else(|| Error::bad("Invalid upload length"))?;
    let m = metadata(&h)?;
    let import = m
        .get("import_id")
        .ok_or_else(|| Error::bad("import_id metadata required"))?;
    let filename = m
        .get("filename")
        .ok_or_else(|| Error::bad("filename metadata required"))?;
    let original = m.get("original_name").unwrap_or(filename);
    if filename.len() > 1024 || original.len() > 1024 || original.contains(['\n', '\r', '\0']) {
        return Err(Error::bad("Invalid filename"));
    }
    let _guard = app.mutations.lock().await;
    let row = sqlx::query("SELECT state,session_plan FROM imports WHERE id=?")
        .bind(import)
        .fetch_optional(&app.db)
        .await?;
    let Some(row) = row else {
        return Err(Error::conflict("Import is missing or already finalized"));
    };
    if row.get::<String, _>("state") != "uploading" {
        return Err(Error::conflict("Import is missing or already finalized"));
    }
    let source_id = m.get("source_id");
    if source_id.is_some_and(|value| !grouping::validate_source_id(value)) {
        return Err(Error::bad("Invalid source ID"));
    }
    if let Some(plan) = row.get::<Option<String>, _>("session_plan") {
        let sessions: Vec<SessionPlan> =
            serde_json::from_str(&plan).map_err(|_| Error::bad("Invalid session plan"))?;
        if !source_id.is_some_and(|source| {
            sessions
                .iter()
                .any(|session| session.source_ids.contains(source))
        }) {
            return Err(Error::bad(
                "source_id must identify a source in this import's session plan",
            ));
        }
    }
    if let Some(source) = source_id {
        let prior =
            sqlx::query("SELECT id,filename,original_name,relative_path,length,offset,claimed_pcm_hash FROM uploads WHERE import_id=? AND source_id=?")
                .bind(import)
                .bind(source)
                .fetch_optional(&app.db)
                .await?;
        if let Some(prior) = prior {
            if prior.get::<i64, _>("length") != length
                || prior.get::<String, _>("filename") != *filename
                || prior.get::<String, _>("original_name") != *original
                || prior.get::<String, _>("relative_path")
                    != *m.get("relative_path").unwrap_or(original)
                || prior.get::<Option<String>, _>("claimed_pcm_hash").as_ref() != m.get("pcm_hash")
            {
                return Err(Error::conflict(
                    "This source ID already belongs to a different upload",
                ));
            }
            let id: String = prior.get("id");
            return Ok(tus(
                StatusCode::CREATED,
                vec![
                    (
                        "location",
                        format!(
                            "{}/v1/uploads/{id}",
                            if uri.path().starts_with("/api/native/") {
                                "/api/native"
                            } else {
                                "/api"
                            }
                        ),
                    ),
                    ("upload-offset", prior.get::<i64, _>("offset").to_string()),
                    ("upload-length", length.to_string()),
                ],
            ));
        }
    }
    let id = id();
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(app.upload_path(&id))
        .await?;
    file.sync_all().await?;
    sqlx::query("INSERT INTO uploads(id,import_id,filename,original_name,relative_path,length,claimed_pcm_hash,source_id)VALUES(?,?,?,?,?,?,?,?)").bind(&id).bind(import).bind(filename).bind(original).bind(m.get("relative_path").unwrap_or(original)).bind(length).bind(m.get("pcm_hash")).bind(source_id).execute(&app.db).await?;
    Ok(tus(
        StatusCode::CREATED,
        vec![
            (
                "location",
                format!(
                    "{}/v1/uploads/{id}",
                    if uri.path().starts_with("/api/native/") {
                        "/api/native"
                    } else {
                        "/api"
                    }
                ),
            ),
            ("upload-offset", "0".into()),
        ],
    ))
}
pub async fn head(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Result<Response> {
    access.upload()?;
    version(&h)?;
    let r = sqlx::query("SELECT offset,length FROM uploads WHERE id=?")
        .bind(id)
        .fetch_optional(&app.db)
        .await?
        .ok_or_else(Error::not_found)?;
    Ok(tus(
        StatusCode::OK,
        vec![
            ("upload-offset", r.get::<i64, _>("offset").to_string()),
            ("upload-length", r.get::<i64, _>("length").to_string()),
        ],
    ))
}
pub async fn patch(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response> {
    access.upload()?;
    version(request.headers())?;
    if request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|s| s.to_str().ok())
        != Some("application/offset+octet-stream")
    {
        return Err(Error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Use application/offset+octet-stream".into(),
        ));
    }
    let offset = request
        .headers()
        .get("upload-offset")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| Error::bad("Upload-Offset required"))?;
    let lock = {
        let mut locks = app.upload_locks.lock().await;
        locks.entry(id.clone()).or_default().clone()
    };
    let _guard = lock.lock().await;
    let r=sqlx::query("SELECT u.offset,u.length,u.state,i.state AS import_state FROM uploads u JOIN imports i ON i.id=u.import_id WHERE u.id=?").bind(&id).fetch_optional(&app.db).await?.ok_or_else(Error::not_found)?;
    let saved: i64 = r.get("offset");
    let length: i64 = r.get("length");
    if offset != saved || r.get::<String, _>("import_state") != "uploading" {
        return Err(Error::conflict(
            "Offset mismatch or import already finalized; query HEAD before retrying",
        ));
    }
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(app.upload_path(&id))
        .await?;
    file.set_len(saved as u64).await?;
    file.seek(std::io::SeekFrom::Start(saved as u64)).await?;
    let mut stream = request.into_body().into_data_stream();
    let mut count = 0i64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::bad("Upload body interrupted; resume using HEAD"))?;
        count += chunk.len() as i64;
        if saved + count > length || count > 8 * 1024 * 1024 {
            file.set_len(saved as u64).await?;
            return Err(Error::bad(
                "Chunk exceeds upload length or 8 MiB chunk limit",
            ));
        }
        file.write_all(&chunk).await?;
    }
    file.sync_all().await?;
    sqlx::query("UPDATE uploads SET offset=?,state=? WHERE id=?")
        .bind(saved + count)
        .bind(if saved + count == length {
            "received"
        } else {
            "uploading"
        })
        .bind(&id)
        .execute(&app.db)
        .await?;
    Ok(tus(
        StatusCode::NO_CONTENT,
        vec![("upload-offset", (saved + count).to_string())],
    ))
}
pub async fn terminate(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Result<Response> {
    access.upload()?;
    version(&h)?;
    let lock = {
        let mut locks = app.upload_locks.lock().await;
        locks.entry(id.clone()).or_default().clone()
    };
    let _upload_guard = lock.lock().await;
    let _guard = app.mutations.lock().await;
    let state: Option<String> = sqlx::query_scalar(
        "SELECT i.state FROM imports i JOIN uploads u ON u.import_id=i.id WHERE u.id=?",
    )
    .bind(&id)
    .fetch_optional(&app.db)
    .await?;
    if state.as_deref() != Some("uploading") {
        return Err(Error::conflict(
            "Only unfinished imports can remove uploads",
        ));
    }
    sqlx::query("DELETE FROM uploads WHERE id=?")
        .bind(&id)
        .execute(&app.db)
        .await?;
    let _ = tokio::fs::remove_file(app.upload_path(&id)).await;
    Ok(tus(StatusCode::NO_CONTENT, vec![]))
}
pub async fn complete(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    access.upload()?;
    let _guard = app.mutations.lock().await;
    let state: Option<String> = sqlx::query_scalar("SELECT state FROM imports WHERE id=?")
        .bind(&id)
        .fetch_optional(&app.db)
        .await?;
    let state = state.ok_or_else(Error::not_found)?;
    if state == "uploading" {
        let plan: Option<String> =
            sqlx::query_scalar("SELECT session_plan FROM imports WHERE id=?")
                .bind(&id)
                .fetch_one(&app.db)
                .await?;
        if let Some(plan) = plan {
            let sessions: Vec<SessionPlan> =
                serde_json::from_str(&plan).map_err(|_| Error::bad("Invalid session plan"))?;
            let expected: std::collections::HashSet<_> = sessions
                .iter()
                .flat_map(|session| session.source_ids.iter().cloned())
                .collect();
            let actual: Vec<Option<String>> =
                sqlx::query_scalar("SELECT source_id FROM uploads WHERE import_id=?")
                    .bind(&id)
                    .fetch_all(&app.db)
                    .await?;
            if actual.iter().any(Option::is_none)
                || actual.len() != expected.len()
                || actual
                    .iter()
                    .flatten()
                    .any(|source| !expected.contains(source))
            {
                return Err(Error::conflict(
                    "Upload every source in the session plan before completing the import",
                ));
            }
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM uploads WHERE import_id=?")
            .bind(&id)
            .fetch_one(&app.db)
            .await?;
        let pending: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM uploads WHERE import_id=? AND offset!=length")
                .bind(&id)
                .fetch_one(&app.db)
                .await?;
        if count == 0 || pending != 0 {
            return Err(Error::conflict(
                "Upload every file before completing the import",
            ));
        }
        sqlx::query("UPDATE imports SET state='verifying' WHERE id=?")
            .bind(&id)
            .execute(&app.db)
            .await?;
        app.wake.notify_one();
    }
    Ok(Json(
        json!({"id":id,"state":if state=="uploading"{"verifying"}else{&state}}),
    ))
}
