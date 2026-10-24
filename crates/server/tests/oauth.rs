use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use transcript_vault::{
    api,
    config::{App, Config},
};
const ORIGIN: &str = "https://vault.example.com";
const RESOURCE: &str = "https://vault.example.com/mcp";
const REDIRECT: &str = "https://chat.example.com/callback";
const VERIFIER: &str = "a-long-and-private-pkce-verifier-which-is-over-forty-three-characters";
async fn setup() -> (tempfile::TempDir, App, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::from_env().unwrap();
    config.data = dir.path().into();
    config.public_url = ORIGIN.parse().unwrap();
    config.proxy_auth = true;
    let app = App::new(config).await.unwrap();
    let router = api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    (dir, app, router)
}
async fn call(
    router: &axum::Router,
    method: &str,
    path: &str,
    body: Option<String>,
    content_type: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "vault.example.com");
    if body.is_some() {
        request = request.header("Content-Type", content_type);
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(body.unwrap_or_default())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn form(values: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(values.iter().copied())
        .finish()
}
async fn json_call(
    r: &axum::Router,
    path: &str,
    value: Value,
    origin: Option<&str>,
) -> (StatusCode, HeaderMap, Value) {
    let headers = origin.map(|v| vec![("Origin", v)]).unwrap_or_default();
    call(
        r,
        "POST",
        path,
        Some(value.to_string()),
        "application/json",
        &headers,
    )
    .await
}
async fn register(r: &axum::Router) -> String {
    let (status,_,value)=json_call(r,"/oauth/register",json!({"client_name":"Chat client","redirect_uris":[REDIRECT],"token_endpoint_auth_method":"none"}),None).await;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    value["client_id"].as_str().unwrap().into()
}
async fn pending(r: &axum::Router, client: &str) -> Value {
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(VERIFIER.as_bytes()));
    let query = form(&[
        ("client_id", client),
        ("redirect_uri", REDIRECT),
        ("response_type", "code"),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("scope", "mcp:read"),
        ("state", "preserved state"),
        ("resource", RESOURCE),
    ]);
    let (status, _, value) = call(
        r,
        "GET",
        &format!("/api/v1/oauth/authorize?{query}"),
        None,
        "",
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value
}
async fn approve(r: &axum::Router, client: &str) -> String {
    let pending = pending(r, client).await;
    let (status, _, value) = json_call(
        r,
        "/api/v1/oauth/authorize",
        json!({"request_id":pending["request_id"],"approve":true}),
        Some(ORIGIN),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let url = url::Url::parse(value["redirect_url"].as_str().unwrap()).unwrap();
    assert_eq!(
        url.query_pairs().find(|(k, _)| k == "state").unwrap().1,
        "preserved state"
    );
    assert_eq!(
        url.query_pairs().find(|(k, _)| k == "iss").unwrap().1,
        ORIGIN
    );
    url.query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned()
}
async fn redeem(
    r: &axum::Router,
    client: &str,
    code: &str,
    verifier: &str,
    resource: &str,
) -> (StatusCode, HeaderMap, Value) {
    call(
        r,
        "POST",
        "/oauth/token",
        Some(form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client),
            ("code", code),
            ("redirect_uri", REDIRECT),
            ("code_verifier", verifier),
            ("resource", resource),
        ])),
        "application/x-www-form-urlencoded",
        &[],
    )
    .await
}
async fn refresh(r: &axum::Router, client: &str, token: &str) -> (StatusCode, HeaderMap, Value) {
    call(
        r,
        "POST",
        "/oauth/token",
        Some(form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client),
            ("refresh_token", token),
            ("resource", RESOURCE),
        ])),
        "application/x-www-form-urlencoded",
        &[],
    )
    .await
}
#[tokio::test]
async fn discovery_and_complete_pkce_flow_are_mcp_only_and_revocable() {
    let (_dir, app, r) = setup().await;
    let (status, headers, _) = call(
        &r,
        "POST",
        "/mcp",
        Some("{}".into()),
        "application/json",
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        headers["www-authenticate"]
            .to_str()
            .unwrap()
            .contains("oauth-protected-resource/mcp")
    );
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let (status, headers, body) = call(&r, "GET", path, None, "", &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["resource"], RESOURCE);
        assert_eq!(body["authorization_servers"][0], ORIGIN);
        assert_eq!(headers["access-control-allow-origin"], "*");
    }
    let (_, _, metadata) = call(
        &r,
        "GET",
        "/.well-known/oauth-authorization-server",
        None,
        "",
        &[],
    )
    .await;
    assert_eq!(
        metadata["code_challenge_methods_supported"],
        json!(["S256"])
    );
    assert_eq!(
        metadata["authorization_response_iss_parameter_supported"],
        true
    );
    let client = register(&r).await;
    let code = approve(&r, &client).await;
    let (status, headers, tokens) = redeem(&r, &client, &code, VERIFIER, RESOURCE).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    assert_eq!(headers["cache-control"], "no-store");
    let access = tokens["access_token"].as_str().unwrap();
    let auth = format!("Bearer {access}");
    let (status,_,_)=call(&r,"POST","/mcp",Some(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}).to_string()),"application/json",&[("Authorization",&auth),("Accept","application/json, text/event-stream")]).await;
    assert_eq!(status, StatusCode::OK);
    for path in ["/api/v1/me", "/api/v1/tokens", "/api/v1/entries"] {
        assert_eq!(
            call(&r, "GET", path, None, "", &[("Authorization", &auth)])
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        json_call(&r, "/api/v1/imports", json!({}), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &r,
            "POST",
            "/api/v1/imports",
            Some("{}".into()),
            "application/json",
            &[("Authorization", &auth)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        redeem(&r, &client, &code, VERIFIER, RESOURCE).await.0,
        StatusCode::BAD_REQUEST
    );
    let (_, _, connections) = call(&r, "GET", "/api/v1/oauth/connections", None, "", &[]).await;
    let grant = connections["connections"][0]["id"].as_str().unwrap();
    let status = call(
        &r,
        "DELETE",
        &format!("/api/v1/oauth/connections/{grant}"),
        None,
        "",
        &[("Origin", ORIGIN)],
    )
    .await
    .0;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call(
            &r,
            "POST",
            "/mcp",
            Some("{}".into()),
            "application/json",
            &[("Authorization", &auth)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        refresh(&r, &client, tokens["refresh_token"].as_str().unwrap())
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let stored: String = sqlx::query_scalar("SELECT hash FROM oauth_access LIMIT 1")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_ne!(stored, access);
}
#[tokio::test]
async fn invalid_verifier_resource_client_and_redirect_cannot_redeem_codes() {
    let (_dir, _app, r) = setup().await;
    let client = register(&r).await;
    let other = register(&r).await;
    let code = approve(&r, &client).await;
    assert_eq!(
        redeem(&r, &client, &code, &"x".repeat(64), RESOURCE)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        redeem(&r, &other, &code, VERIFIER, RESOURCE).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        redeem(&r, &client, &code, VERIFIER, "https://other.example/mcp")
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        redeem(&r, &client, &code, VERIFIER, RESOURCE).await.0,
        StatusCode::OK
    );
    let bad = form(&[
        ("client_id", &client),
        ("redirect_uri", "https://attacker.example/callback"),
        ("response_type", "code"),
        ("code_challenge", &"a".repeat(43)),
        ("code_challenge_method", "S256"),
        ("resource", RESOURCE),
    ]);
    let (status, headers, _) = call(
        &r,
        "GET",
        &format!("/api/v1/oauth/authorize?{bad}"),
        None,
        "",
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!headers.contains_key("location"));
}
#[tokio::test]
async fn consent_requires_same_origin_and_explicit_approval() {
    let (_dir, app, r) = setup().await;
    let client = register(&r).await;
    let pending = pending(&r, &client).await;
    let body = json!({"request_id":pending["request_id"],"approve":true});
    assert_eq!(
        json_call(&r, "/api/v1/oauth/authorize", body.clone(), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        json_call(
            &r,
            "/api/v1/oauth/authorize",
            body,
            Some("https://other.example")
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_codes")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let (status, _, value) = json_call(
        &r,
        "/api/v1/oauth/authorize",
        json!({"request_id":pending["request_id"],"approve":false}),
        Some(ORIGIN),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        value["redirect_url"]
            .as_str()
            .unwrap()
            .contains("error=access_denied")
    );
    assert_eq!(
        json_call(
            &r,
            "/api/v1/oauth/authorize",
            json!({"request_id":pending["request_id"],"approve":true}),
            Some(ORIGIN)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn refresh_rotation_detects_reuse_and_revokes_the_family() {
    let (_dir, _app, r) = setup().await;
    let client = register(&r).await;
    let code = approve(&r, &client).await;
    let (_, _, first) = redeem(&r, &client, &code, VERIFIER, RESOURCE).await;
    let old = first["refresh_token"].as_str().unwrap();
    let (status, _, second) = refresh(&r, &client, old).await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(first["refresh_token"], second["refresh_token"]);
    assert_eq!(refresh(&r, &client, old).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        refresh(&r, &client, second["refresh_token"].as_str().unwrap())
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let auth = format!("Bearer {}", second["access_token"].as_str().unwrap());
    assert_eq!(
        call(
            &r,
            "POST",
            "/mcp",
            Some("{}".into()),
            "application/json",
            &[("Authorization", &auth)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn registration_rejects_unsafe_callbacks_and_limits_public_bodies() {
    let (_dir, _app, r) = setup().await;
    for redirect in [
        "javascript:alert(1)",
        "http://chat.example/callback",
        "https://name:pass@chat.example/callback",
        "https://chat.example/callback#fragment",
    ] {
        assert_eq!(
            json_call(
                &r,
                "/oauth/register",
                json!({"redirect_uris":[redirect]}),
                None
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        json_call(
            &r,
            "/oauth/register",
            json!({"redirect_uris":["http://127.0.0.1:4389/callback"]}),
            None
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        json_call(
            &r,
            "/oauth/register",
            json!({"client_name":"x".repeat(20_000),"redirect_uris":[REDIRECT]}),
            None
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    for path in ["/oauth/register", "/oauth/token", "/oauth/revoke"] {
        let (status, headers, _) = call(&r, "OPTIONS", path, None, "", &[]).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(headers["access-control-allow-origin"], "*");
    }
}

#[tokio::test]
async fn confidential_clients_authenticate_and_revoke_without_copying_user_tokens() {
    let (_dir, _app, r) = setup().await;
    for method in ["client_secret_basic", "client_secret_post"] {
        let (status,_,registration)=json_call(&r,"/oauth/register",json!({"client_name":"Confidential client","redirect_uris":[REDIRECT],"token_endpoint_auth_method":method}),None).await;
        assert_eq!(status, StatusCode::CREATED);
        let client = registration["client_id"].as_str().unwrap();
        let secret = registration["client_secret"].as_str().unwrap();
        let code = approve(&r, client).await;
        let basic = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{client}:{secret}"))
        );
        let mut values = vec![
            ("grant_type", "authorization_code"),
            ("client_id", client),
            ("code", &code),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
            ("resource", RESOURCE),
        ];
        let headers = if method == "client_secret_basic" {
            vec![("Authorization", basic.as_str())]
        } else {
            values.push(("client_secret", secret));
            vec![]
        };
        let (status, _, tokens) = call(
            &r,
            "POST",
            "/oauth/token",
            Some(form(&values)),
            "application/x-www-form-urlencoded",
            &headers,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{tokens}");
        let mut revoke = vec![
            ("client_id", client),
            ("token", tokens["refresh_token"].as_str().unwrap()),
        ];
        if method == "client_secret_post" {
            revoke.push(("client_secret", secret));
        }
        assert_eq!(
            call(
                &r,
                "POST",
                "/oauth/revoke",
                Some(form(&revoke)),
                "application/x-www-form-urlencoded",
                &headers
            )
            .await
            .0,
            StatusCode::OK
        );
        let auth = format!("Bearer {}", tokens["access_token"].as_str().unwrap());
        assert_eq!(
            call(
                &r,
                "POST",
                "/mcp",
                Some("{}".into()),
                "application/json",
                &[("Authorization", &auth)]
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn expired_codes_and_tokens_are_rejected_but_access_expiry_allows_refresh() {
    let (_dir, app, r) = setup().await;
    let client = register(&r).await;
    let code = approve(&r, &client).await;
    sqlx::query("UPDATE oauth_codes SET expires_at=0")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        redeem(&r, &client, &code, VERIFIER, RESOURCE).await.0,
        StatusCode::BAD_REQUEST
    );
    let code = approve(&r, &client).await;
    let (_, _, tokens) = redeem(&r, &client, &code, VERIFIER, RESOURCE).await;
    sqlx::query("UPDATE oauth_access SET expires_at=0")
        .execute(&app.db)
        .await
        .unwrap();
    let auth = format!("Bearer {}", tokens["access_token"].as_str().unwrap());
    assert_eq!(
        call(
            &r,
            "POST",
            "/mcp",
            Some("{}".into()),
            "application/json",
            &[("Authorization", &auth)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, _, rotated) =
        refresh(&r, &client, tokens["refresh_token"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    sqlx::query("UPDATE oauth_refresh SET expires_at=0")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        refresh(&r, &client, rotated["refresh_token"].as_str().unwrap())
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
}
