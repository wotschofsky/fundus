//! Proxy mode trusts a browser request as the owner's only with Cloudflare Access's token for it.
mod common;
use axum::{
    Json, Router,
    body::Body,
    http::{Request, StatusCode},
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use common::{ACCESS_AUD, ACCESS_TEAM, access_jwks, access_token, access_token_with, owner_claims};
use fundus::{
    access::Settings,
    api,
    config::{App, Config},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;

async fn proxied(access: Option<Settings>) -> (tempfile::TempDir, App, Router) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::from_env().unwrap();
    config.data = dir.path().into();
    config.public_url = "https://vault.example.com".parse().unwrap();
    config.proxy_auth = true;
    config.access = access;
    let app = App::new(config).await.unwrap();
    let router = api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    (dir, app, router)
}
async fn status(router: &Router, token: Option<&str>) -> StatusCode {
    let mut request = Request::builder().uri("/api/v1/entries");
    if let Some(token) = token {
        request = request.header("Cf-Access-Jwt-Assertion", token);
    }
    let response = router.clone().oneshot(request.body(Body::empty()).unwrap());
    response.await.unwrap().status()
}
fn owner_with(change: Value) -> Value {
    let mut claims = owner_claims();
    for (name, value) in change.as_object().unwrap() {
        claims[name] = value.clone();
    }
    claims
}

#[tokio::test]
async fn only_a_valid_access_sign_in_is_the_owner() {
    let (_dir, app, router) = proxied(Some(Settings::new(ACCESS_TEAM, ACCESS_AUD).unwrap())).await;
    app.access_keys
        .hold(access_jwks().to_string().as_bytes())
        .await
        .unwrap();
    // What slipped past the proxy's sign-in (a path it lets through, however it was spelled)
    // arrives without the token.
    assert_eq!(status(&router, None).await, StatusCode::UNAUTHORIZED);
    let owner = access_token(owner_claims());
    assert_eq!(status(&router, Some(&owner)).await, StatusCode::OK);
    // One audience may come as a plain string.
    let single = access_token(owner_with(json!({"aud": ACCESS_AUD})));
    assert_eq!(status(&router, Some(&single)).await, StatusCode::OK);

    let now = chrono::Utc::now().timestamp();
    let mut without_expiry = owner_claims();
    without_expiry.as_object_mut().unwrap().remove("exp");
    let parts: Vec<&str> = owner.split('.').collect();
    let elsewhere = access_token(owner_with(json!({"aud": ["another-application"]})));
    let elsewhere: Vec<&str> = elsewhere.split('.').collect();
    let encode = |value: Value| URL_SAFE_NO_PAD.encode(value.to_string());
    for (why, token) in [
        (
            "expired",
            access_token(owner_with(json!({"exp": now - 120}))),
        ),
        (
            "not valid yet",
            access_token(owner_with(json!({"nbf": now + 600}))),
        ),
        (
            "for another application",
            access_token(owner_with(json!({"aud": ["another-application"]}))),
        ),
        (
            "from another team",
            access_token(owner_with(
                json!({"iss": "https://other.cloudflareaccess.com"}),
            )),
        ),
        ("without an expiry", access_token(without_expiry)),
        (
            "with its expiry as text",
            access_token(owner_with(json!({"exp": (now + 600).to_string()}))),
        ),
        (
            "signed with a key Access does not list",
            access_token_with(json!({"alg": "RS256", "kid": "other"}), owner_claims()),
        ),
        (
            "unsigned",
            format!(
                "{}.{}.",
                encode(json!({"alg": "none", "kid": "test"})),
                parts[1]
            ),
        ),
        (
            "naming another algorithm",
            access_token_with(json!({"alg": "HS256", "kid": "test"}), owner_claims()),
        ),
        (
            "another application's token relabelled",
            format!("{}.{}.{}", elsewhere[0], parts[1], elsewhere[2]),
        ),
        ("not a token", "not-a-token".into()),
        ("with a part too many", format!("{owner}.x")),
    ] {
        assert_eq!(
            status(&router, Some(&token)).await,
            StatusCode::UNAUTHORIZED,
            "a token {why}"
        );
    }
}

#[tokio::test]
async fn proxy_mode_without_access_settings_trusts_no_one() {
    let (_dir, app, router) = proxied(None).await;
    app.access_keys
        .hold(access_jwks().to_string().as_bytes())
        .await
        .unwrap();
    let owner = access_token(owner_claims());
    assert_eq!(
        status(&router, Some(&owner)).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn access_keys_are_read_from_the_team_when_first_needed_and_not_on_every_unknown_key() {
    let reads = Arc::new(AtomicUsize::new(0));
    let counted = reads.clone();
    let keys = access_jwks();
    let team = Router::new().route(
        "/cdn-cgi/access/certs",
        get(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            let keys = keys.clone();
            async move { Json(keys) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, team).await.unwrap() });
    let (_dir, _app, router) = proxied(Some(Settings::new(&issuer, ACCESS_AUD).unwrap())).await;

    let owner = access_token(owner_with(json!({"iss": issuer})));
    assert_eq!(status(&router, Some(&owner)).await, StatusCode::OK);
    assert_eq!(status(&router, Some(&owner)).await, StatusCode::OK);
    assert_eq!(reads.load(Ordering::SeqCst), 1, "held keys are reused");
    // Made-up key IDs cannot make the server ask the team again and again.
    let unknown = access_token_with(
        json!({"alg": "RS256", "kid": "made-up"}),
        owner_with(json!({"iss": issuer})),
    );
    for _ in 0..3 {
        assert_eq!(
            status(&router, Some(&unknown)).await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_sign_in_that_cannot_be_checked_is_answered_as_unavailable() {
    // The team cannot be reached (nothing listens there) and no keys were ever read.
    let (_dir, _app, router) = proxied(Some(Settings::new(ACCESS_TEAM, ACCESS_AUD).unwrap())).await;
    let owner = access_token(owner_claims());
    assert_eq!(
        status(&router, Some(&owner)).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn a_key_read_finishes_even_when_the_request_that_started_it_goes_away() {
    // The team answers a key read only when released, so the request can be dropped meanwhile.
    let reads = Arc::new(AtomicUsize::new(0));
    let asked = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (counted, started, held_back) = (reads.clone(), asked.clone(), release.clone());
    let keys = access_jwks();
    let team = Router::new().route(
        "/cdn-cgi/access/certs",
        get(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            let (keys, started, held_back) = (keys.clone(), started.clone(), held_back.clone());
            async move {
                started.notify_one();
                held_back.notified().await;
                Json(keys)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, team).await.unwrap() });
    let (_dir, _app, router) = proxied(Some(Settings::new(&issuer, ACCESS_AUD).unwrap())).await;

    // Anyone can start the read with a made-up key ID; then their client goes away.
    let made_up = access_token_with(
        json!({"alg": "RS256", "kid": "made-up"}),
        owner_with(json!({"iss": issuer})),
    );
    let dropped = {
        let router = router.clone();
        tokio::spawn(async move { status(&router, Some(&made_up)).await })
    };
    // Bounded, so a read that never reaches the team fails the test instead of hanging it.
    tokio::time::timeout(std::time::Duration::from_secs(10), asked.notified())
        .await
        .expect("the key read never reached the team");
    dropped.abort();
    let _ = dropped.await;
    release.notify_one();

    // The read still finished: the owner is let in, and the keys were read only once.
    let owner = access_token(owner_with(json!({"iss": issuer})));
    assert_eq!(status(&router, Some(&owner)).await, StatusCode::OK);
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn other_sites_cannot_make_the_browser_ask_as_the_owner() {
    let (_dir, app, router) = proxied(Some(Settings::new(ACCESS_TEAM, ACCESS_AUD).unwrap())).await;
    app.access_keys
        .hold(access_jwks().to_string().as_bytes())
        .await
        .unwrap();
    let owner = access_token(owner_claims());
    // A page elsewhere can time a search it cannot read; another subdomain is no different.
    for (site, expected) in [
        ("cross-site", StatusCode::FORBIDDEN),
        ("same-site", StatusCode::FORBIDDEN),
        ("same-origin", StatusCode::OK),
        ("none", StatusCode::OK),
    ] {
        let request = Request::builder()
            .uri("/api/v1/search?q=word")
            .header("Cf-Access-Jwt-Assertion", &owner)
            .header("Sec-Fetch-Site", site)
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), expected, "{site}");
    }
}
