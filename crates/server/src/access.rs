//! Cloudflare Access sign-in, for `VAULT_AUTH_MODE=proxy`. A browser request is the owner's only
//! when it carries a valid `Cf-Access-Jwt-Assertion` (and no `Authorization` header, `auth.rs`):
//! the token Access adds to every request it has signed in. A request that reached the server another way (a path the proxy lets through
//! without sign-in, however that path was spelled on its way in, or a direct hit on the origin)
//! carries none of its own. The token is a bearer credential, at most until it expires (plus
//! `LEEWAY`): whoever holds the owner's is let in, wherever the request comes from.
use crate::{
    config::App,
    error::{Error, Result},
};
use axum::http::StatusCode;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
use serde::Deserialize;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub const HEADER: &str = "cf-access-jwt-assertion";
/// Clock skew allowed on a token's expiry and start.
const LEEWAY: i64 = 60;
/// Keys are read again after this, and when a token names one not held (Access rotates them).
const REFRESH: Duration = Duration::from_secs(3600);
/// At most one read of the keys this often, however many unknown key IDs arrive.
const RETRY: Duration = Duration::from_secs(30);

/// Which Access application's sign-ins count as the owner's.
#[derive(Clone, Debug)]
pub struct Settings {
    /// The team's origin, `https://<team>.cloudflareaccess.com`: the tokens' issuer, whose
    /// `/cdn-cgi/access/certs` lists the keys they are signed with.
    pub issuer: String,
    /// The Access application's Audience (AUD) tag: tokens for the team's other applications
    /// name theirs instead.
    pub audience: String,
}
/// What `fundus serve` says when proxy mode has no Access settings.
pub const MISSING: &str = "VAULT_AUTH_MODE=proxy needs VAULT_ACCESS_TEAM_DOMAIN \
     (<team>.cloudflareaccess.com) and VAULT_ACCESS_AUD (the Access application's Audience tag)";
impl Settings {
    /// From `VAULT_ACCESS_TEAM_DOMAIN` and `VAULT_ACCESS_AUD`; none when neither is set, so
    /// commands that never check a sign-in (`token`, `backup`) run without them.
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let team = std::env::var("VAULT_ACCESS_TEAM_DOMAIN").unwrap_or_default();
        let audience = std::env::var("VAULT_ACCESS_AUD").unwrap_or_default();
        if team.trim().is_empty() && audience.trim().is_empty() {
            return Ok(None);
        }
        Self::new(&team, &audience).map(Some)
    }
    /// `team` is the team domain (`<team>.cloudflareaccess.com`), or an origin with its scheme.
    pub fn new(team: &str, audience: &str) -> anyhow::Result<Self> {
        let (team, audience) = (team.trim().trim_end_matches('/'), audience.trim());
        anyhow::ensure!(!team.is_empty() && !audience.is_empty(), MISSING);
        let origin = if team.contains("://") {
            team.to_string()
        } else {
            format!("https://{team}")
        };
        let url = url::Url::parse(&origin)?;
        anyhow::ensure!(
            ["http", "https"].contains(&url.scheme())
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none(),
            "VAULT_ACCESS_TEAM_DOMAIN must be a domain such as <team>.cloudflareaccess.com"
        );
        Ok(Self {
            issuer: url.origin().ascii_serialization(),
            audience: audience.into(),
        })
    }
}

/// RSA signing keys (modulus, exponent) by key ID.
type KeySet = HashMap<String, (Vec<u8>, Vec<u8>)>;

