//! OAuth authorization for separate MCP read-only and native owner resources.
//! Browser identity belongs to existing password/proxy authentication. Native
//! owner tokens work only on the dedicated, bearer-authenticated native API.
use crate::{
    auth::{self, Access},
    config::{App, id},
    error::Error,
};
use axum::{
    Extension, Form, Json, Router,
    extract::{DefaultBodyLimit, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Sqlite, Transaction};

pub const SCOPE: &str = "mcp:read";
pub const NATIVE_SCOPE: &str = "vault:owner";
const ACCESS_LIFETIME: i64 = 3600;
const REFRESH_LIFETIME: i64 = 30 * 86400;
type Result<T> = std::result::Result<T, OAuthError>;
#[derive(Debug)]
pub struct OAuthError(StatusCode, &'static str, String);
impl OAuthError {
    fn bad(code: &'static str, message: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, code, message.into())
    }
    fn grant() -> Self {
        Self::bad(
            "invalid_grant",
            "The authorization grant is invalid, expired, or already used",
        )
    }
    fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(%error, "OAuth storage operation failed");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "The authorization server could not complete the request".into(),
        )
    }
}
impl From<sqlx::Error> for OAuthError {
    fn from(error: sqlx::Error) -> Self {
        Self::internal(error)
    }
}
impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        let mut response = (
            self.0,
            Json(json!({"error":self.1,"error_description":self.2})),
        )
            .into_response();
        no_store(&mut response);
        if self.1 == "invalid_client" {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                "Basic realm=\"OAuth\"".parse().unwrap(),
            );
        }
        response
    }
}
fn seconds() -> i64 {
    chrono::Utc::now().timestamp()
}
fn issuer(app: &App) -> String {
    app.config.public_url.origin().ascii_serialization()
}
pub fn resource(app: &App) -> String {
    app.config.public_url.join("mcp").unwrap().to_string()
}
pub fn native_resource(app: &App) -> String {
    app.config
        .public_url
        .join("api/native")
        .unwrap()
        .to_string()
}
fn resource_scope(app: &App, supplied: &str) -> Option<(&'static str, String)> {
    let normalized = url::Url::parse(supplied).ok()?.to_string();
    if normalized == resource(app) {
        Some((SCOPE, normalized))
    } else if normalized == native_resource(app) {
        Some((NATIVE_SCOPE, normalized))
    } else {
        None
    }
}
fn endpoint(app: &App, path: &str) -> String {
    app.config.public_url.join(path).unwrap().to_string()
}
fn no_store(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::PRAGMA, "no-cache".parse().unwrap());
}
fn response(value: Value) -> Response {
    let mut response = Json(value).into_response();
    no_store(&mut response);
    response
}

pub fn public_router() -> Router<App> {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_metadata).options(preflight),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(protected_metadata).options(preflight),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(server_metadata).options(preflight),
        )
        .route(
            "/.well-known/oauth-protected-resource/api/native",
            get(native_metadata).options(preflight),
        )
        .route("/oauth/register", post(register).options(preflight))
        .route("/oauth/token", post(token).options(preflight))
        .route("/oauth/revoke", post(revoke_token).options(preflight))
        .layer(middleware::from_fn(public_headers))
        .layer(DefaultBodyLimit::max(16 * 1024))
}
async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}
async fn public_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (name, value) in [
        ("access-control-allow-origin", "*"),
        ("access-control-allow-methods", "GET, POST, OPTIONS"),
        (
            "access-control-allow-headers",
            "Content-Type, Authorization, MCP-Protocol-Version",
        ),
    ] {
        response.headers_mut().insert(name, value.parse().unwrap());
    }
    response
}
async fn protected_metadata(State(app): State<App>) -> Json<Value> {
    Json(
        json!({"resource":resource(&app),"authorization_servers":[issuer(&app)],"scopes_supported":[SCOPE],"bearer_methods_supported":["header"],"resource_name":"Fundus"}),
    )
}
async fn native_metadata(State(app): State<App>) -> Json<Value> {
    Json(
        json!({"resource":native_resource(&app),"authorization_servers":[issuer(&app)],"scopes_supported":[NATIVE_SCOPE],"bearer_methods_supported":["header"],"resource_name":"Fundus native app"}),
    )
}
async fn server_metadata(State(app): State<App>) -> Json<Value> {
    Json(
        json!({"issuer":issuer(&app),"authorization_endpoint":endpoint(&app,"oauth/authorize"),"token_endpoint":endpoint(&app,"oauth/token"),"registration_endpoint":endpoint(&app,"oauth/register"),"revocation_endpoint":endpoint(&app,"oauth/revoke"),"scopes_supported":[SCOPE,NATIVE_SCOPE],"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none","client_secret_basic","client_secret_post"],"revocation_endpoint_auth_methods_supported":["none","client_secret_basic","client_secret_post"],"authorization_response_iss_parameter_supported":true}),
    )
}

