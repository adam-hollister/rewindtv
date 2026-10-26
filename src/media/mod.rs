//! Clients for the media servers we pull libraries and streams from.

mod jellyfin;
mod plex;

use reqwest::header::HeaderMap;
use serde::Serialize;
use url::Url;

pub use jellyfin::Jellyfin;
pub use plex::Plex;

#[derive(Debug, Clone, Serialize)]
pub struct Library {
    pub key: String,
    pub name: String,
    /// "movie" or "show"
    pub kind: String,
}

#[derive(Debug, Clone, Default)]
pub struct RemoteItem {
    pub item_key: String,
    pub part_key: Option<String>,
    pub video_codec: Option<String>,
    pub kind: &'static str,
    pub title: String,
    pub show_key: Option<String>,
    pub show_title: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub year: Option<i64>,
    pub duration_ms: i64,
    pub summary: Option<String>,
    pub content_rating: Option<String>,
    pub studio: Option<String>,
    pub thumb: Option<String>,
    pub art: Option<String>,
    pub genres: Vec<String>,
    pub collections: Vec<String>,
}

/// One configured server. Constructed from a `sources` row whenever needed;
/// cheap because the underlying HTTP client is shared.
#[derive(Clone)]
pub enum Source {
    Plex(Plex),
    Jellyfin(Jellyfin),
}

impl Source {
    pub fn new(http: reqwest::Client, kind: &str, url: &str, token: &str) -> anyhow::Result<Self> {
        let base = Url::parse(url.trim_end_matches('/'))?;
        Ok(match kind {
            "plex" => Source::Plex(Plex::new(http, base, token.to_string())),
            "jellyfin" => Source::Jellyfin(Jellyfin::new(http, base, token.to_string())),
            other => anyhow::bail!("unknown source kind {other}"),
        })
    }

    /// Returns the server's display name.
    pub async fn test(&self) -> anyhow::Result<String> {
        match self {
            Source::Plex(p) => p.test().await,
            Source::Jellyfin(j) => j.test().await,
        }
    }

    pub async fn libraries(&self) -> anyhow::Result<Vec<Library>> {
        match self {
            Source::Plex(p) => p.libraries().await,
            Source::Jellyfin(j) => j.libraries().await,
        }
    }

    pub async fn items(&self, lib: &Library) -> anyhow::Result<Vec<RemoteItem>> {
        match self {
            Source::Plex(p) => p.items(lib).await,
            Source::Jellyfin(j) => j.items(lib).await,
        }
    }

    /// HLS master playlist for browser playback, transcoded/remuxed by the
    /// media server to h264/aac. `offset_ms` is a hint; some servers return a
    /// full-length playlist and the player seeks instead.
    pub fn hls_url(&self, item_key: &str, part_key: Option<&str>, video_codec: Option<&str>, offset_ms: i64, session: &str) -> Url {
        match self {
            Source::Plex(p) => p.hls_url(item_key, offset_ms, session, video_codec == Some("h264")),
            Source::Jellyfin(j) => j.hls_url(item_key, part_key, session),
        }
    }

    /// HLS for the Live TV tuner feed: always transcoded to identical
    /// 1080p H.264 + stereo AAC so programs can be stitched together.
    pub fn tuner_url(&self, item_key: &str, part_key: Option<&str>, offset_ms: i64, session: &str) -> Url {
        match self {
            Source::Plex(p) => p.tuner_url(item_key, offset_ms, session),
            Source::Jellyfin(j) => j.tuner_url(item_key, part_key, session),
        }
    }

    /// Direct (untranscoded) file URL, used by ffmpeg for IPTV streams.
    /// Includes the token since it is only ever handed to our own ffmpeg.
    pub fn file_url(&self, item_key: &str, part_key: Option<&str>) -> Option<Url> {
        match self {
            Source::Plex(p) => p.file_url(part_key?),
            Source::Jellyfin(j) => Some(j.file_url(item_key)),
        }
    }

    /// Some servers need a handshake before the HLS URL can be fetched.
    pub async fn prepare(&self, hls: &Url) {
        if let Source::Plex(p) = self {
            if let Err(e) = p.decision(hls).await {
                tracing::debug!("plex decision: {e:#}");
            }
        }
    }

    pub async fn stop(&self, session: &str) {
        let res = match self {
            Source::Plex(p) => p.stop(session).await,
            Source::Jellyfin(j) => j.stop(session).await,
        };
        if let Err(e) = res {
            tracing::debug!("stopping session {session}: {e:#}");
        }
    }

    pub fn image_url(&self, path: &str, width: u32) -> Url {
        match self {
            Source::Plex(p) => p.image_url(path, width),
            Source::Jellyfin(j) => j.image_url(path, width),
        }
    }

    pub fn auth_headers(&self) -> HeaderMap {
        match self {
            Source::Plex(p) => p.auth_headers(),
            Source::Jellyfin(j) => j.auth_headers(),
        }
    }

    /// Auth headers for requests that belong to one playback session.
    pub fn session_headers(&self, session: &str) -> HeaderMap {
        match self {
            Source::Plex(p) => p.session_headers(session),
            Source::Jellyfin(j) => j.auth_headers(),
        }
    }

    pub fn base(&self) -> &Url {
        match self {
            Source::Plex(p) => &p.base,
            Source::Jellyfin(j) => &j.base,
        }
    }

    /// Query parameters that carry credentials and must never be echoed to browsers.
    pub const SECRET_PARAMS: &'static [&'static str] = &["X-Plex-Token", "api_key", "ApiKey", "X-Emby-Token"];
}

/// Small helpers for picking fields out of loosely-typed JSON responses.
pub(crate) mod json {
    use serde_json::Value;

    pub fn s(v: &Value, k: &str) -> Option<String> {
        match v.get(k)? {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }

    pub fn i(v: &Value, k: &str) -> Option<i64> {
        match v.get(k)? {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
        v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Plex style `[{ "tag": "Comedy" }]` lists.
    pub fn tags(v: &Value, k: &str) -> Vec<String> {
        arr(v, k).iter().filter_map(|t| s(t, "tag")).collect()
    }
}
