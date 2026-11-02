//! Channel logos: uploaded images or artwork picked from the channel's shows,
//! stored as files in the logos folder. Channels without one get a generated
//! number badge.

use std::collections::HashSet;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use super::channels::img;
use crate::error::{ApiError, ApiResult};
use crate::schedule::store;
use crate::state::AppState;

pub const MAX_LOGO_BYTES: usize = 3 * 1024 * 1024;

/// File extension for an accepted image, judged by its bytes (never the name).
/// Only raster formats: SVG can carry scripts.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    }
}

/// A logo file name we created: "<id>-<hash>.<ext>", nothing else.
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !name.starts_with('.')
        && !name.contains("..")
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3))
}

pub async fn remove_file(st: &AppState, name: Option<&str>) {
    if let Some(n) = name.filter(|n| safe_name(n)) {
        let _ = tokio::fs::remove_file(st.cfg.logos_dir.join(n)).await;
    }
}

/// Validate, store and attach a logo to a channel, replacing any old one.
pub async fn store_logo(st: &AppState, channel_id: i64, bytes: &[u8]) -> ApiResult<String> {
    if bytes.len() > MAX_LOGO_BYTES {
        return Err(ApiError::BadRequest("Logos can be up to 3 MB".into()));
    }
    let ext = sniff(bytes).ok_or_else(|| ApiError::BadRequest("Use a PNG, JPEG or WebP image".into()))?;
    let old: Option<String> = sqlx::query_scalar::<_, Option<String>>("SELECT logo FROM channels WHERE id = ?")
        .bind(channel_id)
        .fetch_optional(&st.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    let name = format!("{channel_id}-{:016x}.{ext}", fnv1a(bytes));
    tokio::fs::create_dir_all(&st.cfg.logos_dir).await?;
    let tmp = st.cfg.logos_dir.join(format!("{name}.tmp"));
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, st.cfg.logos_dir.join(&name)).await?;
    sqlx::query("UPDATE channels SET logo = ? WHERE id = ?").bind(&name).bind(channel_id).execute(&st.db).await?;
    if old.as_deref() != Some(name.as_str()) {
        remove_file(st, old.as_deref()).await;
    }
    Ok(name)
}

#[derive(Serialize)]
pub struct LogoOut {
    logo: Option<String>,
}

/// PUT /api/channels/:id/logo — raw image bytes.
pub async fn upload(State(st): State<AppState>, Path(id): Path<i64>, body: Bytes) -> ApiResult<Json<LogoOut>> {
    let name = store_logo(&st, id, &body).await?;
    Ok(Json(LogoOut { logo: Some(name) }))
}

#[derive(Deserialize)]
pub struct FromArt {
    source_id: i64,
    path: String,
}

/// POST /api/channels/:id/logo/from-art — use a show's poster or backdrop.
pub async fn from_art(State(st): State<AppState>, Path(id): Path<i64>, Json(a): Json<FromArt>) -> ApiResult<Json<LogoOut>> {
    let src = st.source(a.source_id).await?;
    // Same guard as the artwork proxy: requests carry the server's credentials.
    let original = src.local_url(&a.path).ok_or_else(|| ApiError::BadRequest("bad image path".into()))?;
    let mut bytes = None;
    for url in [src.image_url(&a.path, 600), original] {
        if let Ok(res) = st.http.get(url).headers(src.auth_headers()).timeout(std::time::Duration::from_secs(30)).send().await {
            if res.status().is_success() {
                if let Ok(b) = res.bytes().await {
                    if sniff(&b).is_some() {
                        bytes = Some(b);
                        break;
                    }
                }
            }
        }
    }
    let bytes = bytes.ok_or_else(|| ApiError::BadGateway("Couldn't download that artwork".into()))?;
    let name = store_logo(&st, id, &bytes).await?;
    Ok(Json(LogoOut { logo: Some(name) }))
}

