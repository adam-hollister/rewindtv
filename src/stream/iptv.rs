//! M3U + XMLTV export and HDHomeRun tuner emulation, so Plex/Jellyfin Live TV
//! and IPTV apps (TiviMate, VLC…) can use RewindTV channels. The streams
//! themselves live in `tuner.rs`.

use std::fmt::Write as _;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::db::now_ms;
use crate::error::{ApiError, ApiResult};
use crate::guide::{self, ProgramInfo};
use crate::state::AppState;

pub fn base_url(st: &AppState, headers: &HeaderMap) -> String {
    if let Some(u) = &st.cfg.public_url {
        return u.clone();
    }
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost");
    format!("http://{host}")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn m3u_attr(s: &str) -> String {
    s.replace('"', "'").replace(['\n', ','], " ")
}

/// GET /iptv/:user/playlist.m3u  (user 0 = shared channels only)
pub async fn playlist(State(st): State<AppState>, Path(user): Path<i64>, headers: HeaderMap) -> ApiResult<Response> {
    let base = base_url(&st, &headers);
    let channels = guide::lineup(&st.db, (user > 0).then_some(user), false).await?;
    let mut out = format!("#EXTM3U url-tvg=\"{base}/iptv/{user}/xmltv.xml\" x-tvg-url=\"{base}/iptv/{user}/xmltv.xml\"\n");
    for c in channels {
        let _ = writeln!(
            out,
            "#EXTINF:-1 tvg-id=\"rewindtv.{id}\" tvg-chno=\"{num}\" channel-number=\"{num}\" tvg-name=\"{name}\" tvg-logo=\"{base}/api/channels/{id}/logo.svg\" group-title=\"RewindTV\",{name}\n{base}/iptv/stream/{id}.ts",
            id = c.id,
            num = c.number,
            name = m3u_attr(&c.name),
        );
    }
    Ok(([(header::CONTENT_TYPE, "audio/x-mpegurl; charset=utf-8")], out).into_response())
}

#[derive(Deserialize)]
pub struct XmltvQuery {
    hours: Option<i64>,
}

fn xmltv_time(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().format("%Y%m%d%H%M%S +0000").to_string()
}

/// GET /iptv/:user/xmltv.xml
pub async fn xmltv(
    State(st): State<AppState>,
    Path(user): Path<i64>,
    Query(q): Query<XmltvQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let base = base_url(&st, &headers);
    let channels = guide::lineup(&st.db, (user > 0).then_some(user), false).await?;
    let now = now_ms();
    let hours = q.hours.unwrap_or(72).clamp(1, 168);
    let ids: Vec<i64> = channels.iter().map(|c| c.id).collect();
    let programs = guide::range(&st.db, &ids, now - 3_600_000, now + hours * 3_600_000).await?;
    let genres = item_genres(&st, &programs).await?;

    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE tv SYSTEM \"xmltv.dtd\">\n<tv generator-info-name=\"RewindTV\">\n");
    for c in &channels {
        let _ = writeln!(
            out,
            "  <channel id=\"rewindtv.{id}\">\n    <display-name>{name}</display-name>\n    <display-name>{num}</display-name>\n    <icon src=\"{base}/api/channels/{id}/logo.svg\"/>\n  </channel>",
            id = c.id,
            num = c.number,
            name = xml_escape(&c.name),
        );
    }
    for p in &programs {
        let _ = writeln!(out, "  <programme start=\"{}\" stop=\"{}\" channel=\"rewindtv.{}\">", xmltv_time(p.start_ms), xmltv_time(p.end_ms), p.channel_id);
        let _ = writeln!(out, "    <title>{}</title>", xml_escape(&p.headline()));
        if !p.is_filler() {
            if p.kind.as_deref() == Some("episode") {
                if let Some(t) = &p.title {
                    let _ = writeln!(out, "    <sub-title>{}</sub-title>", xml_escape(t));
                }
            }
            if let Some(d) = &p.summary {
                let _ = writeln!(out, "    <desc>{}</desc>", xml_escape(d));
            }
            if let Some(y) = p.year {
                let _ = writeln!(out, "    <date>{y}</date>");
            }
            for g in genres.get(&p.item_id.unwrap_or_default()).into_iter().flatten() {
                let _ = writeln!(out, "    <category>{}</category>", xml_escape(g));
            }
            if p.kind.as_deref() == Some("movie") {
                let _ = writeln!(out, "    <category>Movie</category>");
            }
            if let (Some(src), Some(thumb)) = (p.source_id, &p.thumb) {
                let enc: String = url::form_urlencoded::byte_serialize(thumb.as_bytes()).collect();
                let _ = writeln!(out, "    <icon src=\"{base}/api/img/{src}?w=400&amp;p={enc}\"/>");
            }
            if let (Some(s), Some(e)) = (p.season, p.episode) {
                let _ = writeln!(out, "    <episode-num system=\"xmltv_ns\">{}.{}.</episode-num>", (s - 1).max(0), (e - 1).max(0));
                let _ = writeln!(out, "    <episode-num system=\"onscreen\">S{s:02}E{e:02}</episode-num>");
            }
            if let Some(r) = &p.content_rating {
                let _ = writeln!(out, "    <rating><value>{}</value></rating>", xml_escape(r));
            }
        }
        out.push_str("  </programme>\n");
    }
    out.push_str("</tv>\n");
    Ok(([(header::CONTENT_TYPE, "application/xml; charset=utf-8")], out).into_response())
}