#[derive(Deserialize)]
pub struct Registration {
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: Option<String>,
    grant_types: Option<Vec<String>>,
    response_types: Option<Vec<String>>,
    scope: Option<String>,
}
fn valid_redirect(value: &str) -> bool {
    if value.len() > 2048 {
        return false;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && (url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(
                    url.host_str(),
                    Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
                )))
}
async fn register(State(app): State<App>, Json(body): Json<Registration>) -> Result<Response> {
    let scope = body.scope.as_deref().unwrap_or(SCOPE);
    let name = body.client_name.unwrap_or_else(|| "Fundus client".into());
    let method = body
        .token_endpoint_auth_method
        .unwrap_or_else(|| "none".into());
    if name.trim().is_empty()
        || name.len() > 120
        || name.chars().any(char::is_control)
        || body.redirect_uris.is_empty()
        || body.redirect_uris.len() > 10
        || !body.redirect_uris.iter().all(|s| valid_redirect(s))
    {
        return Err(OAuthError::bad(
            "invalid_client_metadata",
            "Provide a client name and HTTPS or loopback redirect addresses without credentials or fragments",
        ));
    }
    if !["none", "client_secret_basic", "client_secret_post"].contains(&method.as_str()) {
        return Err(OAuthError::bad(
            "invalid_client_metadata",
            "Unsupported token authentication method",
        ));
    }
    if body.grant_types.as_ref().is_some_and(|types| {
        !types.iter().any(|s| s == "authorization_code")
            || types
                .iter()
                .any(|s| !["authorization_code", "refresh_token"].contains(&s.as_str()))
    }) || body
        .response_types
        .as_ref()
        .is_some_and(|types| types != &["code"])
        || ![SCOPE, NATIVE_SCOPE].contains(&scope)
    {
        return Err(OAuthError::bad(
            "invalid_client_metadata",
            "Use authorization code, refresh token, and one supported resource scope",
        ));
    }
    let _lock = app.mutations.lock().await;
    sqlx::query!("DELETE FROM oauth_pending WHERE expires_at<unixepoch()")
        .execute(&app.db)
        .await?;
    sqlx::query!("DELETE FROM oauth_codes WHERE expires_at<unixepoch()")
        .execute(&app.db)
        .await?;
    sqlx::query!("DELETE FROM oauth_clients WHERE created_at<unixepoch() - 86400 * 7 AND NOT EXISTS(SELECT 1 FROM oauth_grants WHERE client_id=oauth_clients.id) AND NOT EXISTS(SELECT 1 FROM oauth_pending WHERE client_id=oauth_clients.id) AND NOT EXISTS(SELECT 1 FROM oauth_codes WHERE client_id=oauth_clients.id)").execute(&app.db).await?;
    let count: i64 =
        sqlx::query_scalar!("SELECT count(*) FROM oauth_clients WHERE created_at>unixepoch() - 60")
            .fetch_one(&app.db)
            .await?;
    let total: i64 = sqlx::query_scalar!("SELECT count(*) FROM oauth_clients")
        .fetch_one(&app.db)
        .await?;
    if count >= 20 || total >= 1000 {
        return Err(OAuthError(
            StatusCode::TOO_MANY_REQUESTS,
            "temporarily_unavailable",
            "Client registration is temporarily limited; try again later".into(),
        ));
    }
    let client_id = id();
    let secret = (method != "none").then(auth::secret);
    let trimmed_name = name.trim();
    let redirect_uris_json = serde_json::to_string(&body.redirect_uris).unwrap();
    let secret_hash = secret.as_ref().map(|s| auth::hash(s));
    sqlx::query!("INSERT INTO oauth_clients(id,name,redirect_uris,auth_method,secret_hash,created_at,allowed_scope) VALUES(?,?,?,?,?,unixepoch(),?)", client_id, trimmed_name, redirect_uris_json, method, secret_hash, scope).execute(&app.db).await?;
    let mut value = json!({"client_id":client_id,"client_id_issued_at":seconds(),"client_name":name.trim(),"redirect_uris":body.redirect_uris,"token_endpoint_auth_method":method,"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"scope":scope});
    if let Some(secret) = secret {
        value["client_secret"] = json!(secret);
        value["client_secret_expires_at"] = json!(0);
    }
    let mut r = response(value);
    *r.status_mut() = StatusCode::CREATED;
    Ok(r)
}

#[derive(Deserialize)]
pub struct Authorization {
    client_id: String,
    redirect_uri: String,
    response_type: String,
    code_challenge: String,
    code_challenge_method: String,
    resource: String,
    scope: Option<String>,
    state: Option<String>,
}
fn valid_challenge(value: &str) -> bool {
    value.len() == 43
        && base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(value)
            .is_ok_and(|b| b.len() == 32)
}
/// The browser fetches this only after its existing owner authentication succeeds.
pub async fn authorization(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Query(query): Query<Authorization>,
) -> Result<Response> {
    access.admin().map_err(|_| {
        OAuthError(
            StatusCode::FORBIDDEN,
            "access_denied",
            "Owner access required".into(),
        )
    })?;
    let client = sqlx::query!(
        "SELECT name,redirect_uris,allowed_scope FROM oauth_clients WHERE id=?",
        query.client_id
    )
    .fetch_optional(&app.db)
    .await?
    .ok_or_else(|| {
        OAuthError::bad(
            "invalid_request",
            "Unknown OAuth client; register the client first",
        )
    })?;
    let redirects: Vec<String> =
        serde_json::from_str(&client.redirect_uris).map_err(OAuthError::internal)?;
    if !redirects.contains(&query.redirect_uri) {
        return Err(OAuthError::bad(
            "invalid_request",
            "The callback address is not registered for this client",
        ));
    }
    if query.response_type != "code"
        || query.code_challenge_method != "S256"
        || !valid_challenge(&query.code_challenge)
    {
        return Err(OAuthError::bad(
            "invalid_request",
            "Authorization code and PKCE S256 are required",
        ));
    }
    let (scope, requested_resource) = resource_scope(&app, &query.resource)
        .ok_or_else(|| OAuthError::bad("invalid_target", "Unknown resource"))?;
    let registered_scope: String = client.allowed_scope.clone();
    if registered_scope != scope || query.scope.as_deref().unwrap_or(SCOPE) != scope {
        return Err(OAuthError::bad(
            "invalid_scope",
            "The requested scope must match this registered client and resource",
        ));
    }
    if query.state.as_ref().is_some_and(|s| s.len() > 2048) {
        return Err(OAuthError::bad("invalid_request", "State is too long"));
    }
    let request_id = auth::secret();
    sqlx::query!("DELETE FROM oauth_pending WHERE expires_at<unixepoch()")
        .execute(&app.db)
        .await?;
    let request_hash = auth::hash(&request_id);
    sqlx::query!("INSERT INTO oauth_pending(hash,client_id,redirect_uri,challenge,scope,resource,state,expires_at) VALUES(?,?,?,?,?,?,?,unixepoch() + 600)", request_hash, query.client_id, query.redirect_uri, query.code_challenge, scope, requested_resource, query.state).execute(&app.db).await?;
    Ok(response(
        json!({"request_id":request_id,"client_name":client.name.clone(),"redirect_uri":query.redirect_uri,"scope":scope,"resource":requested_resource}),
    ))
}
#[derive(Deserialize)]
pub struct Consent {
    request_id: String,
    approve: bool,
}
pub async fn consent(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Json(body): Json<Consent>,
) -> Result<Response> {
    access.admin().map_err(|_| {
        OAuthError(
            StatusCode::FORBIDDEN,
            "access_denied",
            "Owner access required".into(),
        )
    })?;
    let _lock = app.mutations.lock().await;
    let mut tx = app.db.begin().await?;
    let request_hash = auth::hash(&body.request_id);
    let row = sqlx::query!(
        "DELETE FROM oauth_pending WHERE hash=? AND expires_at>=unixepoch() RETURNING *",
        request_hash
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        OAuthError::bad(
            "invalid_request",
            "This approval request expired; reconnect from your MCP client",
        )
    })?;
    let mut callback = url::Url::parse(&row.redirect_uri).map_err(OAuthError::internal)?;
    if body.approve {
        let code = auth::secret();
        let code_hash = auth::hash(&code);

        sqlx::query!("INSERT INTO oauth_codes(hash,client_id,redirect_uri,challenge,scope,resource,expires_at) VALUES(?,?,?,?,?,?,unixepoch() + 60)", code_hash, row.client_id, row.redirect_uri, row.challenge, row.scope, row.resource).execute(&mut *tx).await?;
        callback.query_pairs_mut().append_pair("code", &code);
    } else {
        callback
            .query_pairs_mut()
            .append_pair("error", "access_denied");
    }
    if let Some(state) = row.state.clone() {
        callback.query_pairs_mut().append_pair("state", &state);
    }
    callback.query_pairs_mut().append_pair("iss", &issuer(&app));
    tx.commit().await?;
    Ok(response(json!({"redirect_url":callback.to_string()})))
}

#[derive(Deserialize)]
pub struct TokenRequest {
    grant_type: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
    resource: Option<String>,
    scope: Option<String>,
}
fn equal_secret(a: &str, b: &str) -> bool {
    let a = Sha256::digest(a.as_bytes());
    let b = Sha256::digest(b.as_bytes());
    a.iter().zip(b).fold(0u8, |sum, (a, b)| sum | (a ^ b)) == 0
}
async fn authenticate(
    app: &App,
    headers: &HeaderMap,
    client_id: Option<&str>,
    client_secret: Option<&str>,
) -> Result<String> {
    let basic = headers.get(header::AUTHORIZATION).map(|header| {
        let value = header.to_str().ok()?.strip_prefix("Basic ")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(value)
            .ok()?;
        let decoded = String::from_utf8(bytes).ok()?;
        let (id, secret) = decoded.split_once(':')?;
        let decode = |s: &str| {
            url::form_urlencoded::parse(format!("v={s}").as_bytes())
                .next()
                .map(|(_, v)| v.into_owned())
        };
        Some((decode(id)?, decode(secret)?))
    });
    let (client_id, secret, method) = match basic {
        Some(Some((id, secret)))
            if client_id.is_none_or(|value| value == id) && client_secret.is_none() =>
        {
            (id, Some(secret), "client_secret_basic")
        }
        Some(_) => {
            return Err(OAuthError(
                StatusCode::UNAUTHORIZED,
                "invalid_client",
                "Invalid client authentication".into(),
            ));
        }
        None => (
            client_id.unwrap_or("").into(),
            client_secret.map(str::to_string),
            if client_secret.is_some_and(|s| !s.is_empty()) {
                "client_secret_post"
            } else {
                "none"
            },
        ),
    };
    let row = sqlx::query!(
        "SELECT auth_method,secret_hash FROM oauth_clients WHERE id=?",
        client_id
    )
    .fetch_optional(&app.db)
    .await?;
    let valid = row.is_some_and(|row| {
        let expected: String = row.auth_method.clone();
        expected == method
            && (method == "none"
                || secret
                    .as_ref()
                    .zip(row.secret_hash.clone())
                    .is_some_and(|(value, hash)| equal_secret(&auth::hash(value), &hash)))
    });
    if !valid {
        return Err(OAuthError(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "Invalid client authentication".into(),
        ));
    }
    Ok(client_id)
}
async fn issue_pair(
    tx: &mut Transaction<'_, Sqlite>,
    grant_id: &str,
    refresh_expiry: i64,
    scope: &str,
) -> Result<Value> {
    let access = format!("oa_{}", auth::secret());
    let refresh = auth::secret();
    let token_hash = auth::hash(&access);
    sqlx::query!(
        "INSERT INTO oauth_access(hash,grant_id,expires_at) VALUES(?,?,unixepoch() + ?)",
        token_hash,
        grant_id,
        ACCESS_LIFETIME
    )
    .execute(&mut **tx)
    .await?;
    let token_hash = auth::hash(&refresh);
    sqlx::query!(
        "INSERT INTO oauth_refresh(hash,grant_id,expires_at) VALUES(?,?,?)",
        token_hash,
        grant_id,
        refresh_expiry
    )
    .execute(&mut **tx)
    .await?;
    Ok(
        json!({"access_token":access,"token_type":"Bearer","expires_in":ACCESS_LIFETIME,"refresh_token":refresh,"scope":scope}),
    )
}
async fn token(
    State(app): State<App>,
    headers: HeaderMap,
    Form(body): Form<TokenRequest>,
) -> Result<Response> {
    let client = authenticate(
        &app,
        &headers,
        body.client_id.as_deref(),
        body.client_secret.as_deref(),
    )
    .await?;
    let (scope, requested_resource) = body
        .resource
        .as_deref()
        .and_then(|value| resource_scope(&app, value))
        .ok_or_else(|| {
            OAuthError::bad(
                "invalid_target",
                "The resource parameter must identify a supported vault resource",
            )
        })?;
    if body.scope.as_ref().is_some_and(|value| value != scope) {
        return Err(OAuthError::bad(
            "invalid_scope",
            "The scope must match the requested resource",
        ));
    }
    let _lock = app.mutations.lock().await;
    let mut tx = app.db.begin().await?;
    let value = match body.grant_type.as_str() {
        "authorization_code" => {
            let code_hash = auth::hash(body.code.as_deref().unwrap_or(""));
            let row = sqlx::query!("SELECT hash AS \"hash!\", client_id, redirect_uri, challenge, scope, resource, expires_at FROM oauth_codes WHERE hash=?", code_hash)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(OAuthError::grant)?;
            let verifier = body.code_verifier.as_deref().unwrap_or("");
            let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(verifier.as_bytes()));
            if row.client_id.clone() != client
                || row.redirect_uri.clone() != body.redirect_uri.as_deref().unwrap_or("")
                || row.expires_at < seconds()
                || row.resource.clone() != requested_resource
                || row.scope.clone() != scope
                || !(43..=128).contains(&verifier.len())
                || !verifier
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
                || !equal_secret(&challenge, &row.challenge)
            {
                return Err(OAuthError::grant());
            }
            let code_hash = auth::hash(body.code.as_deref().unwrap_or(""));
            sqlx::query!("DELETE FROM oauth_codes WHERE hash=?", code_hash)
                .execute(&mut *tx)
                .await?;
            let grant_id = id();
            sqlx::query!("INSERT INTO oauth_grants(id,client_id,scope,resource,created_at,expires_at) VALUES(?,?,?,?,unixepoch(),unixepoch() + ?)", grant_id, client, scope, requested_resource, REFRESH_LIFETIME).execute(&mut *tx).await?;
            issue_pair(&mut tx, &grant_id, seconds() + REFRESH_LIFETIME, scope).await?
        }
        "refresh_token" => {
            let hash = auth::hash(body.refresh_token.as_deref().unwrap_or(""));
            let row=sqlx::query!("SELECT r.grant_id,r.expires_at,r.used_at,g.client_id,g.scope,g.resource,g.revoked_at,g.expires_at AS grant_expiry FROM oauth_refresh r JOIN oauth_grants g ON g.id=r.grant_id WHERE r.hash=?", hash).fetch_optional(&mut *tx).await?.ok_or_else(OAuthError::grant)?;
            if row.client_id.clone() != client
                || row.resource.clone() != requested_resource
                || row.scope.clone() != scope
                || row.revoked_at.is_some()
                || row.expires_at < seconds()
                || row.grant_expiry < seconds()
            {
                return Err(OAuthError::grant());
            }
            let grant_id: String = row.grant_id.clone();
            if row.used_at.is_some() {
                sqlx::query!(
                    "UPDATE oauth_grants SET revoked_at=unixepoch() WHERE id=?",
                    grant_id
                )
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                return Err(OAuthError::grant());
            }
            sqlx::query!(
                "UPDATE oauth_refresh SET used_at=unixepoch() WHERE hash=?",
                hash
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query!(
                "UPDATE oauth_grants SET expires_at=unixepoch() + ? WHERE id=?",
                REFRESH_LIFETIME,
                grant_id
            )
            .execute(&mut *tx)
            .await?;
            issue_pair(&mut tx, &grant_id, seconds() + REFRESH_LIFETIME, scope).await?
        }
        _ => {
            return Err(OAuthError::bad(
                "unsupported_grant_type",
                "Use authorization_code or refresh_token",
            ));
        }
    };
    tx.commit().await?;
    Ok(response(value))
}
#[derive(Deserialize)]
pub struct Revocation {
    client_id: Option<String>,
    client_secret: Option<String>,
    token: String,
}
async fn revoke_token(
    State(app): State<App>,
    headers: HeaderMap,
    Form(body): Form<Revocation>,
) -> Result<Response> {
    let client = authenticate(
        &app,
        &headers,
        body.client_id.as_deref(),
        body.client_secret.as_deref(),
    )
    .await?;
    let token_hash = auth::hash(&body.token);
    sqlx::query!("UPDATE oauth_grants SET revoked_at=unixepoch() WHERE client_id=? AND id IN(SELECT grant_id FROM oauth_access WHERE hash=? UNION SELECT grant_id FROM oauth_refresh WHERE hash=?)", client, token_hash, token_hash).execute(&app.db).await?;
    Ok(response(json!({})))
}
pub async fn connections(
    State(app): State<App>,
    Extension(access): Extension<Access>,
) -> Result<Response> {
    access.admin().map_err(|_| {
        OAuthError(
            StatusCode::FORBIDDEN,
            "access_denied",
            "Owner access required".into(),
        )
    })?;
    let rows=sqlx::query!("SELECT g.id AS \"id!\",c.name,g.scope,g.created_at,g.expires_at FROM oauth_grants g JOIN oauth_clients c ON c.id=g.client_id WHERE g.revoked_at IS NULL AND g.expires_at>=unixepoch() ORDER BY g.created_at DESC").fetch_all(&app.db).await?;
    Ok(response(
        json!({"connections":rows.iter().map(|r|json!({"id":r.id.clone(),"name":r.name.clone(),"scope":r.scope.clone(),"created_at":r.created_at,"expires_at":r.expires_at})).collect::<Vec<_>>()}),
    ))
}
pub async fn disconnect(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    axum::extract::Path(grant): axum::extract::Path<String>,
) -> Result<Response> {
    access.admin().map_err(|_| {
        OAuthError(
            StatusCode::FORBIDDEN,
            "access_denied",
            "Owner access required".into(),
        )
    })?;
    sqlx::query!(
        "UPDATE oauth_grants SET revoked_at=unixepoch() WHERE id=?",
        grant
    )
    .execute(&app.db)
    .await?;
    Ok(response(json!({"ok":true})))
}
pub async fn require_mcp(State(app): State<App>, mut request: Request, next: Next) -> Response {
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    let had_bearer = bearer.is_some();
    let result = if let Some(token) = bearer.filter(|s| s.starts_with("oa_")) {
        let token_hash = auth::hash(token);
        let row=sqlx::query!("SELECT g.scope,g.resource,g.revoked_at,a.expires_at,g.expires_at AS grant_expiry FROM oauth_access a JOIN oauth_grants g ON g.id=a.grant_id WHERE a.hash=?", token_hash).fetch_optional(&app.db).await;
        match row {
            Ok(Some(row))
                if row.scope.clone() == SCOPE
                    && row.resource.clone() == resource(&app)
                    && row.revoked_at.is_none()
                    && row.expires_at >= seconds()
                    && row.grant_expiry >= seconds() =>
            {
                request.extensions_mut().insert(Access {
                    scope: "read".into(),
                });
                next.run(request).await
            }
            Ok(_) => Error(
                StatusCode::UNAUTHORIZED,
                "OAuth access token is invalid or expired".into(),
            )
            .into_response(),
            Err(error) => Error::from(error).into_response(),
        }
    } else {
        auth::require(State(app.clone()), request, next).await
    };
    let mut result = result;
    if matches!(
        result.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        let error = if result.status() == StatusCode::FORBIDDEN {
            ", error=\"insufficient_scope\""
        } else if had_bearer {
            ", error=\"invalid_token\""
        } else {
            ""
        };
        let value = format!(
            "Bearer resource_metadata=\"{}\", scope=\"{}\"{}",
            endpoint(&app, ".well-known/oauth-protected-resource/mcp"),
            SCOPE,
            error
        );
        result
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value.parse().unwrap());
    }
    result
}

