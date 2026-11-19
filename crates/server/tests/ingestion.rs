mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use fundus::{
    auth,
    config::{App, Config},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
async fn setup() -> (tempfile::TempDir, App, axum::Router, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Config::from_env().unwrap();
    c.data = dir.path().to_path_buf();
    c.public_url = "http://localhost:3000".parse().unwrap();
    c.openai_key = "synthetic-only".into();
    c.openai_url = "http://127.0.0.1:9".into();
    let app = App::new(c).await.unwrap();
    auth::init_owner(&app, "synthetic-test-password")
        .await
        .unwrap();
    // A native app's owner grant, and a chat app's MCP read grant.
    let owner = common::grant(&app, "vault:owner").await;
    let read = common::grant(&app, "mcp:read").await;
    let r = fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    (dir, app, r, owner, read)
}
async fn call(
    r: &axum::Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    let response = r
        .clone()
        .oneshot(b.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
/// The status of a request from the vault's own pages, with `headers`.
async fn browser(
    r: &axum::Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> StatusCode {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("Origin", "http://localhost:3000");
    for (name, value) in headers {
        b = b.header(*name, *value);
    }
    r.clone()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}
#[tokio::test]
async fn a_session_lasts_until_sign_out_or_expiry() {
    let (_dir, app, r, _, _) = setup().await;
    assert_eq!(
        browser(&r, "GET", "/api/v1/entries", &[]).await,
        StatusCode::UNAUTHORIZED
    );
    let expiring = format!("vault_session={}", auth::session(&app).await.unwrap());
    assert_eq!(
        browser(&r, "GET", "/api/v1/entries", &[("Cookie", &expiring)]).await,
        StatusCode::OK
    );
    sqlx::query("UPDATE credentials SET expires_at='2000-01-01T00:00:00.000Z'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        browser(&r, "GET", "/api/v1/entries", &[("Cookie", &expiring)]).await,
        StatusCode::UNAUTHORIZED
    );
    let cookie = format!("vault_session={}", auth::session(&app).await.unwrap());
    assert_eq!(
        browser(&r, "POST", "/api/v1/logout", &[("Cookie", &cookie)]).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        browser(&r, "GET", "/api/v1/entries", &[("Cookie", &cookie)]).await,
        StatusCode::UNAUTHORIZED
    );
}
/// Static tokens are gone: a row one left behind opens nothing, and a browser route answers any
/// `Authorization` header with 401, even next to the owner's own session.
#[tokio::test]
async fn leftover_tokens_and_authorization_headers_open_nothing() {
    let (_dir, app, r, owner, read) = setup().await;
    let session = format!("vault_session={}", auth::session(&app).await.unwrap());
    for scope in ["upload", "read", "admin"] {
        let token = common::leftover_token(&app, scope).await;
        let cookie = format!("vault_session={token}");
        let bearer = format!("Bearer {token}");
        assert_eq!(
            browser(&r, "GET", "/api/v1/entries", &[("Cookie", &cookie)]).await,
            StatusCode::UNAUTHORIZED,
            "{scope} as a session"
        );
        for path in ["/api/v1/entries", "/api/native/v1/entries"] {
            assert_eq!(
                call(&r, "GET", path, Some(&token), json!(null)).await.0,
                StatusCode::UNAUTHORIZED,
                "{scope} on {path}"
            );
        }
        let (status, _) = call(&r, "POST", "/mcp", Some(&token), json!({})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{scope} on MCP");
        assert_eq!(
            browser(
                &r,
                "GET",
                "/api/v1/entries",
                &[("Cookie", &session), ("Authorization", &bearer)]
            )
            .await,
            StatusCode::UNAUTHORIZED,
            "{scope} beside a session"
        );
    }
    for authorization in [
        format!("Bearer {owner}"),
        format!("Bearer {read}"),
        "Bearer invalid".into(),
        "Basic b3duZXI6cGFzc3dvcmQ=".into(),
    ] {
        for (method, path) in [("GET", "/api/v1/me"), ("POST", "/api/web/v1/imports")] {
            assert_eq!(
                browser(
                    &r,
                    method,
                    path,
                    &[("Cookie", &session), ("Authorization", &authorization)]
                )
                .await,
                StatusCode::UNAUTHORIZED,
                "{authorization} on {path}"
            );
        }
    }
    // MCP takes no session either.
    assert_eq!(
        browser(&r, "POST", "/mcp", &[("Cookie", &session)]).await,
        StatusCode::UNAUTHORIZED
    );
    // Nothing makes a token any more.
    for (method, path) in [("GET", "/api/v1/tokens"), ("POST", "/api/v1/tokens")] {
        assert_eq!(
            browser(&r, method, path, &[("Cookie", &session)]).await,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        browser(&r, "GET", "/api/v1/me", &[("Cookie", &session)]).await,
        StatusCode::OK
    );
}
#[tokio::test]
async fn tus_offsets_survive_restart_and_finalization_is_idempotent() {
    let (_dir, app, r, u, _) = setup().await;
    let (_, batch) = call(
        &r,
        "POST",
        "/api/native/v1/imports",
        Some(&u),
        json!({"recorder_timezone":"UTC"}),
    )
    .await;
    let batch = batch["id"].as_str().unwrap();
    use base64::Engine;
    let encode = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
    let request = Request::builder()
        .method("POST")
        .uri("/api/native/v1/uploads")
        .header("Authorization", format!("Bearer {u}"))
        .header("Tus-Resumable", "1.0.0")
        .header("Upload-Length", "6")
        .header(
            "Upload-Metadata",
            format!(
                "import_id {},filename {}",
                encode(batch),
                encode("synthetic.wav")
            ),
        )
        .body(Body::empty())
        .unwrap();
    let response = r.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let path = response.headers()["location"].to_str().unwrap().to_string();
    assert!(path.starts_with("/api/native/v1/uploads/"), "{path}");
    for (offset, data, expected) in [
        (0, "abc", StatusCode::NO_CONTENT),
        (0, "def", StatusCode::CONFLICT),
        (3, "def", StatusCode::NO_CONTENT),
    ] {
        let response = r
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(&path)
                    .header("Authorization", format!("Bearer {u}"))
                    .header("Tus-Resumable", "1.0.0")
                    .header("Upload-Offset", offset)
                    .header("Content-Type", "application/offset+octet-stream")
                    .body(Body::from(data))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let reopened = App::new((*app.config).clone()).await.unwrap();
    let rr = fundus::api::router(reopened, tokio_util::sync::CancellationToken::new());
    let response = rr
        .oneshot(
            Request::builder()
                .method("HEAD")
                .uri(&path)
                .header("Authorization", format!("Bearer {u}"))
                .header("Tus-Resumable", "1.0.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["upload-offset"], "6");
    for _ in 0..2 {
        assert_eq!(
            call(
                &r,
                "POST",
                &format!("/api/native/v1/imports/{batch}/complete"),
                Some(&u),
                json!({})
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM imports")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
#[tokio::test]
async fn browser_mutations_require_same_origin() {
    let (_dir, _, r, _, _) = setup().await;
    let response = r
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/login")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({"password":"synthetic-test-password"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let response = r
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/people")
                .header("Cookie", cookie)
                .header("Content-Type", "application/json")
                .header("Origin", "https://foreign.example")
                .body(Body::from(json!({"name":"Synthetic person"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
#[tokio::test]
async fn mcp_lists_real_read_only_tools() {
    let (_dir, _, r, _, read) = setup().await;
    let mut session = None;
    for (method, params) in [
        (
            "initialize",
            json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
        ),
        ("tools/list", json!({})),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("Host", "localhost")
            .header("Authorization", format!("Bearer {read}"))
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json");
        if let Some(s) = &session {
            request = request.header("Mcp-Session-Id", s);
        }
        let response = r
            .clone()
            .oneshot(
                request
                    .body(Body::from(
                        json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .or(session);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&bytes);
        if method == "tools/list" {
            assert!(text.contains("read_transcript"), "{text}");
            assert!(text.contains("list_labels"));
            assert!(text.contains("readOnlyHint"));
        }
    }
}

#[tokio::test]
async fn numeric_search_filters_deserialize() {
    let (_d, _app, r, owner, _) = setup().await;
    let (status, _) = call(
        &r,
        "GET",
        "/api/native/v1/search?q=synthetic&limit=2&cursor=0",
        Some(&owner),
        json!(null),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn proxy_auth_trusts_only_browser_routes() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::from_env().unwrap();
    config.data = dir.path().into();
    config.public_url = "https://vault.example.com".parse().unwrap();
    common::proxy(&mut config);
    let app = App::new(config).await.unwrap();
    // Proxy mode has no initialized owner account or browser session.
    let owner = common::grant(&app, "vault:owner").await;
    let read = common::grant(&app, "mcp:read").await;
    let leftover = common::leftover_token(&app, "admin").await;
    let session = auth::session(&app).await.unwrap();
    let r = common::signed_in(
        &app,
        fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new()),
    );
    let (status, me) = call(&r, "GET", "/api/v1/me", None, json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["auth_mode"], "proxy");
    assert_eq!(me["scope"], "admin");
    // Access's sign-in is for browser routes only.
    for (method, path) in [
        ("POST", "/mcp"),
        ("POST", "/api/native/v1/imports"),
        ("OPTIONS", "/api/native/v1/uploads"),
        ("HEAD", "/api/native/v1/uploads/missing"),
        ("GET", "/api/native/v1/imports/missing"),
    ] {
        assert_eq!(
            call(&r, method, path, None, json!({})).await.0,
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
    }
    assert_eq!(
        call(
            &r,
            "OPTIONS",
            "/api/native/v1/uploads",
            Some(&owner),
            json!(null)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    // A request with its own credential is not taken for the owner's browser, though Access's
    // token comes along: no grant, leftover token or session counts there.
    for token in [owner.as_str(), &read, &leftover, &session, "invalid"] {
        assert_eq!(
            call(&r, "GET", "/api/v1/me", Some(token), json!(null))
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        call(&r, "POST", "/api/login", None, json!({"password":"unused"}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    for origin in [
        None,
        Some("https://attacker.example"),
        Some("https://vault.example.com"),
    ] {
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/v1/people")
            .header("Content-Type", "application/json");
        if let Some(origin) = origin {
            req = req.header("Origin", origin);
        }
        let response = r
            .clone()
            .oneshot(
                req.body(Body::from(r#"{"name":"Synthetic person"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if origin == Some("https://vault.example.com") {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            }
        );
    }
    // A session cookie means nothing in proxy mode, and nothing to the native routes.
    let response = r
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/native/v1/imports")
                .header("Cookie", format!("vault_session={session}"))
                .header("Origin", "https://vault.example.com")
                .header("Content-Type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_web_library_uploads_with_its_proxy_session_and_same_origin_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::from_env().unwrap();
    config.data = dir.path().into();
    config.public_url = "https://vault.example.com".parse().unwrap();
    common::proxy(&mut config);
    let app = App::new(config).await.unwrap();
    let r = common::signed_in(
        &app,
        fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new()),
    );
    let send = |method: &str, uri: &str, origin: &str, body: Body| {
        let r = r.clone();
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("Origin", origin)
            .header("Content-Type", "application/json")
            .header("Tus-Resumable", "1.0.0")
            .body(body)
            .unwrap();
        async move { r.oneshot(request).await.unwrap() }
    };
    let own = "https://vault.example.com";
    let response = send(
        "POST",
        "/api/web/v1/imports",
        "https://attacker.example",
        "{}".into(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = send("POST", "/api/web/v1/imports", own, "{}".into()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let import = serde_json::from_slice::<Value>(&bytes).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    use base64::Engine;
    let encode = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
    let request = Request::builder()
        .method("POST")
        .uri("/api/web/v1/uploads")
        .header("Origin", own)
        .header("Tus-Resumable", "1.0.0")
        .header("Upload-Length", "3")
        .header(
            "Upload-Metadata",
            format!(
                "import_id {},filename {}",
                encode(&import),
                encode("memo.m4a")
            ),
        )
        .body(Body::empty())
        .unwrap();
    let response = r.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    // The upload continues where it was created, behind the proxy, not on the native route.
    let location = response.headers()["location"].to_str().unwrap().to_string();
    assert!(location.starts_with("/api/web/v1/uploads/"), "{location}");
    let request = Request::builder()
        .method("PATCH")
        .uri(&location)
        .header("Origin", own)
        .header("Tus-Resumable", "1.0.0")
        .header("Upload-Offset", "0")
        .header("Content-Type", "application/offset+octet-stream")
        .body(Body::from("abc"))
        .unwrap();
    assert_eq!(
        r.clone().oneshot(request).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    let path = format!("/api/web/v1/imports/{import}/complete");
    let response = send("POST", &path, own, Body::empty()).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn uploads_stop_short_of_the_disk_reserve() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    let config = |reserve: u64| {
        let mut config = Config::from_env().unwrap();
        config.data = dir.path().into();
        config.reserve = reserve;
        config
    };
    let roomy = App::new(config(0)).await.unwrap();
    let token = common::grant(&roomy, "vault:owner").await;
    // The same vault once its disk is full: the reserve is more than the free space.
    let full = App::new(config(u64::MAX / 2)).await.unwrap();
    let router =
        |app: &App| fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    let (status, import) = call(
        &router(&roomy),
        "POST",
        "/api/native/v1/imports",
        Some(&token),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let import = import["id"].as_str().unwrap().to_string();
    let encode = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
    let send =
        |app: &App, method: &str, uri: &str, headers: &[(&str, String)], body: &'static str| {
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .header("Authorization", format!("Bearer {token}"))
                .header("Tus-Resumable", "1.0.0");
            for (key, value) in headers {
                request = request.header(*key, value);
            }
            let router = router(app);
            let request = request.body(Body::from(body)).unwrap();
            async move { router.oneshot(request).await.unwrap() }
        };
    let create = [
        ("Upload-Length", "3".to_string()),
        (
            "Upload-Metadata",
            format!(
                "import_id {},filename {}",
                encode(&import),
                encode("memo.m4a")
            ),
        ),
    ];
    // Created while there was room, then the disk filled: no chunk is written below the reserve.
    let created = send(&roomy, "POST", "/api/native/v1/uploads", &create, "").await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let location = created.headers()["location"].to_str().unwrap().to_string();
    let chunk = [
        ("Upload-Offset", "0".to_string()),
        (
            "Content-Type",
            "application/offset+octet-stream".to_string(),
        ),
    ];
    let patched = send(&full, "PATCH", &location, &chunk, "abc").await;
    assert_eq!(patched.status(), StatusCode::INSUFFICIENT_STORAGE);
    let head = send(&roomy, "HEAD", &location, &[], "").await;
    assert_eq!(head.headers()["upload-offset"], "0");
    // And no new upload starts.
    let refused = send(&full, "POST", "/api/native/v1/uploads", &create, "").await;
    assert_eq!(refused.status(), StatusCode::INSUFFICIENT_STORAGE);
    // An interrupted chunk leaves bytes past the offset. They are the upload's own, so the next
    // chunk gets their space back instead of being refused for it.
    let id = location.rsplit('/').next().unwrap();
    let mut file = std::fs::File::create(roomy.upload_path(id)).unwrap();
    std::io::Write::write_all(&mut file, &vec![1u8; 64 * 1024 * 1024]).unwrap();
    file.sync_all().unwrap();
    let free = fundus::config::free_space(dir.path()).unwrap();
    let tight = App::new(config(free + 32 * 1024 * 1024)).await.unwrap();
    let patched = send(&tight, "PATCH", &location, &chunk, "abc").await;
    assert_eq!(patched.status(), StatusCode::NO_CONTENT);
}
