mod common;
use common::{Options, Provider, until};
use fundus::{
    audio,
    config::{App, Config, now},
    pipeline, retrieval, voice,
};
use std::sync::Arc;

fn wav(path: &std::path::Path) {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for i in 0..(48000 * 12) {
        let value = ((i as f64 * 0.03).sin() * 8000000.) as i32;
        writer.write_sample(value).unwrap();
        writer.write_sample(-value).unwrap();
    }
    writer.finalize().unwrap();
}
async fn setup(
    options: Options,
) -> (
    tempfile::TempDir,
    App,
    Arc<Provider>,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::from_env().unwrap();
    config.data = dir.path().join("vault");
    config.voice_model = None;
    let mut app = App::new(config).await.unwrap();
    let (provider, task) = common::connect(&mut app, options).await;
    (dir, app, provider, task)
}
async fn add(app: &App, source: &std::path::Path, batch: &str, uid: &str) {
    tokio::fs::copy(source, app.upload_path(uid)).await.unwrap();
    let length = std::fs::metadata(source).unwrap().len() as i64;
    sqlx::query(
        "INSERT INTO imports(id,state,recorder_timezone,created_at)VALUES(?,'verifying','UTC',?)",
    )
    .bind(batch)
    .bind(now())
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO uploads(id,import_id,filename,original_name,relative_path,length,offset,state)VALUES(?,?,'recording.wav','recording.wav','recording.wav',?,?,'received')").bind(uid).bind(batch).bind(length).bind(length).execute(&app.db).await.unwrap();
}
/// One finalized entry from a synthetic 12-second stereo recording.
async fn entry(app: &App, dir: &std::path::Path, batch: &str) -> String {
    let source = dir.join(format!("{batch}.wav"));
    wav(&source);
    add(app, &source, batch, &format!("{batch}-upload")).await;
    pipeline::finalize(app, batch).await.unwrap();
    sqlx::query_scalar("SELECT id FROM entries ORDER BY created_at DESC LIMIT 1")
        .fetch_one(&app.db)
        .await
        .unwrap()
}
async fn text(app: &App, sql: &str, entry: &str) -> String {
    sqlx::query_scalar(sql)
        .bind(entry)
        .fetch_one(&app.db)
        .await
        .unwrap()
}
async fn stage(app: &App, entry: &str) -> String {
    text(app, "SELECT stage FROM entries WHERE id=?", entry).await
}

