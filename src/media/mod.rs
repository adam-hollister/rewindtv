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
    /// The file's path as the server sees it.
    pub file_path: Option<String>,
    /// Server's last-modified time, used to refetch markers when it changes.
    pub updated_at: Option<i64>,
    /// Provider ids such as "tvdb:81189" or "imdb:tt0213338".
    pub provider_ids: Vec<String>,
    /// Intro/credits markers if the library listing included them.
    pub markers: Option<Vec<ServerMarker>>,
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

/// An intro/credits marker as reported by the media server (file time, ms).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ServerMarker {
    pub kind: crate::schedule::cuts::Kind,
    pub start_ms: i64,
    pub end_ms: i64,
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

    /// Intro/credits markers for one item. `Ok(None)` means the server can't
    /// provide markers at all (e.g. Jellyfin without a segments plugin).
    pub async fn markers(&self, item_key: &str) -> anyhow::Result<Option<Vec<ServerMarker>>> {
        match self {
            Source::Plex(p) => p.markers(item_key).await.map(Some),
            Source::Jellyfin(j) => j.markers(item_key).await,
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

    /// Resolve a server-relative path ("/library/…") against this server,
    /// refusing anything that would leave it ("//evil/x", "http://…", "\\…").
    /// Requests built from it carry credentials, so this must hold.
    pub fn local_url(&self, path_and_query: &str) -> Option<Url> {
        if !path_and_query.starts_with('/') || path_and_query.starts_with("//") || path_and_query.contains('\\') {
            return None;
        }
        let base = self.base();
        let mut origin = base.clone();
        origin.set_path("/");
        origin.set_query(None);
        let u = origin.join(path_and_query).ok()?;
        (u.scheme() == base.scheme() && u.host_str() == base.host_str() && u.port_or_known_default() == base.port_or_known_default()).then_some(u)
    }

    /// Query parameters that carry credentials and must never be echoed to browsers.
    pub const SECRET_PARAMS: &'static [&'static str] = &["X-Plex-Token", "api_key", "ApiKey", "X-Emby-Token"];
}

/// Mask credential values in text that may echo a URL (ffmpeg errors, logs).
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for key in Source::SECRET_PARAMS {
        let needle = format!("{key}=");
        let mut from = 0;
        while let Some(pos) = out[from..].to_ascii_lowercase().find(&needle.to_ascii_lowercase()) {
            let start = from + pos + needle.len();
            let end = out[start..].find(|c: char| c == '&' || c == '"' || c == '\'' || c.is_whitespace()).map_or(out.len(), |e| start + e);
            out.replace_range(start..end, "***");
            from = start + 3;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_tokens() {
        let s = "http://nas:32400/library/parts/1/file.mkv?X-Plex-Token=abc123: Server returned 404; http://jf/Videos/x/stream?static=true&api_key=ffee&MediaSourceId=x";
        let r = redact(s);
        assert!(!r.contains("abc123") && !r.contains("ffee"), "{r}");
        assert!(r.contains("X-Plex-Token=***") && r.contains("&MediaSourceId=x"));
    }

    #[test]
    fn local_url_never_leaves_the_server() {
        let s = Source::new(reqwest::Client::new(), "plex", "http://nas:32400", "t").unwrap();
        assert_eq!(s.local_url("/library/metadata/1/thumb/2?x=1").unwrap().as_str(), "http://nas:32400/library/metadata/1/thumb/2?x=1");
        for bad in ["//evil.com/x", "http://evil.com/x", "library/x", "/\\evil.com/x", "\\\\evil/x", "//nas:32400@evil.com/"] {
            assert!(s.local_url(bad).is_none(), "{bad}");
        }
    }
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