/// DELETE /api/channels/:id/logo — back to the number badge.
pub async fn clear(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<LogoOut>> {
    let old: Option<String> = sqlx::query_scalar::<_, Option<String>>("SELECT logo FROM channels WHERE id = ?")
        .bind(id)
        .fetch_optional(&st.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    sqlx::query("UPDATE channels SET logo = NULL WHERE id = ?").bind(id).execute(&st.db).await?;
    remove_file(&st, old.as_deref()).await;
    Ok(Json(LogoOut { logo: None }))
}

/// GET /api/channels/:id/logo (and the older …/logo.svg) — the uploaded logo,
/// or a generated number badge.
pub async fn serve(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Response> {
    let (number, name, color, logo): (i64, String, String, Option<String>) =
        sqlx::query_as("SELECT number, name, color, logo FROM channels WHERE id = ?")
            .bind(id)
            .fetch_optional(&st.db)
            .await?
            .ok_or(ApiError::NotFound)?;
    if let Some(file) = logo.filter(|n| safe_name(n)) {
        if let Ok(bytes) = tokio::fs::read(st.cfg.logos_dir.join(&file)).await {
            return Ok((
                [
                    (header::CONTENT_TYPE, content_type(&file)),
                    (header::CACHE_CONTROL, "public, max-age=3600"),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                ],
                bytes,
            )
                .into_response());
        }
    }
    Ok((
        [(header::CONTENT_TYPE, "image/svg+xml"), (header::CACHE_CONTROL, "no-cache"), (header::X_CONTENT_TYPE_OPTIONS, "nosniff")],
        badge_svg(number, &name, &color),
    )
        .into_response())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The generated channel "bug": the number on the channel's color.
pub fn badge_svg(number: i64, name: &str, color: &str) -> String {
    let short: String = name.chars().take(14).collect();
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 300" width="400" height="300">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{color}"/><stop offset="1" stop-color="{color}" stop-opacity="0.55"/></linearGradient></defs>
<rect width="400" height="300" rx="36" fill="#111318"/>
<rect x="14" y="14" width="372" height="272" rx="26" fill="url(#g)"/>
<text x="200" y="170" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-weight="800" font-size="130" fill="#fff">{number}</text>
<text x="200" y="240" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-weight="700" font-size="36" letter-spacing="2" fill="#fff" fill-opacity="0.92">{name}</text>
</svg>"##,
        color = esc(color),
        name = esc(&short.to_uppercase()),
    )
}

#[derive(Serialize)]
pub struct ArtChoice {
    title: String,
    source_id: i64,
    /// Server paths to pass back to from-art, plus proxied URLs to display.
    poster: Option<String>,
    backdrop: Option<String>,
    poster_url: Option<String>,
    backdrop_url: Option<String>,
}

/// GET /api/channels/:id/art-choices — artwork from shows/movies on this channel.
pub async fn art_choices(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Vec<ArtChoice>>> {
    let cfg = store::load_config(&st.db, id).await?.ok_or(ApiError::NotFound)?;
    let items = st.items().await?;
    // One item per show (or movie), up to 24.
    let mut seen: HashSet<String> = HashSet::new();
    let ids: Vec<i64> = items
        .iter()
        .filter(|it| cfg.rule.matches(it) || cfg.slots.iter().any(|s| s.rule.matches(it)))
        .filter(|it| seen.insert(it.group_key()))
        .take(24)
        .map(|it| it.id)
        .collect();
    if ids.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    let rows: Vec<(i64, String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(&format!(
        "SELECT source_id, title, show_title, thumb, art FROM media_items WHERE id IN ({list})"
    ))
    .fetch_all(&st.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .filter(|r| r.3.is_some() || r.4.is_some())
            .map(|(source_id, title, show, thumb, art)| ArtChoice {
                title: show.unwrap_or(title),
                source_id,
                poster_url: img(Some(source_id), &thumb, 300),
                backdrop_url: img(Some(source_id), &art, 400),
                poster: thumb,
                backdrop: art,
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_raster_images() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(sniff(b"\xFF\xD8\xFF\xE0rest"), Some("jpg"));
        assert_eq!(sniff(b"RIFF\x10\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>"), None);
        assert_eq!(sniff(b"<html><script>x</script>"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn file_names_cant_escape_the_folder() {
        assert!(safe_name("4-00ff00ff00ff00ff.png"));
        for bad in ["../x.png", "a/b.png", ".hidden", "x..png", "", "a b.png"] {
            assert!(!safe_name(bad), "{bad}");
        }
    }
}
