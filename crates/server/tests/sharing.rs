//! Share links: a link opens one recording read-only, without signing in, until it expires, the
//! owner stops sharing, or the recording goes to Trash.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use fundus::{
    api, auth,
    config::{App, Config},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const ORIGIN: &str = "https://vault.example.com";
const ID: &str = "cedar";

struct Vault {
    _dir: tempfile::TempDir,
    app: App,
    router: Router,
    owner: String,
    reader: String,
}

async fn setup() -> Vault {
    let dir = tempfile::tempdir().unwrap();
    let web = dir.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "page").unwrap();
    let mut c = Config::from_env().unwrap();
    c.data = dir.path().join("data");
    c.web = web;
    c.public_url = format!("{ORIGIN}/").parse().unwrap();
    c.proxy_auth = false;
    let app = App::new(c).await.unwrap();
    let owner = auth::issue(&app, "Web session", "session").await.unwrap();
    let reader = auth::issue(&app, "test reader", "read").await.unwrap();
    for sql in [
        // With a converted video.
        "INSERT INTO entries(id,fingerprint,title,duration_ms,timezone,stage,summary,audio_path,video_status,video_width,video_height,created_at,updated_at) VALUES('cedar','cedar','Cedar launch',12000,'UTC','complete','{A} plans the launch with {B}.','playback.m4a','ready',1280,720,'2026-11-18T10:00:00.000Z','2026-11-18T10:00:00.000Z')",
        // Another recording, never shared: no link may read it.
        "INSERT INTO entries(id,fingerprint,title,duration_ms,timezone,stage,created_at,updated_at) VALUES('other','other','Other meeting',1000,'UTC','complete','2026-11-18T10:00:00.000Z','2026-11-18T10:00:00.000Z')",
        "INSERT INTO segments(id,entry_id,position,speaker,start_ms,end_ms,text) VALUES('other:0','other',0,'A',0,1000,'Not for the link.')",
        "INSERT INTO people(id,name) VALUES('morgan','Morgan')",
        "INSERT INTO speakers(entry_id,label,person_id,state,evidence) VALUES('cedar','A','morgan','manual','[{\"source\":\"manual\"}]'),('cedar','B',NULL,'unknown','[]')",
        "INSERT INTO segments(id,entry_id,position,speaker,start_ms,end_ms,text) VALUES('cedar:0','cedar',0,'A',0,5000,'The cedar launch is on Tuesday.'),('cedar:1','cedar',1,'B',5200,8000,'Great, I will tell the team.')",
        "INSERT INTO labels(name) VALUES('work')",
        "INSERT INTO entry_labels(entry_id,label) VALUES('cedar','work')",
    ] {
        sqlx::query(sql).execute(&app.db).await.unwrap();
    }
    std::fs::create_dir_all(app.entry_dir(ID)).unwrap();
    std::fs::write(app.entry_dir(ID).join("playback.m4a"), "0123456789").unwrap();
    std::fs::write(app.entry_dir(ID).join("video.mp4"), "videobytes").unwrap();
    let router = api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    Vault {
        _dir: dir,
        app,
        router,
        owner,
        reader,
    }
}

