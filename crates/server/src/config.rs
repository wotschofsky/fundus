use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, Notify};

#[derive(Clone)]
pub struct Config {
    pub data: PathBuf,
    pub bind: String,
    pub public_url: url::Url,
    pub openai_url: String,
    pub openai_key: String,
    pub transcription_model: String,
    pub text_model: String,
    pub embedding_model: String,
    pub voice_model: Option<PathBuf>,
    pub recorder_timezone: String,
    pub secure_cookie: bool,
    pub proxy_auth: bool,
}
impl Config {
    pub fn validate_for_serve(&self) -> Result<()> {
        anyhow::ensure!(
            !self.openai_key.trim().is_empty(),
            "OPENAI_API_KEY is required to start the server"
        );
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
            openai_url: std::env::var("OPENAI_BASE_URL")
                .unwrap_or("https://api.openai.com/v1".into())
                .trim_end_matches('/')
                .into(),
            openai_key: std::env::var("OPENAI_API_KEY")
                .unwrap_or_default()
                .trim()
                .to_string(),
            transcription_model: std::env::var("VAULT_TRANSCRIPTION_MODEL")
                .unwrap_or("gpt-4o-transcribe-diarize".into()),
            text_model: std::env::var("VAULT_TEXT_MODEL").unwrap_or("gpt-6-luna".into()),
            embedding_model: std::env::var("VAULT_EMBEDDING_MODEL")
                .unwrap_or("text-embedding-3-small".into()),
            voice_model: std::env::var_os("VAULT_VOICE_MODEL").map(PathBuf::from),
            recorder_timezone,
        })
    }
}
#[derive(Clone)]
pub struct App {
    pub db: sqlx::SqlitePool,
    pub config: Arc<Config>,
    pub client: reqwest::Client,
    pub wake: Arc<Notify>,
    pub upload_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    pub mutations: Arc<Mutex<()>>,
    pub login_attempts: Arc<Mutex<Vec<std::time::Instant>>>,
}
impl App {
    pub async fn new(config: Config) -> Result<Self> {
        use sqlx::sqlite::{
            SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
        };
        for folder in ["uploads", "entries", "work"] {
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
            .max_connections(5)
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
            upload_locks: Default::default(),
            mutations: Default::default(),
            login_attempts: Default::default(),
        })
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
