use std::collections::HashMap;

use anyhow::Context;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde_json::Value;
use url::Url;

use super::json::{arr, i, s};
use super::{Library, RemoteItem};

const PAGE: usize = 500;
const TICKS_PER_MS: i64 = 10_000;

#[derive(Clone)]
pub struct Jellyfin {
    http: reqwest::Client,
    pub(super) base: Url,
    token: String,
}

impl Jellyfin {
    pub fn new(http: reqwest::Client, base: Url, token: String) -> Self {
        Self { http, base, token }
    }

    pub fn auth_headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        let auth = format!(
            r#"MediaBrowser Client="RewindTV", Device="RewindTV", DeviceId="rewindtv", Version="{}", Token="{}""#,
            env!("CARGO_PKG_VERSION"),
            self.token
        );
        if let Ok(v) = HeaderValue::from_str(&auth) {
            h.insert(AUTHORIZATION, v);
        }
        if let Ok(v) = HeaderValue::from_str(&self.token) {
            h.insert("X-Emby-Token", v);
        }
        h
    }

    fn url(&self, path: &str) -> Url {
        // Keep any base path (e.g. http://host/jellyfin) when joining.
        let mut u = self.base.clone();
        let joined = format!("{}{}", u.path().trim_end_matches('/'), path);
        u.set_path(&joined);
        u
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> anyhow::Result<Value> {
        Ok(self
            .http
            .get(self.url(path))
            .headers(self.auth_headers())
            .query(query)
            .send()
            .await
            .with_context(|| format!("GET {path}"))?
            .error_for_status()?
            .json()
            .await?)
    }

    pub async fn test(&self) -> anyhow::Result<String> {
        let info = self.get("/System/Info", &[]).await?;
        Ok(s(&info, "ServerName").unwrap_or_else(|| "Jellyfin".into()))
    }

    pub async fn libraries(&self) -> anyhow::Result<Vec<Library>> {
        let folders = self.get("/Library/VirtualFolders", &[]).await?;
        Ok(folders
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|f| {
                let kind = match s(f, "CollectionType").as_deref() {
                    Some("movies") => "movie",
                    Some("tvshows") => "show",
                    // Untyped "mixed" folders may hold either.
                    None | Some("mixed") | Some("homevideos") => "mixed",
                    _ => return None,
                };
                Some(Library { key: s(f, "ItemId")?, name: s(f, "Name")?, kind: kind.into() })
            })
            .collect())
    }

    async fn query_all(&self, parent: Option<&str>, types: &str, fields: &str) -> anyhow::Result<Vec<Value>> {
        let mut out = Vec::new();
        loop {
            let mut q = vec![
                ("Recursive", "true".to_string()),
                ("IncludeItemTypes", types.to_string()),
                ("Fields", fields.to_string()),
                ("StartIndex", out.len().to_string()),
                ("Limit", PAGE.to_string()),
                ("EnableImageTypes", "Primary,Backdrop,Thumb".to_string()),
            ];
            if let Some(p) = parent {
                q.push(("ParentId", p.to_string()));
            }
            let res = self.get("/Items", &q).await?;
            let page = arr(&res, "Items");
            out.extend_from_slice(page);
            let total = i(&res, "TotalRecordCount").unwrap_or(0) as usize;
            if page.len() < PAGE || out.len() >= total {
                return Ok(out);
            }
        }
    }

    /// Map of item id -> collection (BoxSet) names, covering both movies and series.
    async fn collections(&self) -> anyhow::Result<HashMap<String, Vec<String>>> {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for set in self.query_all(None, "BoxSet", "").await? {
            let (Some(id), Some(name)) = (s(&set, "Id"), s(&set, "Name")) else { continue };
            let members = self
                .get("/Items", &[("ParentId", id), ("Fields", String::new())])
                .await
                .unwrap_or(Value::Null);
            for m in arr(&members, "Items") {
                if let Some(mid) = s(m, "Id") {
                    map.entry(mid).or_default().push(name.clone());
                }
            }
        }
        Ok(map)
    }

    pub async fn items(&self, lib: &Library) -> anyhow::Result<Vec<RemoteItem>> {
        const FIELDS: &str = "Genres,Studios,Overview,OfficialRating,ProductionYear,PremiereDate";
        let collections = self.collections().await.unwrap_or_default();
        let series: HashMap<String, Value> = self
            .query_all(Some(&lib.key), "Series", FIELDS)
            .await?
            .into_iter()
            .filter_map(|v| Some((s(&v, "Id")?, v)))
            .collect();
        let items = self.query_all(Some(&lib.key), "Movie,Episode", FIELDS).await?;
        Ok(items
            .iter()
            .filter_map(|v| {
                let show = s(v, "SeriesId").and_then(|id| series.get(&id));
                let mut item = parse_item(v, show)?;
                let lookup = item.show_key.clone().unwrap_or_else(|| item.item_key.clone());
                item.collections = collections.get(&lookup).cloned().unwrap_or_default();
                Some(item)
            })
            .collect())
    }

    pub fn hls_url(&self, item_key: &str, part_key: Option<&str>, _offset_ms: i64, session: &str) -> Url {
        let mut u = self.url(&format!("/Videos/{item_key}/master.m3u8"));
        u.query_pairs_mut()
            .append_pair("MediaSourceId", part_key.unwrap_or(item_key))
            .append_pair("PlaySessionId", session)
            .append_pair("DeviceId", &format!("rewindtv-{session}"))
            .append_pair("VideoCodec", "h264")
            .append_pair("AudioCodec", "aac")
            .append_pair("TranscodingMaxAudioChannels", "2")
            .append_pair("MaxStreamingBitrate", "40000000")
            .append_pair("SegmentContainer", "ts")
            .append_pair("MinSegments", "1")
            .append_pair("BreakOnNonKeyFrames", "true");
        // Jellyfin serves a full-length playlist and rejects StartTimeTicks on
        // segment requests, so the player seeks to the live offset instead.
        u
    }

    pub fn file_url(&self, item_key: &str) -> Url {
        let mut u = self.url(&format!("/Videos/{item_key}/stream"));
        u.query_pairs_mut()
            .append_pair("static", "true")
            .append_pair("MediaSourceId", item_key)
            .append_pair("api_key", &self.token);
        u
    }

    pub async fn stop(&self, session: &str) -> anyhow::Result<()> {
        self.http
            .delete(self.url("/Videos/ActiveEncodings"))
            .headers(self.auth_headers())
            .query(&[("DeviceId", format!("rewindtv-{session}")), ("PlaySessionId", session.to_string())])
            .send()
            .await?;
        Ok(())
    }

    pub fn image_url(&self, path: &str, width: u32) -> Url {
        let mut u = self.url(path);
        u.query_pairs_mut().append_pair("maxWidth", &width.to_string()).append_pair("quality", "85");
        u
    }
}

