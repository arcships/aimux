//! aimux-web — local Web console (RFC-0029).
//!
//! Runs an axum server bound to `127.0.0.1` that serves the Vue SPA and the
//! `/api/*` endpoints, with recording / trace / session wiring enabled.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use axum::Router;
use clap::Parser;
use tower_http::services::ServeDir;

mod agents;
mod api;
mod model_builder;
mod state;
mod wire;

#[derive(Parser)]
#[command(
    name = "aimux-web",
    version,
    about = "aimux Web 控制台 — 浏览器端的 model call 验证与 trace 可视化 (RFC-0029)"
)]
struct Cli {
    /// Listen host (default 127.0.0.1; use 0.0.0.0 for LAN sharing).
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// Port (0 = random free port).
    #[arg(long, default_value_t = 0)]
    port: u16,
    /// Do not open the browser automatically.
    #[arg(long)]
    no_open: bool,
    /// Directory of built frontend assets (default: `web/dist` next to the crate).
    #[arg(long)]
    static_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let state = state::AppState::new();

    let static_dir = cli
        .static_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/dist"));

    let app = Router::new()
        .merge(api::router(state))
        .fallback_service(ServeDir::new(&static_dir).append_index_html_on_directories(true));

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    let url = format!("http://{local}");

    println!("aimux-web — RFC-0029 Web console");
    println!("  URL: {url}");
    println!("  static: {}", static_dir.display());
    println!("  wiring: RingRecorder(2048) + RingTraceStore + SessionStore");

    if !static_dir.join("index.html").exists() {
        eprintln!(
            "  warning: frontend not built — API only. Run `npm install && npm run build` in {}",
            static_dir
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );
    }

    if !cli.no_open {
        // Best-effort browser open (ignore failures on headless machines).
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }

    println!("  Ctrl-C to stop");
    axum::serve(listener, app).await?;
    Ok(())
}