async fn call(
    v: &Vault,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(token) = bearer {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    if body.is_some() {
        req = req.header("Content-Type", "application/json");
    }
    for (k, value) in headers {
        req = req.header(*k, *value);
    }
    let body = Body::from(body.map(|b| b.to_string()).unwrap_or_default());
    let response = v
        .router
        .clone()
        .oneshot(req.body(body).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes().to_vec();
    (parts.status, parts.headers, bytes)
}
fn json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or(Value::Null)
}
/// Shares the recording as the owner and returns the link's path (`/s/{token}`).
async fn share(v: &Vault, expires: &str) -> String {
    let (status, _, body) = call(
        v,
        "PUT",
        "/api/v1/entries/cedar/share",
        Some(&v.owner),
        &[],
        Some(json!({"expires": expires})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let url = json(&body)["url"].as_str().unwrap().to_string();
    url.strip_prefix(ORIGIN).unwrap().to_string()
}
/// The status of an anonymous GET.
async fn anonymous(v: &Vault, path: &str) -> StatusCode {
    call(v, "GET", path, None, &[], None).await.0
}
fn private(headers: &HeaderMap) {
    assert_eq!(headers["cache-control"], "no-store");
    assert_eq!(headers["x-robots-tag"], "noindex");
}

#[tokio::test]
async fn a_link_opens_one_recording_without_signing_in() {
    let v = setup().await;
    let link = share(&v, "30d").await;
    assert_eq!(link.len(), "/s/".len() + 43);

    // The page is the web app, kept out of caches and search engines.
    let (status, headers, body) = call(&v, "GET", &link, None, &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"page");
    private(&headers);

    // The recording as the page shows it, and nothing else of the owner's.
    let (status, headers, body) = call(&v, "GET", &format!("{link}/entry"), None, &[], None).await;
    assert_eq!(status, StatusCode::OK);
    private(&headers);
    let entry = json(&body);
    let mut keys: Vec<_> = entry.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "audio_available",
            "duration_ms",
            "recorded_at",
            "speakers",
            "stage",
            "summary",
            "summary_manual",
            "title",
            "video"
        ]
    );
    assert_eq!(entry["summary"], "Morgan plans the launch with Speaker 2.");
    assert_eq!(
        entry["speakers"],
        json!([{"label":"A","name":"Morgan"},{"label":"B","name":null}])
    );
    assert_eq!(
        entry["video"],
        json!({"state":"ready","width":1280,"height":720})
    );

    // The transcript, with positions for ids; a query cannot pick another recording.
    let (status, headers, body) = call(
        &v,
        "GET",
        &format!("{link}/transcript?entry_id=other&cursor=0&limit=2000"),
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    private(&headers);
    let page = json(&body);
    assert_eq!(page["next_cursor"], Value::Null);
    assert_eq!(
        page["segments"][0],
        json!({"id":"0","position":0,"speaker":"A","speaker_name":"Morgan","start_ms":0,"end_ms":5000,"text":"The cedar launch is on Tuesday."})
    );
    assert_eq!(page["segments"].as_array().unwrap().len(), 2);
    assert!(!String::from_utf8_lossy(&body).contains("Not for the link"));

    // Audio and video seek by range, and HEAD answers too; none of it is cached.
    let audio = format!("{link}/audio");
    let (status, headers, body) =
        call(&v, "GET", &audio, None, &[("Range", "bytes=2-5")], None).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, b"2345");
    private(&headers);
    let (status, headers, _) = call(&v, "HEAD", &audio, None, &[], None).await;
    assert_eq!(status, StatusCode::OK);
    private(&headers);
    let video = format!("{link}/video");
    let (status, headers, body) =
        call(&v, "GET", &video, None, &[("Range", "bytes=0-4")], None).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, b"video");
    assert_eq!(headers["content-type"], "video/mp4");
    private(&headers);
    // A failed conversion shows as failed, without its error (FFmpeg's, with server paths).
    sqlx::query("UPDATE entries SET video_status='failed',video_failure='ffmpeg: /data/uploads/x' WHERE id='cedar'")
        .execute(&v.app.db)
        .await
        .unwrap();
    let (_, _, body) = call(&v, "GET", &format!("{link}/entry"), None, &[], None).await;
    assert_eq!(
        json(&body)["video"],
        json!({"state":"failed","width":1280,"height":720})
    );
    assert_eq!(anonymous(&v, &video).await, StatusCode::NOT_FOUND);
    sqlx::query("UPDATE entries SET video_status='ready',video_failure=NULL WHERE id='cedar'")
        .execute(&v.app.db)
        .await
        .unwrap();

    // Nothing else under /s answers, and every miss carries the same headers. (The page itself
    // loads for any token; it says so when its link does not work.)
    for path in [
        format!("{link}/"),
        format!("{link}/typo"),
        format!("{link}/entry/x"),
        "/s".into(),
        format!("/s/{}/entry", "x".repeat(43)),
        "/s/short/entry".into(),
    ] {
        let (status, headers, _) = call(&v, "GET", &path, None, &[], None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        private(&headers);
    }
    assert_eq!(anonymous(&v, "/s/").await, StatusCode::NOT_FOUND);

    // The link is no credential for the owner's API.
    let token = link.trim_start_matches("/s/");
    for bearer in [None, Some(token)] {
        let (status, _, _) = call(&v, "GET", "/api/v1/entries", bearer, &[], None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _, _) =
            call(&v, "GET", "/api/v1/entries/cedar/audio", bearer, &[], None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn only_the_owner_manages_links() {
    let v = setup().await;
    let link = share(&v, "7d").await;
    // Sharing again while the link works keeps its URL, whatever the new expiry.
    assert_eq!(share(&v, "never").await, link);
    let (_, _, body) = call(
        &v,
        "GET",
        "/api/v1/entries/cedar/share",
        Some(&v.owner),
        &[],
        None,
    )
    .await;
    assert_eq!(json(&body)["expires_at"], Value::Null);
    assert_eq!(json(&body)["url"], format!("{ORIGIN}{link}"));
    let (_, _, body) = call(&v, "GET", "/api/v1/shares", Some(&v.owner), &[], None).await;
    assert_eq!(json(&body)["shares"][0]["entry_id"], ID);
    assert_eq!(json(&body)["shares"][0]["title"], "Cedar launch");

    // A read token reads recordings, but neither sees nor changes links.
    for (method, path, body) in [
        ("GET", "/api/v1/entries/cedar/share", None),
        (
            "PUT",
            "/api/v1/entries/cedar/share",
            Some(json!({"expires":"1d"})),
        ),
        ("DELETE", "/api/v1/entries/cedar/share", None),
        ("GET", "/api/v1/shares", None),
    ] {
        let (status, _, _) = call(&v, method, path, Some(&v.reader), &[], body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
    }
    let (status, _, _) = call(
        &v,
        "PUT",
        "/api/v1/entries/cedar/share",
        Some(&v.owner),
        &[],
        Some(json!({"expires":"forever"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = call(
        &v,
        "PUT",
        "/api/v1/entries/missing/share",
        Some(&v.owner),
        &[],
        Some(json!({"expires":"1d"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Stop sharing: the link stops at once, and sharing again makes a new one.
    let (status, _, _) = call(
        &v,
        "DELETE",
        "/api/v1/entries/cedar/share",
        Some(&v.owner),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        anonymous(&v, &format!("{link}/entry")).await,
        StatusCode::NOT_FOUND
    );
    // A player still open on it gets nothing more: seeks and HEAD are refused alike.
    for (method, media, range) in [
        ("GET", "audio", Some("bytes=2-5")),
        ("GET", "video", Some("bytes=0-4")),
        ("HEAD", "audio", None),
    ] {
        let headers: Vec<_> = range.map(|r| ("Range", r)).into_iter().collect();
        let (status, response, _) =
            call(&v, method, &format!("{link}/{media}"), None, &headers, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {media}");
        private(&response);
    }
    let (_, _, body) = call(
        &v,
        "GET",
        "/api/v1/entries/cedar/share",
        Some(&v.owner),
        &[],
        None,
    )
    .await;
    assert_eq!(json(&body), Value::Null);
    let (_, _, body) = call(&v, "GET", "/api/v1/shares", Some(&v.owner), &[], None).await;
    assert_eq!(json(&body)["shares"], json!([]));
    assert_ne!(share(&v, "1d").await, link);
}

#[tokio::test]
async fn expired_and_trashed_links_stay_closed() {
    let v = setup().await;
    let link = share(&v, "1d").await;
    sqlx::query("UPDATE shares SET expires_at='2026-01-01T00:00:00.000Z'")
        .execute(&v.app.db)
        .await
        .unwrap();
    assert_eq!(
        anonymous(&v, &format!("{link}/entry")).await,
        StatusCode::NOT_FOUND
    );
    let (_, _, body) = call(
        &v,
        "GET",
        "/api/v1/entries/cedar/share",
        Some(&v.owner),
        &[],
        None,
    )
    .await;
    assert_eq!(json(&body), Value::Null);
    // Sharing an expired recording again never brings the old link back.
    let fresh = share(&v, "never").await;
    assert_ne!(fresh, link);
    assert_eq!(
        anonymous(&v, &format!("{link}/entry")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        anonymous(&v, &format!("{fresh}/entry")).await,
        StatusCode::OK
    );

    // Trash stops sharing. Taking the recording out of Trash (Restore, or importing the same
    // files again) leaves the link closed, and a recording in Trash cannot be shared.
    let (status, _, _) = call(
        &v,
        "POST",
        "/api/v1/entries/cedar/trash",
        Some(&v.owner),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        anonymous(&v, &format!("{fresh}/entry")).await,
        StatusCode::NOT_FOUND
    );
    let (status, _, _) = call(
        &v,
        "PUT",
        "/api/v1/entries/cedar/share",
        Some(&v.owner),
        &[],
        Some(json!({"expires":"never"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    sqlx::query("UPDATE entries SET trashed_at=NULL")
        .execute(&v.app.db)
        .await
        .unwrap();
    assert_eq!(
        anonymous(&v, &format!("{fresh}/entry")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        anonymous(&v, &format!("{fresh}/audio")).await,
        StatusCode::NOT_FOUND
    );
}
