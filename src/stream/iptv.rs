//! M3U + XMLTV export and continuous MPEG-TS channel streams for IPTV apps
//! (Plex/Jellyfin Live TV, TiviMate, VLC…).

use std::fmt::Write as _;
use std::process::Stdio;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde::Deserialize;
use tokio::io::AsyncReadExt;
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

async fn current_program(st: &AppState, channel_id: i64) -> anyhow::Result<Option<ProgramInfo>> {
    let now = now_ms();
    if let Some(p) = guide::at(&st.db, channel_id, now).await? {
        return Ok(Some(p));
    }
    let _guard = st.guide_lock.lock().await;
    let items = store::load_items(&st.db).await?;
    store::extend_channel(&st.db, &items, channel_id, st.cfg.tz, now).await?;
    Ok(guide::at(&st.db, channel_id, now).await?)
}

/// ffmpeg arguments for one program (or filler) segment of a continuous stream.
fn ffmpeg_args(st: &AppState, input: Option<&str>, offset_s: f64, length_s: f64, ts_offset_s: f64) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin", "-re"].iter().map(|s| s.to_string()).collect();
    let transcode = st.cfg.iptv_video == "h264";
    match input {
        Some(url) => {
            a.extend(["-ss".into(), format!("{offset_s:.3}"), "-i".into(), url.into()]);
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
    a.extend(["-output_ts_offset".into(), format!("{ts_offset_s:.3}")]);
    a.extend(["-mpegts_flags", "+resend_headers", "-f", "mpegts", "pipe:1"].map(String::from));
    a
}

/// GET /iptv/stream/:channel.ts — runs one ffmpeg per program back-to-back and
/// keeps timestamps continuous across program changes.
pub async fn stream(State(st): State<AppState>, Path(file): Path<String>) -> ApiResult<Response> {
    let channel_id: i64 = file.trim_end_matches(".ts").parse().map_err(|_| ApiError::NotFound)?;
    current_program(&st, channel_id).await?.ok_or(ApiError::NotFound)?;

    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(64);
    tokio::spawn(async move {
        let started = Instant::now();
        let mut failed: Option<i64> = None;
        loop {
            let prog = match current_program(&st, channel_id).await {
                Ok(Some(p)) => p,
                _ => break,
            };
            let now = now_ms();
            let offset_s = (now - prog.start_ms).max(0) as f64 / 1000.0;
            let length_s = ((prog.end_ms - now).max(1000)) as f64 / 1000.0;
            let input = if prog.is_filler() || failed == Some(prog.id) {
                None
            } else {
                match st.source(prog.source_id.unwrap_or_default()).await {
                    Ok(src) => src.file_url(prog.item_key.as_deref().unwrap_or_default(), prog.part_key.as_deref()),
                    Err(_) => None,
                }
            };
            let ts_offset = started.elapsed().as_secs_f64();
            let args = ffmpeg_args(&st, input.as_ref().map(|u| u.as_str()), offset_s, length_s, ts_offset);
            let Ok(mut child) = Command::new(&st.cfg.ffmpeg).args(&args).stdout(Stdio::piped()).stderr(Stdio::inherit()).kill_on_drop(true).spawn()
            else {
                tracing::error!("could not start {}", st.cfg.ffmpeg);
                break;
            };
            let mut stdout = child.stdout.take().expect("piped stdout");
            let mut buf = vec![0u8; 188 * 512];
            let mut wrote = false;
            loop {
                match stdout.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        wrote = true;
                        if tx.send(Ok(Bytes::copy_from_slice(&buf[..n]))).await.is_err() {
                            // Viewer went away.
                            let _ = child.kill().await;
                            return;
                        }
                    }
                }
            }
            let _ = child.wait().await;
            if !wrote {
                // Source failed (deleted file, server down): show bars for the
                // rest of this program instead of retrying in a hot loop.
                tracing::warn!("channel {channel_id}: no output for program {}", prog.id);
                if failed == Some(prog.id) {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                failed = Some(prog.id);
            }
        }
    });

    Ok((
        [(header::CONTENT_TYPE, "video/mp2t"), (header::CACHE_CONTROL, "no-cache")],
        Body::from_stream(ReceiverStream::new(rx)),
    )
        .into_response())
}