/// The native API is public at the reverse proxy but never trusts upstream
/// browser identity or cookies. Only an explicitly approved native-owner grant
/// for this audience can create owner access here.
pub async fn require_native(State(app): State<App>, mut request: Request, next: Next) -> Response {
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let token = bearer.filter(|value| value.starts_with("oa_") && value.len() <= 256);
    if let Some(token) = token {
        let token_hash = auth::hash(token);
        let grant = sqlx::query!("SELECT g.scope,g.resource,g.revoked_at,a.expires_at,g.expires_at AS grant_expiry FROM oauth_access a JOIN oauth_grants g ON g.id=a.grant_id WHERE a.hash=?", token_hash).fetch_optional(&app.db).await;
        match grant {
            Ok(Some(grant))
                if grant.scope.clone() == NATIVE_SCOPE
                    && grant.resource.clone() == native_resource(&app)
                    && grant.revoked_at.is_none()
                    && grant.expires_at >= seconds()
                    && grant.grant_expiry >= seconds() =>
            {
                request.extensions_mut().insert(Access {
                    scope: "admin".into(),
                });
                return next.run(request).await;
            }
            Err(error) => return Error::from(error).into_response(),
            _ => {}
        }
    }
    let mut response = Error(
        StatusCode::UNAUTHORIZED,
        "Connect this native app to your vault to continue".into(),
    )
    .into_response();
    let invalid = if bearer.is_some() {
        ", error=\"invalid_token\""
    } else {
        ""
    };
    let challenge = format!(
        "Bearer resource_metadata=\"{}\", scope=\"{}\"{}",
        endpoint(&app, ".well-known/oauth-protected-resource/api/native"),
        NATIVE_SCOPE,
        invalid
    );
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, challenge.parse().unwrap());
    no_store(&mut response);
    response
}
