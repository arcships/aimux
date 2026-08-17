//! aimux-web — local Web console (RFC-0029).
//!
//! Runs an axum server bound to `127.0.0.1` that serves the Vue SPA and the
//! `/api/*` endpoints, with recording / trace / session wiring enabled.
//!
//! Two frontend serving modes:
//! - default (dev): serve `web/dist` from disk (`ServeDir`), so the frontend
//!   can be rebuilt / hot-reloaded independently;
//! - `--features embed-frontend` (release / distribution): the built frontend
//!   is embedded into the binary with `rust-embed`, producing a self-contained
//!   executable users can download with no npm step.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use axum::Router;
use clap::Parser;

mod agents;
mod api;
mod model_builder;
mod settings;
mod state;
mod wire;

#[cfg(feature = "embed-frontend")]
mod embed;

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
    /// Directory of built frontend assets (default: `web/dist` next to the
    /// crate). Ignored when the `embed-frontend` feature is enabled.
    #[arg(long)]
    static_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let state = state::AppState::with_bind_host(&cli.host);
    let loopback = state.loopback;
    let key_store = state.keys.clone();

    let app = Router::new().merge(api::router(state));

    #[cfg(feature = "embed-frontend")]
    let app = app.fallback(embed::serve);
    #[cfg(not(feature = "embed-frontend"))]
    let app = {
        let static_dir = cli
            .static_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/dist"));
        if !static_dir.join("index.html").exists() {
            eprintln!(
                "  warning: frontend not built — API only. Run `npm install && npm run build` in {}",
                static_dir
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
        }
        app.fallback_service(
            tower_http::services::ServeDir::new(&static_dir).append_index_html_on_directories(true),
        )
    };

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    let url = format!("http://{local}");

    println!("aimux-web — RFC-0029 Web console");
    println!("  URL: {url}");
    #[cfg(feature = "embed-frontend")]
    println!("  frontend: embedded in binary");
    #[cfg(not(feature = "embed-frontend"))]
    println!(
        "  static: {}",
        cli.static_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/dist"))
            .display()
    );
    println!("  wiring: RingRecorder(2048) + RingTraceStore + SessionStore");
    if !key_store.is_empty() {
        println!(
            "  key store: {} provider key(s) loaded from Settings",
            key_store.len()
        );
    }
    if !loopback {
        println!("  note: non-loopback bind — web key entry is disabled, use env:VAR references");
    }

    if !cli.no_open {
        // Best-effort browser open (ignore failures on headless machines).
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }

    println!("  Ctrl-C to stop");
    axum::serve(listener, app).await?;
    Ok(())
}
