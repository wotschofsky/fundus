use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, Notify, Semaphore};
use tokio_util::sync::CancellationToken;

/// Disk space uploads and video conversions leave free: the database may share the disk, and a
/// full disk breaks more than them.
pub const RESERVE: u64 = 5 * 1024 * 1024 * 1024;
/// The most a video conversion writes per second of recording: x264's 3 Mbit/s ceiling and
/// 128 kbit/s AAC (`video::arguments`), with 5% for the container.
pub const CONVERSION_BYTES_PER_SECOND: u64 = (3_000_000 + 128_000) / 8 * 105 / 100;
/// Bytes an unprivileged writer may still use at `path`. An error rather than a guess.
pub fn free_space(path: &Path) -> std::io::Result<u64> {
    let stats = rustix::fs::statvfs(path)?;
    Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
}
/// Whether `bytes` more still leave `RESERVE` free after what is already promised: what the
/// uploads of the last day still have to send (one abandoned for longer stops holding its
/// space), and the largest file a running conversion may write. Uploads and conversions both
/// ask, so neither spends the space the other counts on.
pub async fn room_for(app: &App, bytes: u64) -> Result<bool> {
    let since = (chrono::Utc::now() - chrono::Duration::days(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let promised = sqlx::query!("SELECT (SELECT COALESCE(SUM(u.length-u.offset),0) FROM uploads u JOIN imports i ON i.id=u.import_id WHERE i.state='uploading' AND i.created_at>?) AS \"uploads!: i64\",(SELECT COALESCE(SUM(duration_ms),0) FROM entries WHERE video_status='converting') AS \"converting_ms!: i64\"", since)
        .fetch_one(&app.db)
        .await?;
    let converting = promised.converting_ms.max(0) as u64 * CONVERSION_BYTES_PER_SECOND / 1000;
    let promised = promised.uploads.max(0) as u64 + converting;
    // ponytail: one disk for uploads and recordings, as on the server today. Moving the
    // originals to their own disk (personal-cloud backlog) splits these promises by disk.
    let free = free_space(&app.config.data.join("uploads"))?;
    Ok(free >= RESERVE + promised + bytes)
}

#[derive(Clone)]
pub struct Config {
    pub data: PathBuf,
    pub bind: String,
    pub public_url: url::Url,
    pub assemblyai_url: String,
    pub assemblyai_key: String,
    pub transcription_models: Vec<String>,
    pub transcription_languages: Vec<String>,
    /// How often a submitted AssemblyAI job is polled.
    pub poll_interval: std::time::Duration,
    /// OpenAI-compatible endpoint for the notes (titles, summaries, name evidence) and the
    /// embeddings for semantic search. Empty key: neither; entries get a visible warning and
    /// search uses keywords only.
    pub openai_url: String,
    pub openai_key: String,
    pub text_model: String,
    pub embedding_model: String,
    pub voice_model: Option<PathBuf>,
    /// Built web app, served from memory.
    pub web: PathBuf,
    pub recorder_timezone: String,
    pub secure_cookie: bool,
    pub proxy_auth: bool,
}
impl Config {
    pub fn validate_for_serve(&self) -> Result<()> {
        anyhow::ensure!(
            !self.assemblyai_key.is_empty(),
            "ASSEMBLYAI_API_KEY is required to start the server"
        );
        anyhow::ensure!(
            self.web.join("index.html").is_file(),
            "No web app in {}: run pnpm build or set VAULT_WEB_DIR",
            self.web.display()
        );
        if self.openai_key.is_empty() {
            tracing::warn!(
                "OPENAI_API_KEY is not set: recordings get no titles, summaries or name evidence, and search uses keywords only"
            );
        }
        Ok(())
    }
    pub fn from_env() -> Result<Self> {
        let public_url = url::Url::parse(
            &std::env::var("VAULT_PUBLIC_URL").unwrap_or("http://localhost:3000".into()),
        )?;
        if !["http", "https"].contains(&public_url.scheme())
            || public_url.host_str().is_none()
            || !public_url.username().is_empty()
            || public_url.password().is_some()
            || public_url.query().is_some()
            || public_url.fragment().is_some()
            || public_url.path() != "/"
        {
            bail!("VAULT_PUBLIC_URL must be an HTTP(S) origin without a path or credentials");
        }
        let data = PathBuf::from(std::env::var("VAULT_DATA_DIR").unwrap_or("data".into()));
        let recorder_timezone = std::env::var("VAULT_RECORDER_TIMEZONE").unwrap_or("UTC".into());
        recorder_timezone
            .parse::<chrono_tz::Tz>()
            .context("Invalid recorder timezone")?;
        Ok(Self {
            proxy_auth: match std::env::var("VAULT_AUTH_MODE")
                .as_deref()
                .unwrap_or("password")
            {
                "password" => false,
                "proxy" => true,
                _ => bail!("VAULT_AUTH_MODE must be password or proxy"),
            },
            secure_cookie: public_url.scheme() == "https",
            public_url,
            data,
            bind: std::env::var("VAULT_BIND").unwrap_or("127.0.0.1:3000".into()),
            assemblyai_url: base_url("ASSEMBLYAI_BASE_URL", "https://api.assemblyai.com"),
            assemblyai_key: secret("ASSEMBLYAI_API_KEY"),
            transcription_models: list("VAULT_TRANSCRIPTION_MODEL", "universal-3-5-pro"),
            transcription_languages: list("VAULT_TRANSCRIPTION_LANGUAGES", "de,en"),
            poll_interval: std::time::Duration::from_secs(5),
            openai_url: base_url("OPENAI_BASE_URL", "https://api.openai.com/v1"),
            openai_key: secret("OPENAI_API_KEY"),
            text_model: std::env::var("VAULT_TEXT_MODEL").unwrap_or("gpt-6-luna".into()),
            embedding_model: std::env::var("VAULT_EMBEDDING_MODEL")
                .unwrap_or("text-embedding-3-small".into()),
            voice_model: std::env::var_os("VAULT_VOICE_MODEL").map(PathBuf::from),
            web: PathBuf::from(std::env::var("VAULT_WEB_DIR").unwrap_or("web/dist".into())),
            recorder_timezone,
        })
    }
}
fn base_url(name: &str, default: &str) -> String {
    std::env::var(name)
        .unwrap_or(default.into())
        .trim_end_matches('/')
        .into()
}
fn secret(name: &str) -> String {
    std::env::var(name).unwrap_or_default().trim().to_string()
}
fn list(name: &str, default: &str) -> Vec<String> {
    std::env::var(name)
        .unwrap_or(default.into())
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
        .collect()
}
#[derive(Clone)]
pub struct App {
    pub db: sqlx::SqlitePool,
    pub config: Arc<Config>,
    pub client: reqwest::Client,
    pub wake: Arc<Notify>,
    /// Local CPU-heavy steps (decoding, rendering, voice analysis) running at once. The
    /// container has 1.5 CPUs and 1.5 GB; everything else is waiting on providers.
    pub cpu: Arc<Semaphore>,
    pub upload_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    pub mutations: Arc<Mutex<()>>,
    pub background: tokio_util::task::TaskTracker,
    pub login_attempts: Arc<Mutex<Vec<std::time::Instant>>>,
    /// The running video conversion, by entry, so Trash can stop it (`video::convert`).
    pub conversions: Arc<std::sync::Mutex<HashMap<String, CancellationToken>>>,
}
impl App {
    pub async fn new(config: Config) -> Result<Self> {
        use sqlx::sqlite::{
            SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
        };
        for folder in ["uploads", "entries"] {
            tokio::fs::create_dir_all(config.data.join(folder)).await?;
        }
        let options = SqliteConnectOptions::new()
            .filename(config.data.join("vault.sqlite"))
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(std::time::Duration::from_secs(10));
        let db = SqlitePoolOptions::new()
            .max_connections(10)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&db).await?;
        Ok(Self {
            db,
            config: Arc::new(config),
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(20))
                .timeout(std::time::Duration::from_secs(1200))
                .build()?,
            wake: Arc::new(Notify::new()),
            cpu: Arc::new(Semaphore::new(2)),
            upload_locks: Default::default(),
            mutations: Default::default(),
            background: Default::default(),
            login_attempts: Default::default(),
            conversions: Default::default(),
        })
    }
    /// Stops `entry`'s video conversion if it is running; the video goes back to the queue.
    pub fn stop_conversion(&self, entry: &str) {
        if let Some(stop) = self.conversions.lock().unwrap().get(entry) {
            stop.cancel();
        }
    }
    pub fn upload_path(&self, id: &str) -> PathBuf {
        self.config.data.join("uploads").join(id)
    }
    pub fn entry_dir(&self, id: &str) -> PathBuf {
        self.config.data.join("entries").join(id)
    }
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