async fn item_genres(st: &AppState, programs: &[ProgramInfo]) -> ApiResult<std::collections::HashMap<i64, Vec<String>>> {
    let ids: Vec<String> = programs.iter().filter_map(|p| p.item_id).map(|i| i.to_string()).collect();
    let mut map: std::collections::HashMap<i64, Vec<String>> = Default::default();
    if ids.is_empty() {
        return Ok(map);
    }
    let rows: Vec<(i64, String)> = sqlx::query_as(&format!("SELECT item_id, genre FROM item_genres WHERE item_id IN ({})", ids.join(",")))
        .fetch_all(&st.db)
        .await?;
    for (id, g) in rows {
        map.entry(id).or_default().push(g);
    }
    Ok(map)
}

// --- HDHomeRun emulation (so Plex Live TV can add RewindTV as a tuner) --------

fn hdhr_base(st: &AppState, headers: &HeaderMap, user: Option<i64>) -> String {
    let base = base_url(st, headers);
    match user {
        Some(u) => format!("{base}/hdhr/{u}"),
        None => base,
    }
}

fn hdhr_user(st: &AppState, user: Option<i64>) -> i64 {
    user.unwrap_or(st.cfg.hdhr_profile)
}

async fn discover_json(st: &AppState, headers: &HeaderMap, user: Option<i64>) -> Response {
    let base = hdhr_base(st, headers, user);
    let uid = hdhr_user(st, user);
    axum::Json(serde_json::json!({
        "FriendlyName": if uid > 0 { format!("RewindTV (profile {uid})") } else { "RewindTV".to_string() },
        "Manufacturer": "Silicondust",
        "ModelNumber": "HDTC-2US",
        "FirmwareName": "hdhomeruntc_atsc",
        "FirmwareVersion": "20200101",
        "DeviceID": format!("52574{uid:03}"),
        "DeviceAuth": "rewindtv",
        "TunerCount": 6,
        "BaseURL": base,
        "LineupURL": format!("{base}/lineup.json"),
    }))
    .into_response()
}

async fn lineup_json(st: &AppState, headers: &HeaderMap, user: Option<i64>) -> ApiResult<Response> {
    let base = base_url(st, headers);
    let uid = hdhr_user(st, user);
    let channels = guide::lineup(&st.db, (uid > 0).then_some(uid), false).await?;
    let list: Vec<_> = channels
        .iter()
        .map(|c| serde_json::json!({ "GuideNumber": c.number.to_string(), "GuideName": c.name, "URL": format!("{base}/iptv/stream/{}.ts", c.id) }))
        .collect();
    Ok(axum::Json(list).into_response())
}

fn lineup_status() -> Response {
    axum::Json(serde_json::json!({ "ScanInProgress": 0, "ScanPossible": 1, "Source": "Cable", "SourceList": ["Cable"] })).into_response()
}

fn device_xml(st: &AppState, headers: &HeaderMap, user: Option<i64>) -> Response {
    let base = hdhr_base(st, headers, user);
    let uid = hdhr_user(st, user);
    let xml = format!(
        r#"<root xmlns="urn:schemas-upnp-org:device-1-0"><specVersion><major>1</major><minor>0</minor></specVersion><URLBase>{base}</URLBase><device><deviceType>urn:schemas-upnp-org:device:MediaServer:1</deviceType><friendlyName>RewindTV</friendlyName><manufacturer>Silicondust</manufacturer><modelName>HDTC-2US</modelName><modelNumber>HDTC-2US</modelNumber><serialNumber/><UDN>uuid:52574{uid:03}-rewindtv</UDN></device></root>"#
    );
    ([(header::CONTENT_TYPE, "application/xml")], xml).into_response()
}

pub async fn hdhr_root(State(st): State<AppState>, uri: axum::http::Uri, headers: HeaderMap) -> ApiResult<Response> {
    hdhr(&st, &headers, None, uri.path().trim_start_matches('/')).await
}

pub async fn hdhr_profile(State(st): State<AppState>, Path((user, file)): Path<(i64, String)>, headers: HeaderMap) -> ApiResult<Response> {
    hdhr(&st, &headers, Some(user), &file).await
}

async fn hdhr(st: &AppState, headers: &HeaderMap, user: Option<i64>, file: &str) -> ApiResult<Response> {
    Ok(match file {
        "discover.json" => discover_json(st, headers, user).await,
        "lineup.json" => lineup_json(st, headers, user).await?,
        "lineup_status.json" => lineup_status(),
        "device.xml" => device_xml(st, headers, user),
        "lineup.post" => axum::Json(serde_json::json!({})).into_response(),
        _ => return Err(ApiError::NotFound),
    })
}
