use crate::{
    config::{App, now},
    error::{Error, Result},
};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Who a request comes from: the owner ("session" in a password-mode browser, "admin" behind
/// Cloudflare Access or in a native app) or an MCP client's read grant ("read").
#[derive(Clone)]
pub struct Access {
    pub scope: String,
}
impl Access {
    pub fn admin(&self) -> Result<()> {
        if ["session", "admin"].contains(&self.scope.as_str()) {
            Ok(())
        } else {
            Err(Error(StatusCode::FORBIDDEN, "Owner access required".into()))
        }
    }
}
pub fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
pub fn secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
/// Signs a password-mode browser in for seven days; the cookie's value.
pub async fn session(app: &App) -> Result<String> {
    let value = secret();
    let expires = (chrono::Utc::now() + chrono::Duration::days(7))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let token_hash = hash(&value);
    let timestamp = now();
    sqlx::query!(
        "INSERT INTO credentials(hash,name,scope,created_at,expires_at) VALUES(?,'Web session','session',?,?)",
        token_hash,
        timestamp,
        expires
    )
    .execute(&app.db)
    .await?;
    Ok(value)
}
/// For a request the browser's own sign-in (its cookie, or Access's token) lets in.
fn check_origin(app: &App, headers: &HeaderMap, method: &Method) -> Result<()> {
    // Any site can make the browser send one and time the answer without reading it (does a
    // search find this word?). Only this site's pages and what the owner opens directly may ask;
    // browsers too old to say which send nothing.
    let site = headers.get("sec-fetch-site").map(|v| v.as_bytes());
    if site.is_some_and(|site| site != b"same-origin" && site != b"none") {
        return Err(Error(
            StatusCode::FORBIDDEN,
            "Requests from other sites are refused".into(),
        ));
    }
    if !["GET", "HEAD", "OPTIONS"].contains(&method.as_str()) {
        let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
        let expected = app.config.public_url.origin().ascii_serialization();
        if origin != Some(expected.as_str()) {
            return Err(Error(StatusCode::FORBIDDEN, "Origin check failed".into()));
        }
    }
    Ok(())
}

/// Applied only to browser routes. The owner's browser signs in with Cloudflare Access's token in
/// proxy mode (`access.rs`) and with its session cookie otherwise.
pub async fn require_browser(State(app): State<App>, mut request: Request, next: Next) -> Response {
    match browser(&app, request.headers(), request.method()).await {
        Ok(scope) => {
            request.extensions_mut().insert(Access {
                scope: scope.into(),
            });
            next.run(request).await
        }
        Err(error) => error.into_response(),
    }
}

async fn browser(app: &App, headers: &HeaderMap, method: &Method) -> Result<&'static str> {
    let signed_out = || Error(StatusCode::UNAUTHORIZED, "Sign in to continue".into());
    // Native apps and MCP clients have routes of their own. A bearer credential here is no
    // browser's sign-in, whatever else the request carries, so it never stands in for the owner.
    if headers.contains_key(header::AUTHORIZATION) {
        return Err(Error(
            StatusCode::UNAUTHORIZED,
            "Browser routes take no Authorization header".into(),
        ));
    }
    if app.config.proxy_auth {
        let token = headers
            .get(crate::access::HEADER)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(signed_out)?;
        crate::access::verify(app, token).await?;
        check_origin(app, headers, method)?;
        return Ok("admin");
    }
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            s.split(';')
                .find_map(|s| s.trim().strip_prefix("vault_session="))
        })
        .filter(|s| s.len() <= 256)
        .ok_or_else(signed_out)?;
    check_origin(app, headers, method)?;
    let token_hash = hash(cookie);
    let expires = sqlx::query_scalar!(
        "SELECT expires_at FROM credentials WHERE hash=? AND scope='session'",
        token_hash
    )
    .fetch_optional(&app.db)
    .await?
    .ok_or_else(|| {
        Error(
            StatusCode::UNAUTHORIZED,
            "Session invalid or signed out".into(),
        )
    })?;
    if expires.is_none_or(|s| s < now()) {
        return Err(Error(StatusCode::UNAUTHORIZED, "Session expired".into()));
    }
    Ok("session")
}
#[derive(Deserialize)]
pub struct Login {
    password: String,
}
pub async fn login(
    State(app): State<App>,
    headers: HeaderMap,
    Json(body): Json<Login>,
) -> Result<Response> {
    if app.config.proxy_auth {
        return Err(Error(
            StatusCode::FORBIDDEN,
            "Password login is disabled; authentication is handled by the reverse proxy".into(),
        ));
    }
    if body.password.len() > 1024 {
        return Err(Error::bad("Invalid password"));
    }
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        && origin != app.config.public_url.origin().ascii_serialization()
    {
        return Err(Error(StatusCode::FORBIDDEN, "Origin check failed".into()));
    }
    {
        let mut attempts = app.login_attempts.lock().await;
        attempts.retain(|t| t.elapsed().as_secs() < 60);
        if attempts.len() >= 10 {
            return Err(Error(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many sign-in attempts. Wait a minute.".into(),
            ));
        }
        attempts.push(std::time::Instant::now());
    }
    let encoded: Option<String> = sqlx::query_scalar!("SELECT password_hash FROM owner WHERE id=1")
        .fetch_optional(&app.db)
        .await?;
    let encoded = encoded.ok_or_else(|| {
        Error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Initialize the owner account with the server init command".into(),
        )
    })?;
    let valid = tokio::task::spawn_blocking(move || {
        PasswordHash::new(&encoded).ok().is_some_and(|h| {
            Argon2::default()
                .verify_password(body.password.as_bytes(), &h)
                .is_ok()
        })
    })
    .await
    .unwrap_or(false);
    if !valid {
        return Err(Error(StatusCode::UNAUTHORIZED, "Incorrect password".into()));
    }
    let token = session(&app).await?;
    let cookie = format!(
        "vault_session={token}; HttpOnly; Path=/; SameSite=Strict; Max-Age=604800{}",
        if app.config.secure_cookie {
            "; Secure"
        } else {
            ""
        }
    );
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({"ok":true})),
    )
        .into_response())
}
pub async fn init_owner(app: &App, password: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        password.len() >= 12,
        "Use a password with at least 12 characters"
    );
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    let encoded = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .to_string();
    sqlx::query!("INSERT INTO owner(id,password_hash) VALUES(1,?)", encoded)
        .execute(&app.db)
        .await?;
    Ok(())
}
