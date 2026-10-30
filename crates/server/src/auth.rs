use crate::{
    config::{App, now},
    error::{Error, Result},
};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};

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
    pub fn upload(&self) -> Result<()> {
        if ["session", "admin", "upload"].contains(&self.scope.as_str()) {
            Ok(())
        } else {
            Err(Error(
                StatusCode::FORBIDDEN,
                "Upload access required".into(),
            ))
        }
    }
    pub fn read(&self) -> Result<()> {
        if self.scope != "upload" {
            Ok(())
        } else {
            Err(Error(StatusCode::FORBIDDEN, "Read access required".into()))
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
pub async fn issue(app: &App, name: &str, scope: &str) -> Result<String> {
    let value = secret();
    let expires = if scope == "session" {
        Some(
            (chrono::Utc::now() + chrono::Duration::days(7))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )
    } else {
        None
    };
    let token_hash = hash(&value);
    let timestamp = now();
    sqlx::query!(
        "INSERT INTO credentials(hash,name,scope,created_at,expires_at) VALUES(?,?,?,?,?)",
        token_hash,
        name,
        scope,
        timestamp,
        expires
    )
    .execute(&app.db)
    .await?;
    Ok(value)
}
fn check_origin(app: &App, request: &Request) -> Result<()> {
    if !["GET", "HEAD", "OPTIONS"].contains(&request.method().as_str()) {
        let origin = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok());
        let expected = app.config.public_url.origin().ascii_serialization();
        if origin != Some(expected.as_str()) {
            return Err(Error(StatusCode::FORBIDDEN, "Origin check failed".into()));
        }
    }
    Ok(())
}

/// Applied only to browser routes, never to the public upload/MCP routers.
pub async fn require_browser(State(app): State<App>, mut request: Request, next: Next) -> Response {
    if app.config.proxy_auth && !request.headers().contains_key(header::AUTHORIZATION) {
        if let Err(error) = check_origin(&app, &request) {
            return error.into_response();
        }
        request.extensions_mut().insert(Access {
            scope: "admin".into(),
        });
        return next.run(request).await;
    }
    // Explicit credentials retain their scopes; an invalid token never becomes an owner.
    require(State(app), request, next).await
}

pub async fn require(State(app): State<App>, mut request: Request, next: Next) -> Response {
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    let cookie = request
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            s.split(';')
                .find_map(|s| s.trim().strip_prefix("vault_session="))
        });
    let cookie = if app.config.proxy_auth { None } else { cookie };
    let value = match bearer.or(cookie) {
        Some(s) if s.len() <= 256 => s,
        _ => return Error(StatusCode::UNAUTHORIZED, "Sign in to continue".into()).into_response(),
    };
    // Cookie-authenticated mutations require same-origin browser requests. Bearer clients are not subject to CSRF.
    if bearer.is_none()
        && let Err(error) = check_origin(&app, &request)
    {
        return error.into_response();
    }
    let token_hash = hash(value);
    let row = sqlx::query!(
        "SELECT scope,expires_at FROM credentials WHERE hash=?",
        token_hash
    )
    .fetch_optional(&app.db)
    .await;
    match row {
        Ok(Some(row)) => {
            let expiry: Option<String> = row.expires_at.clone();
            if expiry.is_some_and(|s| s < now()) {
                return Error(StatusCode::UNAUTHORIZED, "Session expired".into()).into_response();
            }
            request.extensions_mut().insert(Access {
                scope: row.scope.clone(),
            });
            next.run(request).await
        }
        Ok(None) => Error(
            StatusCode::UNAUTHORIZED,
            "Credential invalid or revoked".into(),
        )
        .into_response(),
        Err(e) => Error::from(e).into_response(),
    }
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
    let token = issue(&app, "Web session", "session").await?;
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
