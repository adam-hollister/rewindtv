use std::collections::HashMap;

use anyhow::Context;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use serde_json::Value;
use url::Url;

use super::json::{arr, i, s, tags};
use super::{Library, RemoteItem};

const PAGE: usize = 500;
const CLIENT_ID: &str = "rewindtv";

#[derive(Clone)]
pub struct Plex {
    http: reqwest::Client,
    pub(super) base: Url,
    token: String,
}

impl Plex {
    pub fn new(http: reqwest::Client, base: Url, token: String) -> Self {
        Self { http, base, token }
    }

    pub fn auth_headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(&self.token) {
            h.insert("X-Plex-Token", v);
        }
        h.insert("X-Plex-Client-Identifier", HeaderValue::from_static(CLIENT_ID));
        h.insert("X-Plex-Product", HeaderValue::from_static("RewindTV"));
        h
    }

    fn url(&self, path: &str) -> Url {
        self.base.join(path).expect("valid plex path")
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> anyhow::Result<Value> {
        let res = self
            .http
            .get(self.url(path))
            .headers(self.auth_headers())
            .header(ACCEPT, "application/json")
            .query(query)
            .send()
            .await
            .with_context(|| format!("GET {path}"))?
            .error_for_status()?;
        let mut v: Value = res.json().await?;
        Ok(v.get_mut("MediaContainer").map(Value::take).unwrap_or(Value::Null))
    }

    pub async fn test(&self) -> anyhow::Result<String> {
        let mc = self.get("/", &[]).await?;
        Ok(s(&mc, "friendlyName").unwrap_or_else(|| "Plex".into()))
    }

    pub async fn libraries(&self) -> anyhow::Result<Vec<Library>> {
        let mc = self.get("/library/sections", &[]).await?;
        Ok(arr(&mc, "Directory")
            .iter()
            .filter_map(|d| {
                let kind = match s(d, "type")?.as_str() {
                    "movie" => "movie",
                    "show" => "show",
                    _ => return None,
                };
                Some(Library { key: s(d, "key")?, name: s(d, "title")?, kind: kind.into() })
            })
            .collect())
    }

    async fn all(&self, section: &str, kind: u8) -> anyhow::Result<Vec<Value>> {
        let mut out = Vec::new();
        loop {
            let mc = self
                .get(
                    &format!("/library/sections/{section}/all"),
                    &[
                        ("type", kind.to_string()),
                        ("X-Plex-Container-Start", out.len().to_string()),
                        ("X-Plex-Container-Size", PAGE.to_string()),
                    ],
                )
                .await?;
            let page = arr(&mc, "Metadata");
            out.extend_from_slice(page);
            let total = i(&mc, "totalSize").unwrap_or(0) as usize;
            if page.len() < PAGE || out.len() >= total {
                return Ok(out);
            }
        }
    }

    pub async fn items(&self, lib: &Library) -> anyhow::Result<Vec<RemoteItem>> {
        if lib.kind == "movie" {
            return Ok(self.all(&lib.key, 1).await?.iter().filter_map(movie).collect());
        }
        // Episodes don't carry genres/collections; inherit them from the show.
        let shows: HashMap<String, Value> = self
            .all(&lib.key, 2)
            .await?
            .into_iter()
            .filter_map(|v| Some((s(&v, "ratingKey")?, v)))
            .collect();
        Ok(self
            .all(&lib.key, 4)
            .await?
            .iter()
            .filter_map(|e| episode(e, shows.get(&s(e, "grandparentRatingKey")?)))
            .collect())
    }

    pub fn hls_url(&self, item_key: &str, offset_ms: i64, session: &str) -> Url {
        let mut u = self.url("/video/:/transcode/universal/start.m3u8");
        u.query_pairs_mut()
            .append_pair("path", &format!("/library/metadata/{item_key}"))
            .append_pair("mediaIndex", "0")
            .append_pair("partIndex", "0")
            .append_pair("protocol", "hls")
            .append_pair("offset", &(offset_ms / 1000).to_string())
            .append_pair("fastSeek", "1")
            .append_pair("directPlay", "0")
            .append_pair("directStream", "1")
            .append_pair("directStreamAudio", "0")
            .append_pair("videoQuality", "100")
            .append_pair("maxVideoBitrate", "20000")
            .append_pair("videoResolution", "1920x1080")
            .append_pair("audioBoost", "100")
            .append_pair("location", "lan")
            .append_pair("session", session)
            .append_pair("X-Plex-Session-Identifier", session)
            .append_pair("X-Plex-Client-Identifier", &format!("{CLIENT_ID}-{session}"))
            .append_pair("X-Plex-Product", "RewindTV")
            .append_pair("X-Plex-Platform", "Chrome")
            .append_pair(
                "X-Plex-Client-Profile-Extra",
                "add-transcode-target(type=videoProfile&context=streaming&protocol=hls&container=mpegts&videoCodec=h264&audioCodec=aac)",
            );
        u
    }

    /// Newer Plex servers want a transcode decision before `start.m3u8`.
    pub async fn decision(&self, hls: &Url) -> anyhow::Result<()> {
        let mut u = hls.clone();
        u.set_path(&hls.path().replace("start.m3u8", "decision"));
        self.http.get(u).headers(self.auth_headers()).header(ACCEPT, "application/json").send().await?;
        Ok(())
    }

    pub fn file_url(&self, part_key: &str) -> Option<Url> {
        let mut u = self.base.join(part_key).ok()?;
        u.query_pairs_mut().append_pair("X-Plex-Token", &self.token);
        Some(u)
    }

    pub async fn stop(&self, session: &str) -> anyhow::Result<()> {
        self.http
            .get(self.url("/video/:/transcode/universal/stop"))
            .headers(self.auth_headers())
            .query(&[("session", session)])
            .send()
            .await?;
        Ok(())
    }

    pub fn image_url(&self, path: &str, width: u32) -> Url {
        let mut u = self.url("/photo/:/transcode");
        u.query_pairs_mut()
            .append_pair("url", path)
            .append_pair("width", &width.to_string())
            .append_pair("height", &(width * 3 / 2).to_string())
            .append_pair("minSize", "1")
            .append_pair("upscale", "0");
        u
    }
}

