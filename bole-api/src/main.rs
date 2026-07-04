// bole-3xj5
//! bole-api server binary.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use bole_api::config::AuthConfig;
use bole_api::{build_router, AppState};

#[derive(Parser)]
#[command(name = "bole-api", version, about = "HTTP/JSON read API over a bole repository")]
struct Cli {
    /// Path to the `.bole` store directory.
    #[arg(long)]
    store: PathBuf,
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: String,
    /// Optional auth config (TOML). Absent ⇒ all requests are anonymous.
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let repo = bole::Repository::disk(&cli.store).await?;
    let config = match cli.config {
        Some(path) => AuthConfig::parse(&std::fs::read_to_string(path)?)?,
        None => AuthConfig::default(),
    };
    let state = AppState { repo: Arc::new(repo), config: Arc::new(config) };

    let listener = tokio::net::TcpListener::bind(&cli.listen).await?;
    tracing::info!("bole-api listening on {}", cli.listen);
    axum::serve(listener, build_router(state)).await?;
    Ok(())
}
