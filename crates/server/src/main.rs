use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use fundus::{
    auth,
    config::{App, Config},
};
#[derive(Parser)]
#[command(
    name = "fundus",
    version,
    about = "A private recording archive with native upload clients and MCP retrieval"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}
#[derive(Subcommand)]
enum Cmd {
    Serve,
    Init,
    Token {
        #[arg(long)]
        name: String,
        #[arg(long,default_value="read",value_parser=["upload","read"])]
        scope: String,
    },
    Backup {
        destination: std::path::PathBuf,
    },
    Healthcheck,
}
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or("fundus=info,tower_http=warn".into()),
        )
        .init();
    let cli = Cli::parse();
    let config = Config::from_env()?;
    if matches!(cli.command, Some(Cmd::Healthcheck)) {
        let bind = config.bind.replace("0.0.0.0", "127.0.0.1");
        reqwest::Client::new()
            .get(format!("http://{bind}/healthz"))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await?
            .error_for_status()?;
        return Ok(());
    }
    if matches!(cli.command, None | Some(Cmd::Serve)) {
        config.validate_for_serve()?;
    }
    tokio::fs::create_dir_all(&config.data).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config.data, std::fs::Permissions::from_mode(0o700))?;
    }
    let _process_lock = if matches!(
        cli.command,
        None | Some(Cmd::Serve) | Some(Cmd::Backup { .. })
    ) {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(config.data.join("server.lock"))?;
        file.try_lock()
            .context("Stop the running server before starting another server or making a backup")?;
        Some(file)
    } else {
        None
    };
    let app = App::new(config).await?;
    match cli.command.unwrap_or(Cmd::Serve) {
        Cmd::Init => {
            let password = match std::env::var("VAULT_OWNER_PASSWORD") {
                Ok(p) => p,
                Err(_) => {
                    use std::io::Read;
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s)?;
                    s.trim_end().to_string()
                }
            };
            auth::init_owner(&app, &password).await?;
            eprintln!("Owner initialized. Sign in through the web library.");
        }
        Cmd::Token { name, scope } => println!(
            "{}",
            auth::issue(&app, &name, &scope)
                .await
                .map_err(|e| anyhow::anyhow!(e.1))?
        ),
        Cmd::Backup { destination } => {
            anyhow::ensure!(!destination.exists(), "Backup destination already exists");
            tokio::fs::create_dir_all(&destination).await?;
            let db = destination.join("vault.sqlite");
            let backup_path = db.to_string_lossy().to_string();
            sqlx::query!("VACUUM INTO ?", backup_path)
                .execute(&app.db)
                .await?;
            copy_tree(
                &app.config.data.join("uploads"),
                &destination.join("uploads"),
            )?;
            copy_tree(
                &app.config.data.join("entries"),
                &destination.join("entries"),
            )?;
            eprintln!("Backup created while holding the exclusive server lock.");
        }
        Cmd::Serve => {
            let owner: Option<i64> = sqlx::query_scalar!("SELECT id FROM owner WHERE id=1")
                .fetch_optional(&app.db)
                .await?;
            anyhow::ensure!(
                app.config.proxy_auth || owner.is_some(),
                "Run fundus init first (password from stdin or VAULT_OWNER_PASSWORD)"
            );
            let listener = tokio::net::TcpListener::bind(&app.config.bind)
                .await
                .context("Could not bind HTTP listener")?;
            let shutdown = tokio_util::sync::CancellationToken::new();
            let worker = tokio::spawn(fundus::pipeline::worker(app.clone(), shutdown.clone()));
            let router = fundus::api::router(app.clone(), shutdown.clone());
            tracing::info!(address=%app.config.bind,"HTTP server listening; TLS belongs to the reverse proxy");
            let stop = shutdown.clone();
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    shutdown_signal().await;
                    stop.cancel();
                })
                .await?;
            shutdown.cancel();
            worker.await?;
            app.db.close().await;
        }
        Cmd::Healthcheck => unreachable!(),
    }
    Ok(())
}
fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for item in std::fs::read_dir(from)? {
        let item = item?;
        if item.file_type()?.is_dir() {
            copy_tree(&item.path(), &to.join(item.file_name()))?
        } else {
            std::fs::copy(item.path(), to.join(item.file_name()))?;
        }
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