fn part_key(v: &Value) -> Option<String> {
    let media = arr(v, "Media").first()?;
    s(arr(media, "Part").first()?, "key")
}

fn movie(v: &Value) -> Option<RemoteItem> {
    Some(RemoteItem {
        item_key: s(v, "ratingKey")?,
        part_key: part_key(v),
        kind: "movie",
        title: s(v, "title")?,
        year: i(v, "year"),
        duration_ms: i(v, "duration").filter(|d| *d > 0)?,
        summary: s(v, "summary"),
        content_rating: s(v, "contentRating"),
        studio: s(v, "studio"),
        thumb: s(v, "thumb"),
        art: s(v, "art"),
        genres: tags(v, "Genre"),
        collections: tags(v, "Collection"),
        ..Default::default()
    })
}

fn episode(v: &Value, show: Option<&Value>) -> Option<RemoteItem> {
    let show_val = show.cloned().unwrap_or(Value::Null);
    let year = i(v, "year")
        .or_else(|| s(v, "originallyAvailableAt").and_then(|d| d.get(..4)?.parse().ok()))
        .or_else(|| i(&show_val, "year"));
    Some(RemoteItem {
        item_key: s(v, "ratingKey")?,
        part_key: part_key(v),
        kind: "episode",
        title: s(v, "title")?,
        show_key: s(v, "grandparentRatingKey"),
        show_title: s(v, "grandparentTitle"),
        season: i(v, "parentIndex"),
        episode: i(v, "index"),
        year,
        duration_ms: i(v, "duration").filter(|d| *d > 0)?,
        summary: s(v, "summary"),
        content_rating: s(v, "contentRating").or_else(|| s(&show_val, "contentRating")),
        studio: s(&show_val, "studio"),
        thumb: s(v, "grandparentThumb").or_else(|| s(v, "thumb")),
        art: s(v, "grandparentArt").or_else(|| s(v, "art")),
        genres: tags(&show_val, "Genre"),
        collections: tags(&show_val, "Collection"),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_episode_with_show_metadata() {
        let show = serde_json::json!({
            "ratingKey": "10", "studio": "NBC", "contentRating": "TV-PG", "year": 1994,
            "Genre": [{"tag": "Comedy"}], "Collection": [{"tag": "90s Sitcoms"}]
        });
        let ep = serde_json::json!({
            "ratingKey": "11", "title": "The Pilot", "grandparentRatingKey": "10",
            "grandparentTitle": "Friends", "parentIndex": 1, "index": 1, "duration": 1320000,
            "grandparentThumb": "/library/metadata/10/thumb/1",
            "Media": [{"Part": [{"key": "/library/parts/5/1/file.mkv"}]}]
        });
        let item = episode(&ep, Some(&show)).unwrap();
        assert_eq!(item.show_title.as_deref(), Some("Friends"));
        assert_eq!(item.genres, vec!["Comedy"]);
        assert_eq!(item.collections, vec!["90s Sitcoms"]);
        assert_eq!(item.year, Some(1994));
        assert_eq!(item.part_key.as_deref(), Some("/library/parts/5/1/file.mkv"));
        assert_eq!((item.season, item.episode), (Some(1), Some(1)));
    }

    #[test]
    fn skips_movies_without_duration() {
        assert!(movie(&serde_json::json!({"ratingKey": "1", "title": "X"})).is_none());
    }
}
