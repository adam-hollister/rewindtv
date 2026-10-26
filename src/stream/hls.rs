//! Reverse proxy for media-server HLS. Playlists are rewritten so every URL
//! points back at us and no credentials ever reach the browser.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use url::Url;

use crate::error::{ApiError, ApiResult};
use crate::media::Source;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ProxyQuery {
    u: Option<String>,
}

fn strip_secrets(u: &mut Url) {
    let kept: Vec<(String, String)> = u
        .query_pairs()
        .filter(|(k, _)| !Source::SECRET_PARAMS.iter().any(|s| s.eq_ignore_ascii_case(k)))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        u.set_query(None);
    } else {
        u.query_pairs_mut().clear().extend_pairs(kept);
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme() && a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default()
}

/// Map one URI found in a playlist to our proxy path.
fn proxied(uri: &str, playlist: &Url, base: &Url, sid: &str) -> String {
    let Ok(mut abs) = playlist.join(uri) else { return uri.to_string() };
    if !same_origin(&abs, base) {
        return abs.to_string();
    }
    strip_secrets(&mut abs);
    let rel = match abs.query() {
        Some(q) => format!("{}?{q}", abs.path()),
        None => abs.path().to_string(),
    };
    let enc: String = url::form_urlencoded::byte_serialize(rel.as_bytes()).collect();
    format!("/stream/{sid}/p?u={enc}")
}

pub fn rewrite_playlist(body: &str, playlist: &Url, base: &Url, sid: &str) -> String {
    let mut out = String::with_capacity(body.len() + 256);
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }
        if trimmed.starts_with('#') {
            // Tags like EXT-X-MEDIA / EXT-X-MAP / EXT-X-KEY carry URI="…".
            if let Some(start) = trimmed.find("URI=\"") {
                let rest = &trimmed[start + 5..];
                if let Some(end) = rest.find('"') {
                    let uri = &rest[..end];
                    out.push_str(&trimmed[..start + 5]);
                    out.push_str(&proxied(uri, playlist, base, sid));
                    out.push_str(&rest[end..]);
                    out.push('\n');
                    continue;
                }
            }
            out.push_str(trimmed);
        } else {
            out.push_str(&proxied(trimmed, playlist, base, sid));
        }
        out.push('\n');
    }
    out
}

/// GET /stream/:sid/master.m3u8 and /stream/:sid/p?u=…
pub async fn proxy(
    State(st): State<AppState>,
    Path((sid, rest)): Path<(String, String)>,
    Query(q): Query<ProxyQuery>,
    req_headers: HeaderMap,
) -> ApiResult<Response> {
    let session = st.sessions.touch(&sid).await.ok_or(ApiError::NotFound)?;
    let base = session.source.base().clone();
    let upstream = match (rest.as_str(), q.u) {
        ("master.m3u8", _) => session.master.clone(),
        ("p", Some(u)) if u.starts_with('/') => {
            let mut origin = base.clone();
            origin.set_path("/");
            origin.set_query(None);
            origin.join(&u).map_err(|e| ApiError::BadRequest(e.to_string()))?
        }
        _ => return Err(ApiError::NotFound),
    };

    let mut req = st.http.get(upstream.clone()).headers(session.source.session_headers(&sid));
    if let Some(range) = req_headers.get(header::RANGE) {
        req = req.header(header::RANGE, range);
    }
    let res = req.send().await.map_err(|e| ApiError::BadGateway(e.to_string()))?;
    let status = StatusCode::from_u16(res.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let ctype = res.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let is_playlist = ctype.contains("mpegurl") || upstream.path().ends_with(".m3u8");

    if is_playlist {
        let text = res.text().await.map_err(|e| ApiError::BadGateway(e.to_string()))?;
        if !status.is_success() {
            tracing::warn!("upstream playlist {status}: {}", text.chars().take(200).collect::<String>());
            return Ok((status, text).into_response());
        }
        let body = rewrite_playlist(&text, &upstream, &base, &sid);
        return Ok((
            status,
            [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl"), (header::CACHE_CONTROL, "no-cache")],
            body,
        )
            .into_response());
    }

    let mut out = Response::builder().status(status);
    for h in [header::CONTENT_TYPE, header::CONTENT_LENGTH, header::CONTENT_RANGE, header::ACCEPT_RANGES] {
        if let Some(v) = res.headers().get(&h) {
            out = out.header(h, v.as_bytes());
        }
    }
    Ok(out.body(Body::from_stream(res.bytes_stream())).map_err(anyhow::Error::from)?)
}

/// URI of the first variant in a master playlist, if it is one.
pub fn first_variant(text: &str) -> Option<&str> {
    let mut lines = text.lines().map(str::trim);
    lines.find(|l| l.starts_with("#EXT-X-STREAM-INF"))?;
    lines.find(|l| !l.is_empty() && !l.starts_with('#'))
}

/// Sum of segment durations in a media playlist.
pub fn extinf_total(text: &str) -> f64 {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("#EXTINF:"))
        .filter_map(|r| r.split(',').next()?.trim().parse::<f64>().ok())
        .sum()
}

/// Length in seconds of a session's playlist. Used to tell whether the server
/// started at the requested offset (Plex) or returned the whole program
/// (Jellyfin), in which case the reader has to seek.
pub async fn playlist_seconds(st: &AppState, sid: &str) -> Option<f64> {
    let s = st.sessions.touch(sid).await?;
    let headers = s.source.session_headers(sid);
    let get = |u: Url| {
        let req = st.http.get(u).headers(headers.clone());
        async move { req.send().await.ok()?.error_for_status().ok()?.text().await.ok() }
    };
    let master = get(s.master.clone()).await?;
    let media = match first_variant(&master) {
        Some(v) => get(s.master.join(v).ok()?).await?,
        None => master,
    };
    Some(extinf_total(&media))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_helpers() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nmain.m3u8?x=1\n";
        assert_eq!(first_variant(master), Some("main.m3u8?x=1"));
        assert_eq!(first_variant("#EXTM3U\n#EXTINF:6,\na.ts\n"), None);
        assert_eq!(extinf_total("#EXTINF:6.000000, nodesc\na.ts\n#EXTINF:4.5,\nb.ts\n"), 10.5);
    }

    #[test]
    fn rewrites_relative_and_absolute_uris_without_tokens() {
        let base = Url::parse("http://nas:32400").unwrap();
        let playlist = Url::parse("http://nas:32400/video/:/transcode/universal/start.m3u8?X-Plex-Token=secret").unwrap();
        let body = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nsession/abc/base/index.m3u8?X-Plex-Token=secret&x=1\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio/a.m3u8\"\nhttp://elsewhere/seg.ts\n";
        let out = rewrite_playlist(body, &playlist, &base, "sid1");
        assert!(!out.contains("secret"), "{out}");
        assert!(out.contains("/stream/sid1/p?u=%2Fvideo%2F%3A%2Ftranscode%2Funiversal%2Fsession%2Fabc%2Fbase%2Findex.m3u8%3Fx%3D1"), "{out}");
        assert!(out.contains("URI=\"/stream/sid1/p?u=%2Fvideo%2F%3A%2Ftranscode%2Funiversal%2Faudio%2Fa.m3u8\""), "{out}");
        assert!(out.contains("http://elsewhere/seg.ts"));
    }
}
