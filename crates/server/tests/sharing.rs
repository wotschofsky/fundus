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
    vault(false).await
}
/// With `proxy_auth`, the owner's browser requests are trusted as they arrive and write nothing
/// before the handler runs (a session token records when it was last used).
async fn vault(proxy_auth: bool) -> Vault {
    let dir = tempfile::tempdir().unwrap();
    let web = dir.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "page").unwrap();
    let mut c = Config::from_env().unwrap();
    c.data = dir.path().join("data");
    c.web = web;
    c.public_url = format!("{ORIGIN}/").parse().unwrap();
    c.proxy_auth = proxy_auth;
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
/// The token in a link's path, which the owner names when changing or stopping that link.
fn token(link: &str) -> &str {
    link.strip_prefix("/s/").unwrap()
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
            "segments",
            "speakers",
            "stage",
            "summary",
            "summary_manual",
            "title",
            "transcript_key",
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

    // The transcript comes with it, with positions for ids; a query cannot pick another recording.
    let (status, headers, body) = call(
        &v,
        "GET",
        &format!("{link}/entry?entry_id=other"),
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    private(&headers);
    let page = json(&body);
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

    // Nothing else under a link answers, and every miss there carries the same headers. (The page
    // itself loads for any token; it says so when its link does not work.)
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
    let read = async || {
        let (_, _, body) = call(
            &v,
            "GET",
            "/api/v1/entries/cedar/share",
            Some(&v.owner),
            &[],
            None,
        )
        .await;
        json(&body)
    };
    let before = read().await;
    // Sharing again while the link works leaves it as it is, its expiry too: a dialog that had
    // not yet seen a link made elsewhere cannot shorten or lengthen it. Only PATCH changes it.
    assert_eq!(share(&v, "never").await, link);
    let after = read().await;
    assert_eq!(after["url"], format!("{ORIGIN}{link}"));
    assert_eq!(after["expires_at"], before["expires_at"]);
    assert_ne!(after["expires_at"], Value::Null);
    let (_, _, body) = call(&v, "GET", "/api/v1/shares", Some(&v.owner), &[], None).await;
    assert_eq!(json(&body)["shares"][0]["entry_id"], ID);
    assert_eq!(json(&body)["shares"][0]["title"], "Cedar launch");

    // A read token reads recordings, but neither sees nor changes links.
    let stop = format!("/api/v1/entries/cedar/share?token={}", token(&link));
    for (method, path, body) in [
        ("GET", "/api/v1/entries/cedar/share", None),
        (
            "PUT",
            "/api/v1/entries/cedar/share",
            Some(json!({"expires":"1d"})),
        ),
        (
            "PATCH",
            "/api/v1/entries/cedar/share",
            Some(json!({"expires":"1d","token":token(&link)})),
        ),
        ("DELETE", stop.as_str(), None),
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
    let (status, _, _) = call(&v, "DELETE", &stop, Some(&v.owner), &[], None).await;
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

/// Whether a link still works is decided when sharing is written. Sharing that had to wait for
/// the database while the link expired makes a new link, as sharing an expired recording again
/// does, rather than bringing the old one back.
#[tokio::test]
async fn sharing_that_waits_past_expiry_makes_a_new_link() {
    let v = vault(true).await;
    let link = share(&v, "1d").await;
    let mut writer = v.app.db.acquire().await.unwrap();
    let soon = (chrono::Utc::now() + chrono::Duration::milliseconds(2000))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    sqlx::query("UPDATE shares SET expires_at=?")
        .bind(soon)
        .execute(&v.app.db)
        .await
        .unwrap();
    // Another writer holds the database from before the change until after the link expired.
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *writer)
        .await
        .unwrap();
    // Still working as the change sets off, so the change cannot have seen it expired.
    assert_eq!(
        anonymous(&v, &format!("{link}/entry")).await,
        StatusCode::OK
    );
    let change = call(
        &v,
        "PUT",
        "/api/v1/entries/cedar/share",
        None,
        &[("Origin", ORIGIN)],
        Some(json!({"expires":"7d"})),
    );
    let meanwhile = async {
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        let status = anonymous(&v, &format!("{link}/entry")).await;
        sqlx::query("COMMIT").execute(&mut *writer).await.unwrap();
        status
    };
    let ((status, _, body), expired) = tokio::join!(change, meanwhile);
    assert_eq!(expired, StatusCode::NOT_FOUND);
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let url = json(&body)["url"].as_str().unwrap().to_string();
    assert_ne!(url.strip_prefix(ORIGIN).unwrap(), link);
    assert_eq!(
        anonymous(&v, &format!("{link}/entry")).await,
        StatusCode::NOT_FOUND
    );
}

/// Changing a link's expiry only ever changes a link that works. A change that lands after Stop
/// sharing (sent from another tab, say) or after the link expired finds nothing to change, so it
/// cannot share the recording again.
#[tokio::test]
async fn an_expiry_change_never_makes_a_link() {
    let v = setup().await;
    let link = share(&v, "1d").await;
    let patch = async |bearer: &str, link: &str, expires: &str| {
        call(
            &v,
            "PATCH",
            "/api/v1/entries/cedar/share",
            Some(bearer),
            &[],
            Some(json!({"expires": expires, "token": token(link)})),
        )
        .await
    };
    // A working link keeps its URL.
    let (status, _, body) = patch(&v.owner, &link, "never").await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let saved = json(&body);
    assert_eq!(saved["url"], format!("{ORIGIN}{link}"));
    assert_eq!(saved["expires_at"], Value::Null);
    assert_eq!(patch(&v.reader, &link, "7d").await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        patch(&v.owner, &link, "forever").await.0,
        StatusCode::BAD_REQUEST
    );

    // After Stop sharing.
    let (status, _, _) = call(
        &v,
        "DELETE",
        &format!("/api/v1/entries/cedar/share?token={}", token(&link)),
        Some(&v.owner),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(patch(&v.owner, &link, "7d").await.0, StatusCode::NOT_FOUND);
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
    assert_eq!(
        anonymous(&v, &format!("{link}/entry")).await,
        StatusCode::NOT_FOUND
    );

    // After the link expired.
    let fresh = share(&v, "1d").await;
    sqlx::query("UPDATE shares SET expires_at='2026-01-01T00:00:00.000Z'")
        .execute(&v.app.db)
        .await
        .unwrap();
    assert_eq!(patch(&v.owner, &fresh, "7d").await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        anonymous(&v, &format!("{fresh}/entry")).await,
        StatusCode::NOT_FOUND
    );
}

/// A link reads the whole transcript with the recording, so a correction the owner makes meanwhile
/// cannot land between two pages of it and mix speaker assignments from before and after.
#[tokio::test]
async fn a_link_reads_the_whole_transcript_at_once() {
    let v = setup().await;
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 2 UNION ALL SELECT i+1 FROM n WHERE i<2101)
         INSERT INTO segments(id,entry_id,position,speaker,start_ms,end_ms,text)
         SELECT 'cedar:'||i,'cedar',i,'B',8000+i*10,8005+i*10,'More.' FROM n",
    )
    .execute(&v.app.db)
    .await
    .unwrap();
    let link = share(&v, "1d").await;
    let (status, _, body) = call(&v, "GET", &format!("{link}/entry"), None, &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let page = json(&body);
    assert_eq!(page["segments"].as_array().unwrap().len(), 2102);

    // A reader holding these segments is not sent them again, until one changes.
    let key = page["transcript_key"].as_str().unwrap().to_owned();
    let held = format!("{link}/entry?transcript_key={key}");
    let (_, _, body) = call(&v, "GET", &held, None, &[], None).await;
    assert_eq!(json(&body)["segments"], Value::Null);
    sqlx::query("UPDATE segments SET text='Changed.' WHERE id='cedar:2'")
        .execute(&v.app.db)
        .await
        .unwrap();
    let (_, _, body) = call(&v, "GET", &held, None, &[], None).await;
    let page = json(&body);
    assert_eq!(page["segments"][2]["text"], "Changed.");
    assert_ne!(page["transcript_key"], key);
}

/// A link's `transcript_key` comes only from what the link shows: the recording's zone, which stays
/// with the owner, cannot be found by trying zones against it (while there is no speech yet, say).
#[tokio::test]
async fn a_link_keys_its_transcript_only_by_what_it_shows() {
    let v = setup().await;
    let link = share(&v, "1d").await;
    let entry = format!("{link}/entry");
    let before = json(&call(&v, "GET", &entry, None, &[], None).await.2)["transcript_key"].clone();
    sqlx::query("UPDATE entries SET timezone='Europe/Berlin' WHERE id='cedar'")
        .execute(&v.app.db)
        .await
        .unwrap();
    let after = json(&call(&v, "GET", &entry, None, &[], None).await.2)["transcript_key"].clone();
    assert!(before.is_string());
    assert_eq!(after, before);
}

/// Changing or stopping a link names it, so a dialog still showing a link that was stopped and
/// replaced meanwhile (from another tab, say) leaves the replacement as it is.
#[tokio::test]
async fn a_change_or_stop_for_an_old_link_leaves_its_replacement_alone() {
    let v = setup().await;
    let old = share(&v, "1d").await;
    let request = async |method: &str, path: &str, body: Option<Value>| {
        call(&v, method, path, Some(&v.owner), &[], body).await.0
    };
    let stop = |link: &str| format!("/api/v1/entries/cedar/share?token={}", token(link));
    assert_eq!(
        request("DELETE", &stop(&old), None).await,
        StatusCode::NO_CONTENT
    );
    let new = share(&v, "7d").await;
    assert_ne!(new, old);

    let change = Some(json!({"expires":"never","token":token(&old)}));
    assert_eq!(
        request("PATCH", "/api/v1/entries/cedar/share", change).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request("DELETE", &stop(&old), None).await,
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
    let current = json(&body);
    assert_eq!(current["url"], format!("{ORIGIN}{new}"));
    assert_ne!(current["expires_at"], Value::Null);
    assert_eq!(anonymous(&v, &format!("{new}/entry")).await, StatusCode::OK);

    assert_eq!(
        request("DELETE", &stop(&new), None).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        anonymous(&v, &format!("{new}/entry")).await,
        StatusCode::NOT_FOUND
    );
}

/// Sharing and changing an expiry answer only once the change is saved: a commit that fails
/// after the statement has produced its row is a failure, not a link that was never stored.
#[tokio::test]
async fn a_change_that_fails_to_commit_is_not_reported_as_saved() {
    let v = setup().await;
    // A check that SQLite runs only at commit, after the statement has returned its row.
    for sql in [
        "CREATE TABLE refused (x TEXT REFERENCES nowhere(id) DEFERRABLE INITIALLY DEFERRED)",
        "CREATE TABLE nowhere (id TEXT PRIMARY KEY)",
        "CREATE TRIGGER refuse_insert AFTER INSERT ON shares BEGIN INSERT INTO refused VALUES ('x'); END",
        "CREATE TRIGGER refuse_update AFTER UPDATE ON shares BEGIN INSERT INTO refused VALUES ('x'); END",
    ] {
        sqlx::query(sql).execute(&v.app.db).await.unwrap();
    }
    let (status, _, _) = call(
        &v,
        "PUT",
        "/api/v1/entries/cedar/share",
        Some(&v.owner),
        &[],
        Some(json!({"expires":"7d"})),
    )
    .await;
    assert!(status.is_server_error(), "{status}");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM shares")
        .fetch_one(&v.app.db)
        .await
        .unwrap();
    assert_eq!(count, 0);

    // A link saved before the check, then changed.
    sqlx::query("DROP TRIGGER refuse_insert")
        .execute(&v.app.db)
        .await
        .unwrap();
    sqlx::query("DROP TRIGGER refuse_update")
        .execute(&v.app.db)
        .await
        .unwrap();
    let link = share(&v, "1d").await;
    sqlx::query(
        "CREATE TRIGGER refuse_update AFTER UPDATE ON shares BEGIN INSERT INTO refused VALUES ('x'); END",
    )
    .execute(&v.app.db)
    .await
    .unwrap();
    let (status, _, _) = call(
        &v,
        "PATCH",
        "/api/v1/entries/cedar/share",
        Some(&v.owner),
        &[],
        Some(json!({"expires":"never","token":token(&link)})),
    )
    .await;
    assert!(status.is_server_error(), "{status}");
    let expires: Option<String> = sqlx::query_scalar("SELECT expires_at FROM shares")
        .fetch_one(&v.app.db)
        .await
        .unwrap();
    assert!(expires.is_some());
}

#[tokio::test]
async fn no_cache_keeps_an_owner_answer_that_holds_a_link() {
    let v = setup().await;
    let link = share(&v, "7d").await;
    // Behind a caching proxy, a kept answer would hand out a link after it was stopped.
    for (method, path, body) in [
        (
            "PUT",
            "/api/v1/entries/cedar/share",
            Some(json!({"expires":"7d"})),
        ),
        (
            "PATCH",
            "/api/v1/entries/cedar/share",
            Some(json!({"expires":"30d","token":token(&link)})),
        ),
        ("GET", "/api/v1/entries/cedar/share", None),
        ("GET", "/api/v1/shares", None),
    ] {
        let (status, headers, _) = call(&v, method, path, Some(&v.owner), &[], body).await;
        assert_eq!(status, StatusCode::OK, "{method} {path}");
        assert_eq!(headers["cache-control"], "no-store", "{method} {path}");
    }
}
