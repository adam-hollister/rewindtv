//! M3U + XMLTV export and continuous MPEG-TS channel streams for IPTV apps
//! (Plex/Jellyfin Live TV, TiviMate, VLC…).

use std::fmt::Write as _;
use std::process::Stdio;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::db::now_ms;
use crate::error::{ApiError, ApiResult};
use crate::guide::{self, ProgramInfo};
use crate::schedule::store;
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

async fn program_at(st: &AppState, channel_id: i64, t_ms: i64) -> anyhow::Result<Option<ProgramInfo>> {
    if let Some(p) = guide::at(&st.db, channel_id, t_ms).await? {
        return Ok(Some(p));
    }
    let _guard = st.guide_lock.lock().await;
    let items = store::load_items(&st.db).await?;
    store::extend_channel(&st.db, &items, channel_id, st.cfg.tz, now_ms()).await?;
    Ok(guide::at(&st.db, channel_id, t_ms).await?)
}

/// Upper bound on how far before the requested offset a keyframe seek can land.
const SEEK_PREROLL_S: f64 = 5.0;

/// Finds the highest PES timestamp in an MPEG-TS byte stream.
struct PtsScanner {
    carry: Vec<u8>,
    /// Seconds, unwrapped relative to `base`.
    max: Option<f64>,
    base: f64,
}

impl PtsScanner {
    const WRAP: f64 = (1u64 << 33) as f64 / 90_000.0;

    fn new(base: f64) -> Self {
        Self { carry: Vec::new(), max: None, base }
    }

    fn feed(&mut self, data: &[u8]) {
        self.carry.extend_from_slice(data);
        let mut i = 0;
        while i + 188 <= self.carry.len() {
            if self.carry[i] != 0x47 {
                i += 1; // resync
                continue;
            }
            if let Some(ts) = pes_pts(&self.carry[i..i + 188]) {
                let mut secs = ts as f64 / 90_000.0;
                while secs < self.base - Self::WRAP / 2.0 {
                    secs += Self::WRAP;
                }
                self.max = Some(self.max.map_or(secs, |m| m.max(secs)));
            }
            i += 188;
        }
        self.carry.drain(..i);
    }
}

/// PTS from a TS packet that starts an audio/video PES.
fn pes_pts(pkt: &[u8]) -> Option<u64> {
    if pkt[1] & 0x40 == 0 {
        return None; // not a payload unit start
    }
    let afc = (pkt[3] >> 4) & 3;
    let start = match afc {
        1 => 4,
        3 => 5 + pkt[4] as usize,
        _ => return None,
    };
    let p = pkt.get(start..)?;
    if p.len() < 14 || p[..3] != [0, 0, 1] || !(0xC0..=0xEF).contains(&p[3]) || p[7] & 0x80 == 0 {
        return None;
    }
    let t = &p[9..14];
    Some(((t[0] as u64 >> 1) & 7) << 30 | (t[1] as u64) << 22 | ((t[2] as u64) >> 1) << 15 | (t[3] as u64) << 7 | (t[4] as u64) >> 1)
}

/// ffmpeg arguments for one program (or filler) segment of a continuous stream.
fn ffmpeg_args(st: &AppState, input: Option<&str>, offset_s: f64, length_s: f64, ts_offset_s: f64) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin", "-re"].iter().map(|s| s.to_string()).collect();
    let transcode = st.cfg.iptv_video == "h264";
    match input {
        Some(url) => {
            // Non-accurate seek starts cleanly on the keyframe before the offset,
            // so copied video has no negative pre-roll overlapping the last program.
            a.extend(["-noaccurate_seek".into(), "-ss".into(), format!("{offset_s:.3}"), "-i".into(), url.into()]);
            a.extend(["-t", &format!("{length_s:.3}"), "-map", "0:v:0", "-map", "0:a:0?"].map(String::from));
            if transcode {
                a.extend(["-vf", "scale=-2:720,fps=30", "-c:v", "libx264", "-preset", "veryfast", "-b:v", "4M", "-g", "60"].map(String::from));
            } else {
                a.extend(["-c:v", "copy"].map(String::from));
            }
        }
        None => {
            // Retro color bars with silence while nothing is scheduled.
            a.extend(["-f", "lavfi", "-i", "smptehdbars=size=1280x720:rate=30", "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo"].map(String::from));
            a.extend(["-t".into(), format!("{length_s:.3}")]);
            a.extend(["-c:v", "libx264", "-preset", "ultrafast", "-tune", "stillimage", "-g", "60", "-pix_fmt", "yuv420p"].map(String::from));
        }
    }
    a.extend(["-c:a", "aac", "-ac", "2", "-ar", "48000", "-b:a", "192k"].map(String::from));
    // Shift this program to start right where the previous one ended.
    a.extend(["-muxdelay", "0", "-muxpreload", "0"].map(String::from));
    a.extend(["-output_ts_offset".into(), format!("{ts_offset_s:.3}")]);
    a.extend(["-mpegts_flags", "+resend_headers", "-f", "mpegts", "pipe:1"].map(String::from));
    a
}