fn first_studio(v: &Value) -> Option<String> {
    arr(v, "Studios").first().and_then(|st| s(st, "Name"))
}

fn genres(v: &Value) -> Vec<String> {
    arr(v, "Genres").iter().filter_map(|g| g.as_str().map(str::to_string)).collect()
}

fn has_image(v: &Value, kind: &str) -> bool {
    match kind {
        "Backdrop" => !arr(v, "BackdropImageTags").is_empty(),
        _ => v.get("ImageTags").and_then(|t| t.get(kind)).is_some(),
    }
}

fn parse_item(v: &Value, show: Option<&Value>) -> Option<RemoteItem> {
    let id = s(v, "Id")?;
    let ticks = i(v, "RunTimeTicks").filter(|t| *t > 0)?;
    let year = i(v, "ProductionYear").or_else(|| s(v, "PremiereDate").and_then(|d| d.get(..4)?.parse().ok()));
    let mut item = RemoteItem {
        part_key: Some(id.clone()),
        title: s(v, "Name")?,
        year,
        duration_ms: ticks / TICKS_PER_MS,
        summary: s(v, "Overview"),
        content_rating: s(v, "OfficialRating"),
        studio: first_studio(v),
        genres: genres(v),
        thumb: has_image(v, "Primary").then(|| format!("/Items/{id}/Images/Primary")),
        art: has_image(v, "Backdrop").then(|| format!("/Items/{id}/Images/Backdrop/0")),
        item_key: id,
        ..Default::default()
    };
    match s(v, "Type").as_deref() {
        Some("Movie") => item.kind = "movie",
        Some("Episode") => {
            item.kind = "episode";
            let series_id = s(v, "SeriesId");
            item.show_title = s(v, "SeriesName");
            item.season = i(v, "ParentIndexNumber");
            item.episode = i(v, "IndexNumber");
            if let (Some(show), Some(sid)) = (show, series_id.as_ref()) {
                if item.genres.is_empty() {
                    item.genres = genres(show);
                }
                item.studio = item.studio.or_else(|| first_studio(show));
                item.content_rating = item.content_rating.or_else(|| s(show, "OfficialRating"));
                item.year = item.year.or_else(|| i(show, "ProductionYear"));
                if has_image(show, "Primary") {
                    item.thumb = Some(format!("/Items/{sid}/Images/Primary"));
                }
                if has_image(show, "Backdrop") {
                    item.art = Some(format!("/Items/{sid}/Images/Backdrop/0"));
                }
            }
            item.show_key = series_id;
        }
        _ => return None,
    }
    Some(item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episode_inherits_series_metadata() {
        let show = serde_json::json!({
            "Id": "s1", "Genres": ["Animation"], "OfficialRating": "TV-Y7",
            "Studios": [{"Name": "Fox Kids"}], "ImageTags": {"Primary": "x"}, "BackdropImageTags": ["y"]
        });
        let ep = serde_json::json!({
            "Id": "e1", "Name": "Pilot", "Type": "Episode", "SeriesId": "s1", "SeriesName": "X-Men",
            "ParentIndexNumber": 1, "IndexNumber": 2, "RunTimeTicks": 13_200_000_000i64,
            "ProductionYear": 1992, "Genres": []
        });
        let item = parse_item(&ep, Some(&show)).unwrap();
        assert_eq!(item.kind, "episode");
        assert_eq!(item.duration_ms, 1_320_000);
        assert_eq!(item.genres, vec!["Animation"]);
        assert_eq!(item.studio.as_deref(), Some("Fox Kids"));
        assert_eq!(item.thumb.as_deref(), Some("/Items/s1/Images/Primary"));
        assert_eq!(item.show_key.as_deref(), Some("s1"));
    }

    #[test]
    fn url_keeps_base_path() {
        let j = Jellyfin::new(reqwest::Client::new(), Url::parse("http://nas:8096/jf").unwrap(), "k".into());
        assert_eq!(j.url("/System/Info").as_str(), "http://nas:8096/jf/System/Info");
    }
}
