//! Embedded frontend serving (`embed-frontend` feature).
//!
//! The built SPA (`web/dist`) is compiled into the binary via `rust-embed`, so
//! a release executable is fully self-contained — users download one file and
//! run it, no npm step. The SPA uses hash routing, so every path falls back to
//! `index.html`.

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct Assets;

/// Serve an embedded asset; unknown paths fall back to `index.html` (the SPA
/// entry). Content type comes from the embedded metadata (no extra deps).
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    let asset = Assets::get(path).or_else(|| Assets::get("index.html"));
    match asset {
        Some(file) => (
            [(header::CONTENT_TYPE, file.metadata.mimetype())],
            file.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