/// The first video stream's codec, size and display rotation, as `h264,1280,720,`.
fn probe(path: &std::path::Path) -> String {
    let output = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .args([
            "stream=codec_name,width,height:stream_side_data=rotation",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .replace('\n', ",")
}
/// A media file made by FFmpeg from `args`, then turned by `rotation` degrees in its display
/// matrix, as phones store portrait video.
fn media(
    dir: &std::path::Path,
    name: &str,
    args: &str,
    rotation: Option<i32>,
) -> std::path::PathBuf {
    let ffmpeg = |args: Vec<String>| {
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y"])
            .args(args)
            .status()
            .unwrap();
        assert!(made.success(), "{name}");
    };
    let path = dir.join(name);
    let made = dir.join(format!("unrotated-{name}"));
    let mut first: Vec<String> = args.split_whitespace().map(String::from).collect();
    first.push(made.display().to_string());
    ffmpeg(first);
    match rotation {
        Some(degrees) => ffmpeg(vec![
            "-display_rotation".into(),
            degrees.to_string(),
            "-i".into(),
            made.display().to_string(),
            "-c".into(),
            "copy".into(),
            path.display().to_string(),
        ]),
        None => std::fs::rename(&made, &path).unwrap(),
    }
    path
}
/// The entry `finalize` made from `source`, uploaded alone.
async fn import(app: &App, source: &std::path::Path, batch: &str) -> String {
    add(app, source, batch, &format!("{batch}-upload")).await;
    pipeline::finalize(app, batch).await.unwrap();
    sqlx::query_scalar("SELECT entry_id FROM import_entries WHERE import_id=?")
        .bind(batch)
        .fetch_one(&app.db)
        .await
        .unwrap()
}
async fn call(
    router: &axum::Router,
    token: &str,
    method: &str,
    path: String,
) -> axum::http::Response<axum::body::Body> {
    use tower::ServiceExt;
    router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", format!("Bearer {token}"))
                .header("Range", "bytes=0-31")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn a_plain_mp4_plays_as_uploaded_and_other_video_is_converted_without_holding_up_the_transcript()
 {
    let (dir, app, _provider, task) = setup(Options::default()).await;
    let token = fundus::auth::issue(&app, "test", "admin").await.unwrap();
    let router = fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    let video = |entry: &serde_json::Value| entry["video"].clone();

    // A phone's MP4 plays in every browser as it is: no conversion, no second file.
    let phone = media(
        dir.path(),
        "phone.mp4",
        "-f lavfi -i testsrc=s=1920x1080:r=5:d=2 -f lavfi -i sine=f=440:d=2 -c:v libx264 -pix_fmt yuv420p -c:a aac",
        None,
    );
    let entry = import(&app, &phone, "phone").await;
    let detail = retrieval::get(&app, &entry).await.unwrap();
    assert_eq!(
        video(&detail),
        serde_json::json!({"state":"ready","width":1920,"height":1080,"failure":null})
    );
    let response = call(
        &router,
        &token,
        "GET",
        format!("/api/v1/entries/{entry}/video"),
    )
    .await;
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-type"], "video/mp4");
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(bytes[..], std::fs::read(&phone).unwrap()[..32]);
    assert!(!app.entry_dir(&entry).join(fundus::video::FILE).exists());

    // A portrait video in another codec, stored as landscape frames plus a display matrix.
    let portrait = media(
        dir.path(),
        "portrait.mov",
        "-f lavfi -i testsrc=s=1920x1080:r=5:d=2 -f lavfi -i sine=f=550:d=2 -c:v mpeg4 -c:a pcm_s16le",
        Some(90),
    );
    let entry = import(&app, &portrait, "portrait").await;
    // The transcript does not wait for the conversion.
    pipeline::process(&app, &entry).await.unwrap();
    let detail = retrieval::get(&app, &entry).await.unwrap();
    assert_eq!(detail["stage"], "complete");
    assert_eq!(
        video(&detail),
        serde_json::json!({"state":"preparing","width":720,"height":1280,"failure":null})
    );
    let response = call(
        &router,
        &token,
        "GET",
        format!("/api/v1/entries/{entry}/video"),
    )
    .await;
    assert_eq!(response.status(), 404);
    assert_eq!(
        fundus::video::claim(&app).await.as_deref(),
        Some(entry.as_str())
    );
    fundus::video::convert(&app, &entry, tokio_util::sync::CancellationToken::new()).await;
    assert_eq!(
        video(&retrieval::get(&app, &entry).await.unwrap())["state"],
        "ready"
    );
    // Upright H.264 at 720 pixels, with no rotation left for the browser to apply twice.
    assert_eq!(
        probe(&app.entry_dir(&entry).join(fundus::video::FILE)),
        "h264,720,1280"
    );
    let response = call(
        &router,
        &token,
        "GET",
        format!("/api/v1/entries/{entry}/video"),
    )
    .await;
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-type"], "video/mp4");

    // Cover art in an audio file is not a video.
    let cover = media(
        dir.path(),
        "cover.m4a",
        "-f lavfi -i sine=f=660:d=2 -f lavfi -i color=c=red:s=64x64:r=1:d=1 -map 0:a -map 1:v -c:a aac -c:v png -disposition:v attached_pic",
        None,
    );
    let entry = import(&app, &cover, "cover").await;
    assert_eq!(
        video(&retrieval::get(&app, &entry).await.unwrap()),
        serde_json::Value::Null
    );
    let response = call(
        &router,
        &token,
        "GET",
        format!("/api/v1/entries/{entry}/video"),
    )
    .await;
    assert_eq!(response.status(), 404);
    task.abort();
}

#[tokio::test]
async fn a_converted_video_holds_its_audio_as_the_transcript_does() {
    let (dir, app, _provider, task) = setup(Options::default()).await;
    // The file's timeline starts at 1 s with the picture, 60 brighter for each of its seconds;
    // the speech starts a second later, as after a cut in an editor, and its timestamps skip a
    // second after its first second. MPEG-TS has no index: a seek in it lands between keyframes.
    for name in ["late.mkv", "late.ts"] {
        let late = media(
            dir.path(),
            name,
            "-f lavfi -i nullsrc=s=320x180:r=10:d=4,geq=lum=60*T:cb=128:cr=128,format=yuv420p -itsoffset 1 -f lavfi -i sine=f=440:d=2 -filter_complex [1:a]asetpts=PTS+gte(T\\,2)/TB[a] -map 0:v -map [a] -c:v libx264 -g 25 -bf 0 -c:a aac -output_ts_offset 1",
            None,
        );
        let entry = import(&app, &late, name).await;
        assert_eq!(
            fundus::video::claim(&app).await.as_deref(),
            Some(entry.as_str())
        );
        fundus::video::convert(&app, &entry, tokio_util::sync::CancellationToken::new()).await;
        let detail = retrieval::get(&app, &entry).await.unwrap();
        assert_eq!(
            detail["video"]["state"], "ready",
            "{name}: {}",
            detail["video"]
        );
        // Its audio starts at 0 and is every sample the transcript was made from, back to back.
        let video = app.entry_dir(&entry).join(fundus::video::FILE);
        let probe = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "a:0", "-show_entries"])
            .args(["stream=start_time", "-of", "csv=p=0"])
            .arg(&video)
            .output()
            .unwrap();
        let start: f64 = String::from_utf8(probe.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(start.abs() < 0.05, "{name} starts at {start}");
        let decoded = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&video)
            .args([
                "-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "s32le", "-",
            ])
            .output()
            .unwrap();
        let seconds = decoded.stdout.len() as f64 / 4. / 48000.;
        let recording = detail["duration_ms"].as_f64().unwrap() / 1000.;
        assert!(
            (seconds - recording).abs() < 0.05,
            "{name} keeps {seconds} s of {recording}"
        );
        // Its first picture is the one shown as the speech began, a second into the picture.
        let brightness = |path: &std::path::Path| {
            let frames = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-i"])
                .arg(path)
                .args([
                    "-map",
                    "0:v:0",
                    "-vf",
                    "scale=1:1,format=gray",
                    "-f",
                    "rawvideo",
                    "-",
                ])
                .output()
                .unwrap();
            frames.stdout
        };
        let (first, then) = (brightness(&video)[0], brightness(&late)[10]);
        assert!(
            first.abs_diff(then) <= 4,
            "{name} begins at {first}, not {then}"
        );
    }
    task.abort();
}

#[tokio::test]
async fn a_video_whose_sound_changes_format_midway_is_not_converted() {
    let (dir, app, _provider, task) = setup(Options::default()).await;
    // Two clips joined without re-encoding, stereo MP3 and then mono, as a cut that copies
    // streams makes them.
    let mut list = String::new();
    for (name, channels) in [("stereo.mkv", 2), ("mono.mkv", 1)] {
        let args = format!(
            "-f lavfi -i testsrc=s=320x240:r=5:d=2 -f lavfi -i sine=f=440:d=2 -ac {channels} -c:v mpeg4 -c:a libmp3lame"
        );
        list.push_str(&format!(
            "file '{}'\n",
            media(dir.path(), name, &args, None).display()
        ));
    }
    std::fs::write(dir.path().join("list.txt"), list).unwrap();
    let args = format!(
        "-f concat -safe 0 -i {} -c copy",
        dir.path().join("list.txt").display()
    );
    let joined = media(dir.path(), "joined.mkv", &args, None);
    let entry = import(&app, &joined, "joined").await;
    let detail = retrieval::get(&app, &entry).await.unwrap();
    let recording = detail["duration_ms"].as_i64().unwrap();
    assert!((recording - 4000).abs() < 100, "transcribed {recording} ms");
    assert_eq!(
        fundus::video::claim(&app).await.as_deref(),
        Some(entry.as_str())
    );
    fundus::video::convert(&app, &entry, tokio_util::sync::CancellationToken::new()).await;
    // Its sound would start over at 0 where the format changes and run beside the wrong words.
    let video = &retrieval::get(&app, &entry).await.unwrap()["video"];
    assert_eq!(video["state"], "failed", "{video}");
    assert!(!app.entry_dir(&entry).join(fundus::video::FILE).exists());
    task.abort();
}

#[tokio::test]
async fn a_conversion_stops_short_of_the_disk_reserve() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::from_env().unwrap();
    config.data = dir.path().join("vault");
    config.voice_model = None;
    // More than the disk has free.
    config.reserve = u64::MAX / 2;
    let mut app = App::new(config).await.unwrap();
    let (_provider, task) = common::connect(&mut app, Options::default()).await;
    let source = media(
        dir.path(),
        "clip.mkv",
        "-f lavfi -i testsrc=s=640x360:r=5:d=2 -f lavfi -i sine=f=440:d=2 -c:v mpeg4 -c:a flac",
        None,
    );
    let entry = import(&app, &source, "clip").await;
    assert_eq!(
        fundus::video::claim(&app).await.as_deref(),
        Some(entry.as_str())
    );
    fundus::video::convert(&app, &entry, tokio_util::sync::CancellationToken::new()).await;
    let video = &retrieval::get(&app, &entry).await.unwrap()["video"];
    assert_eq!(video["state"], "failed", "{video}");
    assert_eq!(
        video["failure"],
        "Not enough free disk space to convert this video"
    );
    assert!(!app.entry_dir(&entry).join("video.partial.mp4").exists());
    task.abort();
}