/// GET /iptv/stream/:channel.ts — runs one ffmpeg per program back-to-back and
/// keeps timestamps continuous across program changes.
pub async fn stream(State(st): State<AppState>, Path(file): Path<String>) -> ApiResult<Response> {
    let channel_id: i64 = file.trim_end_matches(".ts").parse().map_err(|_| ApiError::NotFound)?;
    program_at(&st, channel_id, now_ms()).await?.ok_or(ApiError::NotFound)?;

    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(64);
    tokio::spawn(async move {
        // Output timeline position in seconds; each program continues from
        // where the last one actually ended, so timestamps never jump back.
        let mut ts_offset = 1.0f64;
        // Schedule position being played. Programs play once each, start to
        // finish; only tuning in (or rejoining after falling behind) seeks.
        let mut cursor = now_ms();
        let mut failed: Option<i64> = None;
        loop {
            if cursor < now_ms() - 5_000 {
                cursor = now_ms();
            }
            let prog = match program_at(&st, channel_id, cursor).await {
                Ok(Some(p)) => p,
                _ => break,
            };
            let offset_ms = (cursor - prog.start_ms).max(0);
            let length_ms = prog.end_ms - prog.start_ms - offset_ms;
            if length_ms < 500 {
                cursor = prog.end_ms;
                continue;
            }
            if offset_ms > 0 {
                // A seek starts on the keyframe before the offset; leave room
                // for that pre-roll so it can't overlap what was already sent.
                ts_offset += SEEK_PREROLL_S;
            }
            let (offset_s, length_s) = (offset_ms as f64 / 1000.0, length_ms as f64 / 1000.0);
            let input = if prog.is_filler() || failed == Some(prog.id) {
                None
            } else {
                match st.source(prog.source_id.unwrap_or_default()).await {
                    Ok(src) => src.file_url(prog.item_key.as_deref().unwrap_or_default(), prog.part_key.as_deref()),
                    Err(_) => None,
                }
            };
            let args = ffmpeg_args(&st, input.as_ref().map(|u| u.as_str()), offset_s, length_s, ts_offset);
            let Ok(mut child) = Command::new(&st.cfg.ffmpeg).args(&args).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn()
            else {
                tracing::error!("could not start {}", st.cfg.ffmpeg);
                break;
            };
            let mut stdout = child.stdout.take().expect("piped stdout");
            let stderr = child.stderr.take().expect("piped stderr");
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::warn!("ffmpeg: {line}");
                }
            });
            let mut scanner = PtsScanner::new(ts_offset);
            let mut buf = vec![0u8; 188 * 512];
            let mut wrote = false;
            loop {
                match stdout.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        wrote = true;
                        scanner.feed(&buf[..n]);
                        if tx.send(Ok(Bytes::copy_from_slice(&buf[..n]))).await.is_err() {
                            // Viewer went away.
                            let _ = child.kill().await;
                            return;
                        }
                    }
                }
            }
            let _ = child.wait().await;
            // Continue right after the last timestamp actually written; the slack
            // covers B-frame DTS sitting just before the next program's first PTS.
            tracing::debug!("channel {channel_id}: program {} ended, offset {ts_offset:.3}, max pts {:?}", prog.id, scanner.max);
            if let Some(max) = scanner.max {
                ts_offset = ts_offset.max(max + 0.15);
            }
            if !wrote && failed != Some(prog.id) {
                // Source failed (deleted file, server down): show bars for the
                // rest of this program instead.
                tracing::warn!("channel {channel_id}: no output for program {}", prog.id);
                failed = Some(prog.id);
                continue;
            }
            if !wrote {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            cursor = prog.end_ms;
        }
    });

    Ok((
        [(header::CONTENT_TYPE, "video/mp2t"), (header::CACHE_CONTROL, "no-cache")],
        Body::from_stream(ReceiverStream::new(rx)),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pes_packet(pts: u64) -> Vec<u8> {
        let mut p = vec![0x47, 0x41, 0x00, 0x10, 0, 0, 1, 0xE0, 0, 0, 0x80, 0x80, 5];
        p.push(0x21 | ((pts >> 29) & 0x0e) as u8);
        p.push((pts >> 22) as u8);
        p.push(0x01 | ((pts >> 14) & 0xfe) as u8);
        p.push((pts >> 7) as u8);
        p.push(0x01 | ((pts << 1) & 0xfe) as u8);
        p.resize(188, 0xff);
        p
    }

    #[test]
    fn scanner_tracks_max_pts_across_chunk_splits() {
        let mut bytes = pes_packet(90_000 * 12);
        bytes.extend(pes_packet(90_000 * 15));
        bytes.extend(pes_packet(90_000 * 14));
        let mut s = PtsScanner::new(10.0);
        for chunk in bytes.chunks(100) {
            s.feed(chunk);
        }
        assert_eq!(s.max, Some(15.0));
    }
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
