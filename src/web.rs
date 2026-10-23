//! Serves the compiled web UI embedded in the binary.

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "web/dist"]
#[allow_missing = true]
struct Assets;

pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") || path.starts_with("stream/") || path.starts_with("iptv/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(file) = Assets::get(path).filter(|_| !path.is_empty()) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        // Vite fingerprints everything under assets/, so it can be cached forever.
        let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
        return ([(header::CONTENT_TYPE, mime.as_ref()), (header::CACHE_CONTROL, cache)], file.data).into_response();
    }
    // Client-side routes all land on index.html.
    match Assets::get("index.html") {
        Some(file) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], file.data).into_response(),
        None => (StatusCode::OK, "RewindTV is running, but the web UI was not built. Run `npm run build` in web/.").into_response(),
    }
}
