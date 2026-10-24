use axum::{
    Json, Router,
    body::Body,
    extract::Extension,
    http::{Request, StatusCode},
    middleware,
    routing::get,
};
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use transcript_vault::{
    api,
    auth::{self, Access},
    config::{App, Config},
    oauth,
};
const ORIGIN: &str = "https://vault.example.com";
const MCP: &str = "https://vault.example.com/mcp";
const NATIVE: &str = "https://vault.example.com/api/native";
const CALLBACK: &str = "http://127.0.0.1:57231/callback";
const VERIFIER: &str = "a-native-client-verifier-longer-than-forty-three-characters";
async fn setup() -> (tempfile::TempDir, App, Router) {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Config::from_env().unwrap();
    c.data = dir.path().into();
    c.public_url = ORIGIN.parse().unwrap();
    c.proxy_auth = true;
    let app = App::new(c).await.unwrap();
    let probe = Router::new()
        .route(
            "/api/native/v1/probe",
            get(|Extension(a): Extension<Access>| async move {
                a.admin().unwrap();
                a.read().unwrap();
                a.upload().unwrap();
                Json(json!({"scope":a.scope}))
            }),
        )
        .route_layer(middleware::from_fn_with_state(
            app.clone(),
            oauth::require_native,
        ))
        .with_state(app.clone());
    let router = api::router(app.clone(), tokio_util::sync::CancellationToken::new()).merge(probe);
    (dir, app, router)
}
async fn call(
    r: &Router,
    method: &str,
    path: &str,
    body: Option<String>,
    ctype: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "vault.example.com");
    if body.is_some() {
        req = req.header("Content-Type", ctype);
    }
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let response = r
        .clone()
        .oneshot(req.body(Body::from(body.unwrap_or_default())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn form(fields: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish()
}
async fn register(r: &Router, scope: &str) -> String {
    let(s,v)=call(r,"POST","/oauth/register",Some(json!({"client_name":"Native test","redirect_uris":[CALLBACK],"token_endpoint_auth_method":"none","scope":scope}).to_string()),"application/json",&[]).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(v["scope"], scope);
    v["client_id"].as_str().unwrap().into()
}
async fn pending(r: &Router, cid: &str, scope: &str, resource: &str) -> (StatusCode, Value) {
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(VERIFIER.as_bytes()));
    let q = form(&[
        ("client_id", cid),
        ("redirect_uri", CALLBACK),
        ("response_type", "code"),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("scope", scope),
        ("resource", resource),
        ("state", "native-state"),
    ]);
    call(
        r,
        "GET",
        &format!("/api/v1/oauth/authorize?{q}"),
        None,
        "",
        &[],
    )
    .await
}
async fn grant(r: &Router, cid: &str, scope: &str, resource: &str) -> Value {
    let (s, p) = pending(r, cid, scope, resource).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(p["scope"], scope);
    assert_eq!(p["resource"], resource);
    let (s, v) = call(
        r,
        "POST",
        "/api/v1/oauth/authorize",
        Some(json!({"request_id":p["request_id"],"approve":true}).to_string()),
        "application/json",
        &[("Origin", ORIGIN)],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let url = url::Url::parse(v["redirect_url"].as_str().unwrap()).unwrap();
    let code = url
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    let (s, pair) = call(
        r,
        "POST",
        "/oauth/token",
        Some(form(&[
            ("grant_type", "authorization_code"),
            ("client_id", cid),
            ("redirect_uri", CALLBACK),
            ("code", &code),
            ("code_verifier", VERIFIER),
            ("resource", resource),
        ])),
        "application/x-www-form-urlencoded",
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(pair["scope"], scope);
    pair
}
#[tokio::test]
async fn native_owner_requires_explicit_correct_scope_and_never_trusts_proxy_or_cookies() {
    let (_dir, app, r) = setup().await;
    assert_eq!(
        call(&r, "GET", "/api/native/v1/probe", None, "", &[])
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let session = auth::issue(&app, "old session", "session").await.unwrap();
    let cookie = format!("vault_session={session}");
    assert_eq!(
        call(
            &r,
            "GET",
            "/api/native/v1/probe",
            None,
            "",
            &[("Cookie", &cookie)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    for scope in ["upload", "read", "admin"] {
        let token = auth::issue(&app, "legacy", scope).await.unwrap();
        let bearer = format!("Bearer {token}");
        assert_eq!(
            call(
                &r,
                "GET",
                "/api/native/v1/probe",
                None,
                "",
                &[("Authorization", &bearer)]
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let cid = register(&r, "vault:owner").await;
    let pair = grant(&r, &cid, "vault:owner", NATIVE).await;
    let bearer = format!("Bearer {}", pair["access_token"].as_str().unwrap());
    let (s, v) = call(
        &r,
        "GET",
        "/api/native/v1/probe",
        None,
        "",
        &[("Authorization", &bearer)],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["scope"], "admin");
    assert_eq!(
        call(
            &r,
            "POST",
            "/mcp",
            Some("{}".into()),
            "application/json",
            &[
                ("Authorization", &bearer),
                ("Accept", "application/json, text/event-stream")
            ]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &r,
            "GET",
            "/api/v1/entries",
            None,
            "",
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn registered_mcp_clients_and_mcp_grants_cannot_elevate_to_native_owner() {
    let (_dir, _app, r) = setup().await;
    let mcp = register(&r, "mcp:read").await;
    assert_eq!(
        pending(&r, &mcp, "vault:owner", NATIVE).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        pending(&r, &mcp, "mcp:read", NATIVE).await.0,
        StatusCode::BAD_REQUEST
    );
    let pair = grant(&r, &mcp, "mcp:read", MCP).await;
    let bearer = format!("Bearer {}", pair["access_token"].as_str().unwrap());
    assert_eq!(
        call(
            &r,
            "GET",
            "/api/native/v1/probe",
            None,
            "",
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (s, v) = call(
        &r,
        "POST",
        "/oauth/token",
        Some(form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &mcp),
            ("refresh_token", pair["refresh_token"].as_str().unwrap()),
            ("resource", NATIVE),
            ("scope", "vault:owner"),
        ])),
        "application/x-www-form-urlencoded",
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"], "invalid_grant");
    let native = register(&r, "vault:owner").await;
    assert_eq!(
        pending(&r, &native, "mcp:read", MCP).await.0,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn native_discovery_refresh_rotation_and_revocation_preserve_audience() {
    let (_dir, _app, r) = setup().await;
    let (s, v) = call(
        &r,
        "GET",
        "/.well-known/oauth-protected-resource/api/native",
        None,
        "",
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["resource"], NATIVE);
    assert_eq!(v["scopes_supported"], json!(["vault:owner"]));
    let cid = register(&r, "vault:owner").await;
    let pair = grant(&r, &cid, "vault:owner", NATIVE).await;
    let (s, next) = call(
        &r,
        "POST",
        "/oauth/token",
        Some(form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &cid),
            ("refresh_token", pair["refresh_token"].as_str().unwrap()),
            ("resource", NATIVE),
        ])),
        "application/x-www-form-urlencoded",
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(next["scope"], "vault:owner");
    assert_ne!(next["refresh_token"], pair["refresh_token"]);
    let (s, _) = call(
        &r,
        "POST",
        "/oauth/revoke",
        Some(form(&[
            ("client_id", &cid),
            ("token", next["refresh_token"].as_str().unwrap()),
        ])),
        "application/x-www-form-urlencoded",
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let bearer = format!("Bearer {}", next["access_token"].as_str().unwrap());
    assert_eq!(
        call(
            &r,
            "GET",
            "/api/native/v1/probe",
            None,
            "",
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn native_owner_alias_supports_crud_uploads_and_excludes_credential_management() {
    let (_dir, _app, r) = setup().await;
    assert_eq!(
        call(&r, "GET", "/api/native/v1/me", None, "", &[]).await.0,
        StatusCode::UNAUTHORIZED
    );
    let cid = register(&r, "vault:owner").await;
    let pair = grant(&r, &cid, "vault:owner", NATIVE).await;
    let bearer = format!("Bearer {}", pair["access_token"].as_str().unwrap());
    let h = [("Authorization", bearer.as_str())];
    let (status, me) = call(&r, "GET", "/api/native/v1/me", None, "", &h).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["scope"], "admin");
    let (status, person) = call(
        &r,
        "POST",
        "/api/native/v1/people",
        Some(json!({"name":"Synthetic Person"}).to_string()),
        "application/json",
        &h,
    )
    .await;
    assert!(status.is_success());
    let person_path = format!("/api/native/v1/people/{}", person["id"].as_str().unwrap());
    assert_eq!(
        call(
            &r,
            "PATCH",
            &person_path,
            Some(json!({"name":"Updated Synthetic"}).to_string()),
            "application/json",
            &h
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&r, "DELETE", &person_path, None, "", &h).await.0,
        StatusCode::NO_CONTENT
    );
    let (status, batch) = call(
        &r,
        "POST",
        "/api/native/v1/imports",
        Some(json!({"recorder_timezone":"UTC"}).to_string()),
        "application/json",
        &h,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(batch["state"], "uploading");
    for path in [
        "/api/native/v1/tokens",
        "/api/native/v1/oauth/authorize",
        "/api/native/v1/logout",
    ] {
        assert_eq!(
            call(&r, "GET", path, None, "", &h).await.0,
            StatusCode::NOT_FOUND
        );
    }
}