/// The team's signing keys, as last read.
#[derive(Default)]
pub struct Keys(std::sync::Arc<tokio::sync::Mutex<Held>>);
#[derive(Default)]
struct Held {
    keys: KeySet,
    read: Option<Instant>,
    asked: Option<Instant>,
}
#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}
#[derive(Deserialize)]
struct Jwk {
    kid: Option<String>,
    kty: String,
    n: Option<String>,
    e: Option<String>,
}
impl Keys {
    /// Holds the keys of a key set (the JSON at `/cdn-cgi/access/certs`) as freshly read.
    pub async fn hold(&self, jwks: &[u8]) -> anyhow::Result<()> {
        let keys = parse(jwks)?;
        let mut held = self.0.lock().await;
        held.keys = keys;
        held.read = Some(Instant::now());
        Ok(())
    }
    async fn get(&self, app: &App, settings: &Settings, kid: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut held = self.0.clone().lock_owned().await;
        let stale = held.read.is_none_or(|t| t.elapsed() > REFRESH);
        if (stale || !held.keys.contains_key(kid)) && held.asked.is_none_or(|t| t.elapsed() > RETRY)
        {
            held.asked = Some(Instant::now());
            let (client, settings) = (app.client.clone(), settings.clone());
            // Read in a task of its own, holding the keys: a request dropped meanwhile (its client
            // gone) must not abandon the read, leaving it asked for but never read until RETRY.
            let read = tokio::spawn(async move {
                match fetch(&client, &settings).await {
                    Ok(keys) => {
                        held.keys = keys;
                        held.read = Some(Instant::now());
                    }
                    // Keys already held stay in use: Access keeps a replaced key valid for days.
                    Err(error) => tracing::warn!(%error, "could not read the Access signing keys"),
                }
                held
            });
            held = read.await.map_err(|_| unavailable())?;
        }
        match held.keys.get(kid) {
            Some(key) => Ok(key.clone()),
            None if held.read.is_none() => Err(unavailable()),
            None => Err(denied()),
        }
    }
}
fn parse(jwks: &[u8]) -> anyhow::Result<KeySet> {
    let set: Jwks = serde_json::from_slice(jwks)?;
    let mut keys = HashMap::new();
    for key in set.keys {
        if let (Some(kid), "RSA", Some(n), Some(e)) = (key.kid, key.kty.as_str(), key.n, key.e) {
            keys.insert(
                kid,
                (URL_SAFE_NO_PAD.decode(n)?, URL_SAFE_NO_PAD.decode(e)?),
            );
        }
    }
    Ok(keys)
}
fn unavailable() -> Error {
    Error(
        StatusCode::SERVICE_UNAVAILABLE,
        "Could not check your sign-in with Cloudflare Access; try again shortly".into(),
    )
}
async fn fetch(client: &reqwest::Client, settings: &Settings) -> anyhow::Result<KeySet> {
    let body = client
        .get(format!("{}/cdn-cgi/access/certs", settings.issuer))
        .timeout(Duration::from_secs(10))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    parse(&body)
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    kid: String,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}
#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: Audience,
    exp: i64,
    nbf: Option<i64>,
}
fn denied() -> Error {
    Error(StatusCode::UNAUTHORIZED, "Sign in to continue".into())
}
fn decode<T: serde::de::DeserializeOwned>(part: &str) -> Result<T> {
    let bytes = URL_SAFE_NO_PAD.decode(part).map_err(|_| denied())?;
    serde_json::from_slice(&bytes).map_err(|_| denied())
}

/// Accepts `token` only if the configured Access application signed it in and it is still valid.
pub async fn verify(app: &App, token: &str) -> Result<()> {
    // Proxy mode without Access settings (`fundus serve` refuses to start so) trusts no one.
    let Some(settings) = &app.config.access else {
        return Err(denied());
    };
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(denied());
    };
    let head: Header = decode(header)?;
    // Only what Access signs with: a token cannot pick a weaker algorithm (or none).
    if head.alg != "RS256" {
        return Err(denied());
    }
    let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| denied())?;
    let (n, e) = app.access_keys.get(app, settings, &head.kid).await?;
    let signed = &token[..header.len() + 1 + payload.len()];
    RsaPublicKeyComponents { n: &n, e: &e }
        .verify(&RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &signature)
        .map_err(|_| denied())?;
    let claims: Claims = decode(payload)?;
    let now = chrono::Utc::now().timestamp();
    let audience = match &claims.aud {
        Audience::One(one) => one == &settings.audience,
        Audience::Many(many) => many.contains(&settings.audience),
    };
    if claims.iss != settings.issuer
        || !audience
        || claims.exp <= now - LEEWAY
        || claims.nbf.is_some_and(|nbf| nbf > now + LEEWAY)
    {
        return Err(denied());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn the_team_domain_becomes_the_issuer() {
        let issuer = |team: &str| Settings::new(team, "aud").map(|s| s.issuer);
        assert_eq!(
            issuer("team.cloudflareaccess.com").unwrap(),
            "https://team.cloudflareaccess.com"
        );
        assert_eq!(
            issuer(" https://team.cloudflareaccess.com/ ").unwrap(),
            "https://team.cloudflareaccess.com"
        );
        assert_eq!(
            issuer("http://provider:3119").unwrap(),
            "http://provider:3119"
        );
        assert!(issuer("").is_err());
        assert!(issuer("team.cloudflareaccess.com/path").is_err());
        assert!(issuer("ftp://team.cloudflareaccess.com").is_err());
        assert!(Settings::new("team.cloudflareaccess.com", " ").is_err());
    }
}