#[tokio::test]
async fn trash_stops_a_conversion_delete_waits_for_it_and_a_crash_loop_ends() {
    let (dir, app, _provider, task) = setup(Options::default()).await;
    let token = fundus::auth::issue(&app, "test", "admin").await.unwrap();
    let router = fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    let source = media(
        dir.path(),
        "clip.mkv",
        "-f lavfi -i testsrc=s=640x360:r=5:d=2 -f lavfi -i sine=f=440:d=2 -c:v mpeg4 -c:a flac",
        None,
    );
    let entry = import(&app, &source, "clip").await;
    let status = || async {
        sqlx::query_as::<_, (Option<String>, i64)>(
            "SELECT video_status,video_attempts FROM entries WHERE id=?",
        )
        .bind(&entry)
        .fetch_one(&app.db)
        .await
        .unwrap()
    };

    // Claimed, and waiting for a CPU the test holds, as behind a long import.
    let cpus = app.cpu.clone().acquire_many_owned(2).await.unwrap();
    assert_eq!(
        fundus::video::claim(&app).await.as_deref(),
        Some(entry.as_str())
    );
    let stop = tokio_util::sync::CancellationToken::new();
    app.conversions
        .lock()
        .unwrap()
        .insert(entry.clone(), stop.clone());
    let (work_app, work_entry) = (app.clone(), entry.clone());
    let converting =
        tokio::spawn(async move { fundus::video::convert(&work_app, &work_entry, stop).await });
    let path = format!("/api/v1/entries/{entry}");
    assert_eq!(
        call(&router, &token, "DELETE", path.clone()).await.status(),
        409
    );
    assert_eq!(
        call(&router, &token, "POST", format!("{path}/trash"))
            .await
            .status(),
        204
    );
    // Trash stopped it at once, though it never got a CPU.
    let stopped = tokio::time::timeout(std::time::Duration::from_secs(10), converting);
    stopped.await.unwrap().unwrap();
    drop(cpus);
    // Back in the queue, without counting as an attempt, and not claimed while in Trash.
    assert_eq!(status().await, (Some("queued".into()), 0));
    assert_eq!(fundus::video::claim(&app).await, None);
    assert_eq!(
        call(&router, &token, "POST", format!("{path}/restore"))
            .await
            .status(),
        200
    );
    // Trashed between the claim and the registration that lets Trash stop it.
    assert_eq!(
        fundus::video::claim(&app).await.as_deref(),
        Some(entry.as_str())
    );
    assert_eq!(
        call(&router, &token, "POST", format!("{path}/trash"))
            .await
            .status(),
        204
    );
    fundus::video::convert(&app, &entry, tokio_util::sync::CancellationToken::new()).await;
    assert_eq!(status().await, (Some("queued".into()), 0));
    assert!(!app.entry_dir(&entry).join(fundus::video::FILE).exists());
    assert_eq!(
        call(&router, &token, "POST", format!("{path}/restore"))
            .await
            .status(),
        200
    );

    // A conversion the server dies in twice is given up, and Retry starts it over.
    for attempts in [1, 2] {
        assert_eq!(
            fundus::video::claim(&app).await.as_deref(),
            Some(entry.as_str())
        );
        assert_eq!(status().await, (Some("converting".into()), attempts));
        fundus::video::recover(&app).await;
    }
    assert_eq!(status().await, (Some("failed".into()), 0));
    let failure = &retrieval::get(&app, &entry).await.unwrap()["video"]["failure"];
    assert!(
        failure.as_str().unwrap().contains("cut short twice"),
        "{failure}"
    );
    assert_eq!(
        call(&router, &token, "POST", format!("{path}/video/retry"))
            .await
            .status(),
        200
    );
    assert_eq!(status().await, (Some("queued".into()), 0));
    assert_eq!(
        call(&router, &token, "POST", format!("{path}/video/retry"))
            .await
            .status(),
        409
    );
    assert_eq!(call(&router, &token, "DELETE", path).await.status(), 204);
    task.abort();
}

#[tokio::test]
async fn audio_to_search_checkpoints_and_pcm_deduplication() {
    let (dir, app, provider, task) = setup(Options {
        pending_polls: 1,
        ..Default::default()
    })
    .await;
    let entry = entry(&app, dir.path(), "batch-a").await;
    // Interrupted preparation leaves only partial artifacts; a resume replaces them.
    std::fs::create_dir_all(app.entry_dir(&entry)).unwrap();
    for name in [
        "playback.partial.m4a",
        "transcription.partial.flac",
        "enrichment.partial.json",
    ] {
        std::fs::write(app.entry_dir(&entry).join(name), b"interrupted").unwrap();
    }
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(stage(&app, &entry).await, "complete");
    assert_eq!(provider.submits(), 1);
    // The job waited through a pending poll, and the whole recording went up as one rendition.
    assert!(provider.polls.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    let request = provider.transcripts.lock().unwrap()[0].clone();
    assert_eq!(request["speaker_labels"], true);
    assert_eq!(request["speech_models"][0], "universal-3-5-pro");
    assert_eq!(request["language_codes"], serde_json::json!(["de", "en"]));
    assert!(request.get("prompt").is_none(), "no context, no prompt");
    let dir_entry = app.entry_dir(&entry);
    assert!(dir_entry.join("playback.m4a").exists());
    assert!(dir_entry.join("transcription.json").exists());
    assert!(
        !dir_entry.join("transcription.flac").exists(),
        "rendition is temporary"
    );
    let detail = retrieval::get(&app, &entry).await.unwrap();
    assert_eq!(detail["title"], "Cedar launch planning");
    assert_eq!(detail["audio_available"], true);
    // Only the unconfigured voice model is reported; summaries and search worked.
    assert_eq!(detail["warnings"].as_array().unwrap().len(), 1);
    assert!(
        detail["warnings"][0]
            .as_str()
            .unwrap()
            .starts_with("Speaker recognition unavailable")
    );
    assert_eq!(detail["speakers"].as_array().unwrap().len(), 2);
    // A self-introduction, validated against speaker A's own words, names a new person. Without
    // a voice model there is no voice to enroll.
    assert_eq!(detail["speakers"][0]["label"], "A");
    assert_eq!(detail["speakers"][0]["evidence"][0]["name"], "Morgan");
    assert_eq!(detail["speakers"][0]["evidence"][0]["applied"], true);
    assert_eq!(detail["speakers"][0]["name"], "Morgan");
    assert_eq!(detail["speakers"][0]["state"], "automatic");
    assert_eq!(detail["speakers"][1]["person_id"], serde_json::Value::Null);
    // An unidentified speaker has no name, not an empty one: clients number them ("Speaker 2").
    assert_eq!(detail["speakers"][1]["name"], serde_json::Value::Null);
    let samples: i64 = sqlx::query_scalar("SELECT count(*) FROM voice_samples")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(samples, 0);
    let result = retrieval::search(
        &app,
        retrieval::Search {
            query: "cedar".into(),
            filters: Default::default(),
        },
    )
    .await
    .unwrap();
    assert_eq!(result["mode"], "hybrid");
    assert_eq!(result["results"][0]["entry_id"], entry);
    assert!(
        result["results"][0]["url"]
            .as_str()
            .unwrap()
            .contains("?t=")
    );
    let transcript = retrieval::transcript(
        &app,
        retrieval::Read {
            entry_id: entry.clone(),
            cursor: Some(0),
            limit: Some(1),
            from_ms: None,
            to_ms: None,
        },
        300,
    )
    .await
    .unwrap();
    assert_eq!(transcript["segments"].as_array().unwrap().len(), 1);
    assert_eq!(transcript["next_cursor"], 1);
    // An explicit retry reuses the saved result rather than billing transcription again.
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(provider.submits(), 1);
    let source = dir.path().join("batch-a.wav");
    let flac = dir.path().join("source.flac");
    audio::ffmpeg(&[
        "-i".into(),
        source.display().to_string(),
        "-c:a".into(),
        "flac".into(),
        flac.display().to_string(),
    ])
    .await
    .unwrap();
    assert_eq!(
        audio::verify(&source).await.unwrap().1,
        audio::verify(&flac).await.unwrap().1
    );
    add(&app, &flac, "batch-b", "upload-b").await;
    pipeline::finalize(&app, "batch-b").await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let duplicate: String = sqlx::query_scalar("SELECT state FROM uploads WHERE id='upload-b'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(duplicate, "duplicate");
    // A manual correction changes the sample owner.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_samples")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(count, 0);
    for id in ["morgan", "avery"] {
        sqlx::query("INSERT INTO people(id,name)VALUES(?,?)")
            .bind(id)
            .bind(id)
            .execute(&app.db)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO voice_probes(entry_id,speaker,model,vector_json)VALUES(?,'A',?,'[1.0,0.0]')",
    )
    .bind(&entry)
    .bind(voice::MODEL)
    .execute(&app.db)
    .await
    .unwrap();
    voice::assign(&app, &entry, "A", Some("morgan"))
        .await
        .unwrap();
    voice::assign(&app, &entry, "A", Some("avery"))
        .await
        .unwrap();
    let owner: String = sqlx::query_scalar("SELECT person_id FROM voice_samples")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(owner, "avery");
    task.abort();
}

#[tokio::test]
async fn graceful_shutdown_saves_the_job_and_restart_resumes_polling() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (dir, app, provider, task) = setup(Options {
        gate: Some(gate.clone()),
        pending_polls: 1,
        ..Default::default()
    })
    .await;
    let entry = entry(&app, dir.path(), "shutdown").await;
    let stop = tokio_util::sync::CancellationToken::new();
    let mut worker = tokio::spawn(pipeline::worker(app.clone(), stop.clone()));
    until(|| provider.submits() == 1).await;
    sqlx::query("INSERT INTO entries(id,fingerprint,title,duration_ms,timezone,stage,created_at,updated_at) VALUES('next-job','next-job','Synthetic queued job',1,'UTC','queued',?,?)")
        .bind(now()).bind(now()).execute(&app.db).await.unwrap();
    stop.cancel();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut worker)
            .await
            .is_err(),
        "Shutdown discarded the in-flight transcription request instead of draining it"
    );
    gate.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap();
    // The job ID was saved; the remaining wait was abandoned rather than holding up shutdown.
    let job = text(
        &app,
        "SELECT remote_id FROM transcriptions WHERE entry_id=?",
        &entry,
    )
    .await;
    assert_eq!(job, "job-0");
    assert_eq!(stage(&app, &entry).await, "transcribing");
    assert_eq!(
        stage(&app, "next-job").await,
        "queued",
        "Shutdown must not start the next job"
    );
    sqlx::query("DELETE FROM entries WHERE id='next-job'")
        .execute(&app.db)
        .await
        .unwrap();
    // After a restart the same job is polled to completion, never submitted again.
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(pipeline::worker(app.clone(), stop.clone()));
    let mut done = false;
    for _ in 0..1000 {
        if stage(&app, &entry).await == "complete" {
            done = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    stop.cancel();
    worker.await.unwrap();
    assert!(done, "the resumed job did not complete");
    assert_eq!(provider.submits(), 1);
    task.abort();
}

#[tokio::test]
async fn ambiguous_submission_requires_explicit_retry_and_saved_jobs_resume() {
    let (_dir, app, provider, task) = setup(Options::default()).await;
    for id in ["interrupted", "resumable"] {
        sqlx::query("INSERT INTO entries(id,fingerprint,title,duration_ms,timezone,stage,active_stage,created_at,updated_at) VALUES(?,?,'Synthetic job',1,'UTC','transcribing','transcribing',?,?)")
            .bind(id).bind(id).bind(now()).bind(now()).execute(&app.db).await.unwrap();
    }
    sqlx::query("INSERT INTO transcriptions(entry_id,state,submitted_at) VALUES('interrupted','submitted',?)")
        .bind(now())
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO transcriptions(entry_id,state,remote_id,submitted_at) VALUES('resumable','processing','job-7',?)")
        .bind(now())
        .execute(&app.db)
        .await
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    pipeline::worker(app.clone(), stop).await;
    let (stage_name, failure): (String, String) =
        sqlx::query_as("SELECT stage,failure FROM entries WHERE id='interrupted'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(stage_name, "failed");
    assert!(failure.contains("Retry explicitly"));
    assert_eq!(stage(&app, "resumable").await, "queued");
    assert_eq!(provider.submits(), 0);
    // Processing on its own never resubmits an ambiguous request.
    let error = pipeline::process(&app, "interrupted").await.unwrap_err();
    assert!(error.to_string().contains("Retry explicitly"));
    // The explicit Retry clears only an ambiguous marker; a saved job ID survives it.
    let admin = fundus::auth::issue(&app, "test owner", "admin")
        .await
        .unwrap();
    let router = fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    sqlx::query("UPDATE entries SET stage='failed' WHERE id IN ('interrupted','resumable')")
        .execute(&app.db)
        .await
        .unwrap();
    for id in ["interrupted", "resumable"] {
        use tower::ServiceExt;
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/v1/entries/{id}/retry"))
                    .header("Authorization", format!("Bearer {admin}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
    let remaining: Vec<String> = sqlx::query_scalar("SELECT entry_id FROM transcriptions")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(remaining, vec!["resumable".to_string()]);
    task.abort();
}

#[tokio::test]
async fn failed_job_clears_its_checkpoint_so_retry_resubmits() {
    let (dir, app, provider, task) = setup(Options {
        failing_jobs: 1,
        ..Default::default()
    })
    .await;
    let entry = entry(&app, dir.path(), "failing").await;
    let error = pipeline::process(&app, &entry).await.unwrap_err();
    assert!(error.to_string().contains("synthetic failure"));
    let saved: i64 = sqlx::query_scalar("SELECT count(*) FROM transcriptions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(saved, 0);
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(provider.submits(), 2);
    assert_eq!(stage(&app, &entry).await, "complete");
    task.abort();
}

#[tokio::test]
async fn optional_providers_degrade_to_keyword_search_and_a_visible_warning() {
    let (dir, mut app, provider, task) = setup(Options::default()).await;
    let config = Arc::make_mut(&mut app.config);
    config.openai_key = String::new();
    let entry = entry(&app, dir.path(), "optional").await;
    pipeline::process(&app, &entry).await.unwrap();
    let detail = retrieval::get(&app, &entry).await.unwrap();
    assert_eq!(detail["stage"], "complete");
    let warnings: Vec<&str> = detail["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("Summary unavailable") && w.contains("OPENAI_API_KEY"))
    );
    assert!(
        !warnings.iter().any(|w| w.starts_with("Semantic search")),
        "{warnings:?}"
    );
    assert!(provider.chats.lock().unwrap().is_empty());
    let result = retrieval::search(
        &app,
        retrieval::Search {
            query: "cedar".into(),
            filters: Default::default(),
        },
    )
    .await
    .unwrap();
    assert_eq!(result["mode"], "keyword");
    assert_eq!(result["partial_results"], false);
    assert_eq!(result["results"][0]["entry_id"], entry);
    task.abort();
}

#[tokio::test]
async fn pause_waits_for_the_job_id_and_resume_polls_the_same_job() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (dir, app, provider, task) = setup(Options {
        gate: Some(gate.clone()),
        pending_polls: 1,
        ..Default::default()
    })
    .await;
    let entry = entry(&app, dir.path(), "pause").await;
    let (work_app, work_entry) = (app.clone(), entry.clone());
    let process = tokio::spawn(async move { pipeline::process(&work_app, &work_entry).await });
    until(|| provider.submits() == 1).await;
    // Audio is playable while the transcription is still running.
    assert_eq!(
        retrieval::get(&app, &entry).await.unwrap()["audio_available"],
        true
    );
    sqlx::query("UPDATE entries SET pause_requested=1 WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    assert!(
        !process.is_finished(),
        "A pause must not discard an in-flight transcription request"
    );
    gate.notify_one();
    process.await.unwrap().unwrap();
    let (stage_name, requested): (String, bool) =
        sqlx::query_as("SELECT stage,pause_requested FROM entries WHERE id=?")
            .bind(&entry)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(stage_name, "paused");
    assert!(!requested);
    assert_eq!(
        text(
            &app,
            "SELECT remote_id FROM transcriptions WHERE entry_id=?",
            &entry
        )
        .await,
        "job-0"
    );
    // Clean restarts keep a deliberately paused entry paused.
    let shutdown = tokio_util::sync::CancellationToken::new();
    shutdown.cancel();
    pipeline::worker(app.clone(), shutdown).await;
    assert_eq!(stage(&app, &entry).await, "paused");
    sqlx::query("UPDATE entries SET stage='queued' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(provider.submits(), 1);
    assert_eq!(stage(&app, &entry).await, "complete");
    task.abort();
}

#[tokio::test]
async fn trash_cancels_processing_at_the_next_checkpoint() {
    use tower::ServiceExt;
    let gate = Arc::new(tokio::sync::Notify::new());
    let (dir, app, provider, task) = setup(Options {
        gate: Some(gate.clone()),
        pending_polls: 1,
        ..Default::default()
    })
    .await;
    let entry = entry(&app, dir.path(), "cancel").await;
    let (work_app, work_entry) = (app.clone(), entry.clone());
    let process = tokio::spawn(async move { pipeline::process(&work_app, &work_entry).await });
    until(|| provider.submits() == 1).await;
    let token = fundus::auth::issue(&app, "test", "admin").await.unwrap();
    let response = fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new())
        .oneshot(
            axum::http::Request::post(format!("/api/v1/entries/{entry}/trash"))
                .header("Authorization", format!("Bearer {token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    gate.notify_one();
    process.await.unwrap().unwrap();
    assert_eq!(stage(&app, &entry).await, "paused");
    task.abort();
}

#[tokio::test]
async fn speaker_reanalysis_preserves_transcription_and_manual_identities() {
    let (dir, app, provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "identity").await;
    pipeline::process(&app, &entry).await.unwrap();
    sqlx::query("INSERT INTO people(id,name)VALUES('manual-person','Synthetic person')")
        .execute(&app.db)
        .await
        .unwrap();
    voice::assign(&app, &entry, "A", Some("manual-person"))
        .await
        .unwrap();
    // The summary names speakers by placeholder, so it follows the assignment.
    let summary = |app: App, entry: String| async move {
        fundus::retrieval::get(&app, &entry).await.unwrap()["summary"].clone()
    };
    assert_eq!(
        summary(app.clone(), entry.clone()).await,
        "Synthetic person plans the launch with Speaker 2."
    );
    sqlx::query("UPDATE entries SET summary='Old summary' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    let transcript: Vec<(String, String)> =
        sqlx::query_as("SELECT id,text FROM segments WHERE entry_id=? ORDER BY position")
            .bind(&entry)
            .fetch_all(&app.db)
            .await
            .unwrap();
    pipeline::reanalyze_speakers(&app, &entry).await.unwrap();
    assert_eq!(provider.submits(), 1);
    let after: Vec<(String, String)> =
        sqlx::query_as("SELECT id,text FROM segments WHERE entry_id=? ORDER BY position")
            .bind(&entry)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(transcript, after);
    let identity: (String, String) =
        sqlx::query_as("SELECT person_id,state FROM speakers WHERE entry_id=? AND label='A'")
            .bind(&entry)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(identity, ("manual-person".into(), "manual".into()));
    // Reanalyze writes fresh notes, e.g. for summaries from before placeholders.
    assert_eq!(
        summary(app.clone(), entry.clone()).await,
        "Synthetic person plans the launch with Speaker 2."
    );
    task.abort();
}

#[tokio::test]
async fn summary_regenerates_alone_and_keeps_the_speakers() {
    use tower::ServiceExt;
    let (dir, app, provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "summary").await;
    pipeline::process(&app, &entry).await.unwrap();
    sqlx::query("INSERT INTO people(id,name)VALUES('manual-person','Synthetic person')")
        .execute(&app.db)
        .await
        .unwrap();
    voice::assign(&app, &entry, "A", Some("manual-person"))
        .await
        .unwrap();
    sqlx::query("UPDATE entries SET summary='Old summary',warnings='[\"Summary unavailable: synthetic\"]' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    let chats = provider.chats.lock().unwrap().len();
    let token = fundus::auth::issue(&app, "test", "admin").await.unwrap();
    let response = fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new())
        .oneshot(
            axum::http::Request::post(format!("/api/v1/entries/{entry}/summarize"))
                .header("Authorization", format!("Bearer {token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(stage(&app, &entry).await, "queued");
    pipeline::regenerate_summary(&app, &entry).await.unwrap();
    let detail = retrieval::get(&app, &entry).await.unwrap();
    assert_eq!(detail["stage"], "complete");
    assert_eq!(
        detail["summary"],
        "Synthetic person plans the launch with Speaker 2."
    );
    assert_eq!(detail["warnings"], serde_json::json!([]));
    assert_eq!(provider.chats.lock().unwrap().len(), chats + 1);
    // No voice work ran: the owner's assignment stands.
    let identity: (String, String) =
        sqlx::query_as("SELECT person_id,state FROM speakers WHERE entry_id=? AND label='A'")
            .bind(&entry)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(identity, ("manual-person".into(), "manual".into()));
    task.abort();
}

#[tokio::test]
async fn retranscribing_a_legacy_entry_replaces_its_part_labels() {
    let (dir, app, _provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "legacy").await;
    // A transcript from the former ten-minute-part pipeline, with a person on one part label.
    sqlx::query("INSERT INTO people(id,name)VALUES('p','Synthetic')")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO speakers(entry_id,label,person_id,state)VALUES(?,'1:A','p','manual')")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    // An earlier job's "A", named and enrolled: the new job's "A" may be someone else.
    sqlx::query("INSERT INTO speakers(entry_id,label,person_id,state)VALUES(?,'A','p','manual')")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO voice_samples(id,person_id,entry_id,speaker,model,vector_json,provenance,created_at)VALUES('s','p',?,'A','m','[1]','manual','2026-01-01')")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    for position in 0..5 {
        sqlx::query("INSERT INTO segments(id,entry_id,position,speaker,start_ms,end_ms,text)VALUES(?,?,?,'1:A',0,1,'old')")
            .bind(format!("{entry}:{position}"))
            .bind(&entry)
            .bind(position)
            .execute(&app.db)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO parts(entry_id,part,state,raw_json)VALUES(?,0,'complete','{\"segments\":[]}')",
    )
    .bind(&entry)
    .execute(&app.db)
    .await
    .unwrap();
    // Its notes describe the old labels and text; they must not survive the new transcript.
    tokio::fs::create_dir_all(app.entry_dir(&entry))
        .await
        .unwrap();
    tokio::fs::write(
        app.entry_dir(&entry).join("enrichment.json"),
        r#"{"title":"Old","summary":"Old summary","names":[{"speaker":"1:A","name":"Old","quote":"old"}]}"#,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE entries SET summary='Old summary' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(
        text(&app, "SELECT summary FROM entries WHERE id=?", &entry).await,
        "{A} plans the launch with {B}."
    );
    assert!(
        text(
            &app,
            "SELECT evidence FROM speakers WHERE entry_id=? AND label='A'",
            &entry
        )
        .await
        .contains("Morgan")
    );
    let speakers: Vec<String> =
        sqlx::query_scalar("SELECT label FROM speakers WHERE entry_id=? ORDER BY label")
            .bind(&entry)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(speakers, vec!["A", "B"]);
    // Only the new job's own evidence names anyone: A's introduction, not the old manual name.
    let named: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT s.label,p.name FROM speakers s LEFT JOIN people p ON p.id=s.person_id WHERE s.entry_id=? ORDER BY s.label",
    )
    .bind(&entry)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        named,
        [("A".into(), Some("Morgan".into())), ("B".into(), None)]
    );
    let samples: i64 = sqlx::query_scalar("SELECT count(*) FROM voice_samples WHERE entry_id=?")
        .bind(&entry)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(samples, 0);
    let segments: i64 = sqlx::query_scalar("SELECT count(*) FROM segments WHERE entry_id=?")
        .bind(&entry)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(segments, 3);
    task.abort();
}

#[tokio::test]
async fn a_lost_create_answer_is_found_by_its_upload_url_and_paid_once() {
    let (dir, app, provider, task) = setup(Options {
        lost_answers: 1,
        ..Default::default()
    })
    .await;
    let entry = entry(&app, dir.path(), "lost").await;
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(stage(&app, &entry).await, "complete");
    assert_eq!(provider.submits(), 1);
    // The vault keeps the result; the job and its audio are removed at AssemblyAI.
    assert_eq!(provider.deletes(), 1);
    task.abort();
}

#[tokio::test]
async fn a_restart_after_an_unanswered_submit_resumes_the_job_it_finds() {
    let (dir, app, provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "restart").await;
    provider
        .jobs
        .lock()
        .unwrap()
        .push(("job-5".into(), "https://cdn.example/earlier".into()));
    sqlx::query("INSERT INTO transcriptions(entry_id,state,audio_url,submitted_at)VALUES(?,'submitted','https://cdn.example/earlier',?)")
        .bind(&entry).bind(now()).execute(&app.db).await.unwrap();
    sqlx::query("UPDATE entries SET stage='transcribing' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    pipeline::worker(app.clone(), stop).await;
    assert_eq!(stage(&app, &entry).await, "queued");
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(stage(&app, &entry).await, "complete");
    assert_eq!(provider.submits(), 0);
    task.abort();
}

#[tokio::test]
async fn a_job_gone_at_assemblyai_fails_cleanly_so_retry_submits_again() {
    let (dir, app, provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "gone").await;
    sqlx::query("INSERT INTO transcriptions(entry_id,state,remote_id,audio_url,submitted_at)VALUES(?,'processing','gone','https://cdn.example/gone',?)")
        .bind(&entry).bind(now()).execute(&app.db).await.unwrap();
    let error = pipeline::process(&app, &entry).await.unwrap_err();
    assert!(error.to_string().contains("no longer has"), "{error}");
    assert_eq!(stage(&app, &entry).await, "failed");
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(provider.submits(), 1);
    assert_eq!(stage(&app, &entry).await, "complete");
    task.abort();
}

#[tokio::test]
async fn recordings_over_assemblyais_limit_are_refused_before_upload() {
    let (dir, app, provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "long").await;
    sqlx::query("UPDATE entries SET duration_ms=? WHERE id=?")
        .bind(10 * 60 * 60 * 1000 + 1)
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    let error = pipeline::process(&app, &entry).await.unwrap_err();
    assert!(error.to_string().contains("10 hours"), "{error}");
    assert_eq!(
        provider.uploads.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    task.abort();
}

#[tokio::test]
async fn read_tokens_cannot_list_trash() {
    use tower::ServiceExt;
    let (_dir, app, _provider, task) = setup(Options::default()).await;
    let router = fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new());
    for (scope, expected) in [("read", 403), ("admin", 200)] {
        let token = fundus::auth::issue(&app, "test", scope).await.unwrap();
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::get("/api/v1/entries?trash=true")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{scope}");
    }
    task.abort();
}

#[tokio::test]
async fn a_hand_edited_summary_is_kept_and_search_finds_it() {
    use tower::ServiceExt;
    let (dir, app, _provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "search-summary").await;
    pipeline::process(&app, &entry).await.unwrap();
    let found = retrieval::search(
        &app,
        retrieval::Search {
            query: "plans".into(),
            filters: retrieval::Filters {
                limit: Some(5),
                ..Default::default()
            },
        },
    )
    .await
    .unwrap();
    assert!(
        found["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| hit["summary"] == true && hit["excerpt"].as_str().unwrap().contains("plans"))
    );
    let token = fundus::auth::issue(&app, "test", "admin").await.unwrap();
    let response = fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new())
        .oneshot(
            axum::http::Request::patch(format!("/api/v1/entries/{entry}"))
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"summary":"A handmade note about the cedar."}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    pipeline::regenerate_summary(&app, &entry).await.unwrap();
    let detail = retrieval::get(&app, &entry).await.unwrap();
    assert_eq!(detail["summary"], "A handmade note about the cedar.");
    assert_eq!(detail["summary_manual"], true);
    let found = retrieval::search(
        &app,
        retrieval::Search {
            query: "handmade".into(),
            filters: retrieval::Filters {
                limit: Some(5),
                ..Default::default()
            },
        },
    )
    .await
    .unwrap();
    assert!(
        found["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| hit["summary"] == true
                && hit["excerpt"].as_str().unwrap().contains("handmade"))
    );
    let response = fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new())
        .oneshot(
            axum::http::Request::patch(format!("/api/v1/entries/{entry}"))
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(r#"{"summary":""}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    pipeline::regenerate_summary(&app, &entry).await.unwrap();
    assert_eq!(retrieval::get(&app, &entry).await.unwrap()["summary"], "");
    let summaries: i64 =
        sqlx::query_scalar("SELECT count(*) FROM passages WHERE entry_id=? AND position=-1")
            .bind(&entry)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(summaries, 0);
    task.abort();
}

#[tokio::test]
async fn long_transcripts_are_summarized_in_one_request() {
    let (_dir, app, provider, task) = setup(Options::default()).await;
    let lines: Vec<String> = (0..6000)
        .map(|i| format!("Sentence {i} about the cedar launch plan and the next proposal."))
        .collect();
    let transcript = fundus::provider::Transcript {
        duration_ms: 6_000_000,
        lines: lines
            .into_iter()
            .enumerate()
            .map(|(i, text)| fundus::provider::Line {
                start_ms: (i as i64) * 1000,
                speaker: "A".into(),
                text,
            })
            .collect(),
    };
    fundus::provider::summarize(&app, &transcript)
        .await
        .unwrap();
    let chats = provider.chats.lock().unwrap().clone();
    assert_eq!(chats.len(), 1);
    assert_eq!(
        chats[0]["response_format"]["json_schema"]["name"],
        "recording_summary"
    );
    let system = chats[0]["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("Never write a clock time"));
    let user = chats[0]["messages"][1]["content"].as_str().unwrap();
    assert!(user.contains("Duration:"));
    assert!(user.contains("as a short block for each topic"));
    assert!(user.ends_with("Sentence 5999 about the cedar launch plan and the next proposal."));
    task.abort();
}

#[tokio::test]
async fn reviewed_passage_groups_survive_speaker_reanalysis_and_retries() {
    let (dir, app, provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "correction").await;
    pipeline::process(&app, &entry).await.unwrap();
    sqlx::query(
        "INSERT INTO speakers(entry_id,label,state) VALUES(?,'corrected','manual_unknown')",
    )
    .bind(&entry)
    .execute(&app.db)
    .await
    .unwrap();
    let segment = format!("{entry}:2");
    sqlx::query("UPDATE segments SET speaker='corrected' WHERE id=?")
        .bind(&segment)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO speaker_corrections(segment_id,entry_id,speaker,created_at) VALUES(?,?,'corrected','now')")
        .bind(&segment).bind(&entry).execute(&app.db).await.unwrap();
    let before: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id,speaker,text FROM segments WHERE entry_id=? ORDER BY position")
            .bind(&entry)
            .fetch_all(&app.db)
            .await
            .unwrap();
    pipeline::reanalyze_speakers(&app, &entry).await.unwrap();
    let after: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id,speaker,text FROM segments WHERE entry_id=? ORDER BY position")
            .bind(&entry)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(before, after);
    assert_eq!(provider.submits(), 1);
    let locked: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM speaker_corrections WHERE segment_id=?")
            .bind(&segment)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(locked, 1);
    task.abort();
}

#[tokio::test]
async fn transcription_keeps_summaries_edited_before_or_during_the_job() {
    use tower::ServiceExt;
    for edit_before_job in [true, false] {
        let gate = Arc::new(tokio::sync::Notify::new());
        let (dir, app, provider, task) = setup(Options {
            gate: Some(gate.clone()),
            ..Default::default()
        })
        .await;
        let entry = entry(&app, dir.path(), "manual-transcription").await;
        let token = fundus::auth::issue(&app, "test", "admin").await.unwrap();
        let edit = || async {
            let response =
                fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new())
                    .oneshot(
                        axum::http::Request::patch(format!("/api/v1/entries/{entry}"))
                            .header("Authorization", format!("Bearer {token}"))
                            .header("Content-Type", "application/json")
                            .body(axum::body::Body::from(
                                r#"{"summary":"My handwritten outcome."}"#,
                            ))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
            assert_eq!(response.status(), 200);
        };
        if edit_before_job {
            edit().await;
        }
        let work = tokio::spawn({
            let app = app.clone();
            let entry = entry.clone();
            async move { pipeline::process(&app, &entry).await.unwrap() }
        });
        until(|| provider.submits() == 1).await;
        if !edit_before_job {
            edit().await;
        }
        gate.notify_one();
        work.await.unwrap();
        let detail = retrieval::get(&app, &entry).await.unwrap();
        assert_eq!(detail["title"], "Cedar launch planning");
        assert_eq!(detail["summary"], "My handwritten outcome.");
        assert_eq!(detail["summary_manual"], true);
        task.abort();
    }
}

#[tokio::test]
async fn an_older_summary_refresh_cannot_overwrite_a_newer_edit() {
    for full_index in [false, true] {
        let gate = Arc::new(tokio::sync::Notify::new());
        let (dir, app, provider, task) = setup(Options {
            first_embedding_gate: Some(gate.clone()),
            ..Default::default()
        })
        .await;
        let entry = entry(&app, dir.path(), "stale-summary").await;
        sqlx::query("UPDATE entries SET summary='Earlier outcome' WHERE id=?")
            .bind(&entry)
            .execute(&app.db)
            .await
            .unwrap();
        let older = tokio::spawn({
            let app = app.clone();
            let entry = entry.clone();
            async move {
                if full_index {
                    pipeline::index(&app, &entry).await.unwrap();
                } else {
                    pipeline::refresh_summary_passage(&app, &entry)
                        .await
                        .unwrap();
                }
            }
        });
        until(|| {
            provider
                .embeddings
                .load(std::sync::atomic::Ordering::SeqCst)
                == 1
        })
        .await;
        sqlx::query(
            "UPDATE entries SET summary='Newer handmade outcome',summary_manual=1 WHERE id=?",
        )
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
        pipeline::refresh_summary_passage(&app, &entry)
            .await
            .unwrap();
        gate.notify_one();
        older.await.unwrap();
        let indexed = text(
            &app,
            "SELECT text FROM passages WHERE entry_id=? AND position=-1",
            &entry,
        )
        .await;
        assert_eq!(indexed, "Newer handmade outcome");
        let indexed = text(
            &app,
            "SELECT text FROM passage_fts WHERE entry_id=?",
            &entry,
        )
        .await;
        assert_eq!(indexed, "Newer handmade outcome");
        task.abort();
    }
}

#[tokio::test]
async fn shutdown_waits_for_person_summary_refreshes() {
    use tower::ServiceExt;
    let gate = Arc::new(tokio::sync::Notify::new());
    let (dir, app, provider, task) = setup(Options {
        first_embedding_gate: Some(gate.clone()),
        ..Default::default()
    })
    .await;
    let entry = entry(&app, dir.path(), "tracked-summary").await;
    sqlx::query("UPDATE entries SET summary='{A} outcome' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO people(id,name) VALUES('p','Before')")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO speakers(entry_id,label,person_id,state) VALUES(?,'A','p','manual')")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    let token = fundus::auth::issue(&app, "test", "admin").await.unwrap();
    let response = fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new())
        .oneshot(
            axum::http::Request::patch("/api/v1/people/p")
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(r#"{"name":"After"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    until(|| {
        provider
            .embeddings
            .load(std::sync::atomic::Ordering::SeqCst)
            == 1
    })
    .await;
    app.background.close();
    assert!(!app.background.is_empty());
    assert!(app.mutations.try_lock().is_ok());
    gate.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), app.background.wait())
        .await
        .unwrap();
    assert_eq!(
        text(
            &app,
            "SELECT text FROM passages WHERE entry_id=? AND position=-1",
            &entry
        )
        .await,
        "After outcome"
    );
    task.abort();
}

/// Status of an owner request to the browser API.
async fn owner(app: &App, method: &str, uri: &str) -> u16 {
    use tower::ServiceExt;
    let token = fundus::auth::issue(app, "test", "admin").await.unwrap();
    fundus::api::router(app.clone(), tokio_util::sync::CancellationToken::new())
        .oneshot(
            axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .header("Authorization", format!("Bearer {token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
async fn a_summary_trashed_mid_request_waits_for_restore_and_is_not_paid_twice() {
    let (dir, mut app, _provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "trashed-summary").await;
    pipeline::process(&app, &entry).await.unwrap();
    task.abort();
    let gate = Arc::new(tokio::sync::Notify::new());
    let (provider, task) = common::connect(
        &mut app,
        Options {
            chat_gate: Some(gate.clone()),
            ..Default::default()
        },
    )
    .await;
    sqlx::query("UPDATE entries SET summary='Old summary' WHERE id=?")
        .bind(&entry)
        .execute(&app.db)
        .await
        .unwrap();
    let uri = |action: &str| format!("/api/v1/entries/{entry}/{action}");
    assert_eq!(owner(&app, "POST", &uri("summarize")).await, 200);
    let (work_app, work_entry) = (app.clone(), entry.clone());
    let job =
        tokio::spawn(async move { pipeline::regenerate_summary(&work_app, &work_entry).await });
    until(|| provider.chats.lock().unwrap().len() == 1).await;
    assert_eq!(owner(&app, "POST", &uri("trash")).await, 204);
    gate.notify_one();
    job.await.unwrap().unwrap();
    // The answer came back after the trash: nothing was written, and the request stays open.
    assert_eq!(stage(&app, &entry).await, "paused");
    let open = "SELECT requested_work FROM entries WHERE id=?";
    assert_eq!(text(&app, open, &entry).await, "summary");
    let summary = "SELECT summary FROM entries WHERE id=?";
    assert_eq!(text(&app, summary, &entry).await, "Old summary");
    assert_eq!(owner(&app, "POST", &uri("restore")).await, 200);
    assert_eq!(owner(&app, "POST", &uri("retry")).await, 200);
    gate.notify_one(); // a second request would otherwise wait forever; the count below catches it
    pipeline::regenerate_summary(&app, &entry).await.unwrap();
    assert_eq!(stage(&app, &entry).await, "complete");
    assert_ne!(text(&app, summary, &entry).await, "Old summary");
    assert_eq!(
        provider.chats.lock().unwrap().len(),
        1,
        "the saved answer is reused, not requested again"
    );
    task.abort();
}

#[tokio::test]
async fn a_summary_job_that_finds_its_recording_in_trash_stops_and_can_be_deleted() {
    let (dir, app, provider, task) = setup(Options::default()).await;
    let entry = entry(&app, dir.path(), "trashed-early").await;
    pipeline::process(&app, &entry).await.unwrap();
    assert_eq!(
        owner(&app, "POST", &format!("/api/v1/entries/{entry}/summarize")).await,
        200
    );
    // The worker claimed the job, then the owner moved the recording to Trash.
    sqlx::query(
        "UPDATE entries SET stage='preparing',active_stage='preparing',trashed_at=? WHERE id=?",
    )
    .bind(now())
    .bind(&entry)
    .execute(&app.db)
    .await
    .unwrap();
    let chats = provider.chats.lock().unwrap().len();
    pipeline::regenerate_summary(&app, &entry).await.unwrap();
    assert_eq!(stage(&app, &entry).await, "paused");
    assert_eq!(provider.chats.lock().unwrap().len(), chats);
    assert_eq!(
        owner(&app, "DELETE", &format!("/api/v1/entries/{entry}")).await,
        204
    );
    task.abort();
}

#[tokio::test]
async fn restart_stops_work_interrupted_in_trash_without_resubmitting() {
    let (_dir, app, provider, task) = setup(Options::default()).await;
    for (id, step) in [
        ("trashed-summary", "enriching"),
        ("trashed-submit", "transcribing"),
    ] {
        sqlx::query("INSERT INTO entries(id,fingerprint,title,duration_ms,timezone,stage,active_stage,trashed_at,created_at,updated_at) VALUES(?,?,'Synthetic job',1,'UTC',?,?,?,?,?)")
            .bind(id).bind(id).bind(step).bind(step).bind(now()).bind(now()).bind(now()).execute(&app.db).await.unwrap();
    }
    sqlx::query("INSERT INTO transcriptions(entry_id,state,submitted_at) VALUES('trashed-submit','submitted',?)")
        .bind(now())
        .execute(&app.db)
        .await
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    pipeline::worker(app.clone(), stop).await;
    assert_eq!(stage(&app, "trashed-summary").await, "paused");
    let (stage_name, failure): (String, String) =
        sqlx::query_as("SELECT stage,failure FROM entries WHERE id='trashed-submit'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(stage_name, "failed");
    assert!(failure.contains("Retry explicitly"));
    assert_eq!(provider.submits(), 0);
    task.abort();
}

#[tokio::test]
async fn only_running_recordings_report_the_step_they_are_in() {
    let (_dir, app, _provider, task) = setup(Options::default()).await;
    for (id, step) in [
        ("failed-job", "failed"),
        ("paused-job", "paused"),
        ("queued-job", "queued"),
        ("running-job", "transcribing"),
    ] {
        // Stopped and waiting rows keep the step they last ran in storage.
        sqlx::query("INSERT INTO entries(id,fingerprint,title,duration_ms,timezone,stage,active_stage,created_at,updated_at) VALUES(?,?,'Synthetic job',1,'UTC',?,'transcribing',?,?)")
            .bind(id).bind(id).bind(step).bind(now()).bind(now()).execute(&app.db).await.unwrap();
    }
    for (id, reported) in [
        ("failed-job", serde_json::Value::Null),
        ("paused-job", serde_json::Value::Null),
        ("queued-job", serde_json::Value::Null),
        ("running-job", serde_json::json!("transcribing")),
    ] {
        let detail = retrieval::get(&app, id).await.unwrap();
        assert_eq!(detail["active_stage"], reported, "{id}");
    }
    task.abort();
}
