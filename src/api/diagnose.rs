//! "Run check" for a media server: exercises every path RewindTV uses against a
//! real item and reports exactly where something fails.

use std::process::Stdio;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::error::ApiResult;
use crate::media::Source;
use crate::state::AppState;
use crate::stream::hls;

#[derive(Serialize)]
pub struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
    ms: u128,
}

#[derive(Serialize)]
pub struct Report {
    source: String,
    kind: String,
    item: Option<String>,
    ok: bool,
    checks: Vec<Check>,
}

#[derive(sqlx::FromRow)]
struct Sample {
    item_key: String,
    part_key: Option<String>,
    title: String,
    show_title: Option<String>,
    duration_ms: i64,
    thumb: Option<String>,
    video_codec: Option<String>,
}

/// Codecs/sizes reported by ffprobe, e.g. "h264 1920x1080, aac 2ch".
#[derive(Default, Debug)]
pub struct Probe {
    pub video: Option<String>,
    pub audio: Option<String>,
    pub summary: String,
}

pub fn parse_probe(v: &Value) -> Probe {
    let mut p = Probe::default();
    let mut parts = Vec::new();
    for s in v.get("streams").and_then(Value::as_array).into_iter().flatten() {
        let codec = s.get("codec_name").and_then(Value::as_str).unwrap_or("?").to_string();
        match s.get("codec_type").and_then(Value::as_str) {
            Some("video") if p.video.is_none() => {
                let (w, h) = (s.get("width").and_then(Value::as_i64).unwrap_or(0), s.get("height").and_then(Value::as_i64).unwrap_or(0));
                parts.push(format!("{codec} {w}x{h}"));
                p.video = Some(codec);
            }
            Some("audio") if p.audio.is_none() => {
                let ch = s.get("channels").and_then(Value::as_i64).unwrap_or(0);
                parts.push(format!("{codec} {ch}ch"));
                p.audio = Some(codec);
            }
            _ => {}
        }
    }
    p.summary = if parts.is_empty() { "no audio/video streams".into() } else { parts.join(", ") };
    p
}

async fn ffprobe(st: &AppState, input: ProbeInput<'_>) -> Result<Probe, String> {
    let mut cmd = Command::new(&st.cfg.ffprobe);
    cmd.args(["-v", "error", "-show_entries", "stream=codec_type,codec_name,width,height,channels", "-of", "json"]);
    match &input {
        ProbeInput::Url(u) => cmd.args(["-rw_timeout", "15000000", "-i", u]),
        ProbeInput::Bytes(_) => cmd.args(["-i", "pipe:0"]),
    };
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("could not run {}: {e}", st.cfg.ffprobe))?;
    let mut stdin = child.stdin.take();
    if let (ProbeInput::Bytes(b), Some(mut sin)) = (&input, stdin.take()) {
        let data = b.to_vec();
        tokio::spawn(async move {
            let _ = sin.write_all(&data).await;
        });
    }
    drop(stdin);
    let out = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .map_err(|_| "ffprobe timed out".to_string())?
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(crate::media::redact(String::from_utf8_lossy(&out.stderr).trim()).chars().take(300).collect());
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    Ok(parse_probe(&v))
}

enum ProbeInput<'a> {
    Url(&'a str),
    Bytes(&'a [u8]),
}

/// Starts a session, downloads its first segment and probes it.
async fn try_session(st: &AppState, src: &Source, master: url::Url, sid: &str) -> Result<(Probe, f64), String> {
    src.prepare(&master).await;
    st.sessions.insert(sid.to_string(), src.clone(), master).await;
    let result = async {
        let total = hls::playlist_seconds(st, sid).await?;
        let seg = hls::first_segment(st, sid).await.map_err(|e| format!("first video segment: {e}"))?;
        let probe = ffprobe(st, ProbeInput::Bytes(&seg)).await?;
        Ok::<_, String>((probe, total))
    }
    .await;
    st.sessions.stop(sid).await;
    result
}

/// POST /api/sources/:id/diagnose
pub async fn diagnose(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Report>> {
    let (name, kind): (String, String) = sqlx::query_as("SELECT name, kind FROM sources WHERE id = ?")
        .bind(id)
        .fetch_optional(&st.db)
        .await?
        .ok_or(crate::error::ApiError::NotFound)?;
    let src = st.source(id).await?;
    let mut checks = Vec::new();
    let mut push = |name: &'static str, started: Instant, r: Result<String, String>| {
        let ok = r.is_ok();
        checks.push(Check { name, ok, detail: r.unwrap_or_else(|e| e), ms: started.elapsed().as_millis() });
        ok
    };

    let t = Instant::now();
    let connected = push("Connect to server", t, src.test().await.map(|n| format!("Connected to “{n}”")).map_err(|e| format!("{e:#}")));

    let t = Instant::now();
    let (items, libs): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM media_items WHERE source_id = ?1 AND missing = 0),
                (SELECT COUNT(*) FROM libraries WHERE source_id = ?1 AND enabled = 1)",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await?;
    push(
        "Library sync",
        t,
        if items > 0 { Ok(format!("{items} items in {libs} libraries")) } else { Err("No items synced yet — run Sync now".into()) },
    );

    let sample: Option<Sample> = sqlx::query_as(
        "SELECT item_key, part_key, title, show_title, duration_ms, thumb, video_codec FROM media_items
         WHERE source_id = ? AND missing = 0 AND duration_ms > 180000 ORDER BY RANDOM() LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&st.db)
    .await?;
    let item_label = sample.as_ref().map(|s| match &s.show_title {
        Some(show) => format!("{show} — {}", s.title),
        None => s.title.clone(),
    });

    if let (true, Some(s)) = (connected, &sample) {
        let t = Instant::now();
        let art = match &s.thumb {
            Some(path) => {
                let url = src.image_url(path, 300);
                match st.http.get(url).headers(src.auth_headers()).send().await {
                    Ok(r) if r.status().is_success() => Ok("Poster loaded".to_string()),
                    Ok(r) => Err(format!("Server answered {}", r.status())),
                    Err(e) => Err(e.to_string()),
                }
            }
            None => Err("This item has no poster".into()),
        };
        push("Artwork", t, art);

        let offset = (s.duration_ms / 3).min(600_000);
        let t = Instant::now();
        let sid = uuid::Uuid::new_v4().simple().to_string();
        let master = src.hls_url(&s.item_key, s.part_key.as_deref(), s.video_codec.as_deref(), offset, &sid);
        let r = try_session(&st, &src, master, &sid).await.and_then(|(p, total)| match (p.video.as_deref(), p.audio.as_deref()) {
            (Some("h264"), Some("aac")) => Ok(format!("{} — playlist {:.0}s", p.summary, total)),
            _ => Err(format!("Browsers need h264 + aac, got {}", p.summary)),
        });
        push("Browser playback", t, r);

        let t = Instant::now();
        let sid = uuid::Uuid::new_v4().simple().to_string();
        let master = src.tuner_url(&s.item_key, s.part_key.as_deref(), offset, &sid);
        let r = try_session(&st, &src, master, &sid).await.and_then(|(p, _)| match (p.video.as_deref(), p.audio.as_deref()) {
            (Some("h264"), Some("aac")) => Ok(format!("{} — ready in {:.1}s", p.summary, t.elapsed().as_secs_f64())),
            _ => Err(format!("Live TV needs h264 + aac, got {}", p.summary)),
        });
        push("Live TV transcode", t, r);

        let t = Instant::now();
        let r = match src.file_url(&s.item_key, s.part_key.as_deref()) {
            Some(u) => ffprobe(&st, ProbeInput::Url(u.as_str())).await.map(|p| format!("Original file: {}", p.summary)),
            None => Err("No file path for this item".into()),
        };
        push("Direct file access", t, r);
    }

    let t = Instant::now();
    let (eps, intro, credits): (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),
                SUM(EXISTS (SELECT 1 FROM markers k WHERE k.media_key = m.media_key AND k.kind = 'intro')),
                SUM(EXISTS (SELECT 1 FROM markers k WHERE k.media_key = m.media_key AND k.kind = 'credits'))
         FROM media_items m WHERE m.source_id = ? AND m.kind = 'episode' AND m.missing = 0",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await
    .map(|(a, b, c): (i64, Option<i64>, Option<i64>)| (a, b.unwrap_or(0), c.unwrap_or(0)))?;
    let server_markers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM media_items m WHERE m.source_id = ? AND EXISTS
            (SELECT 1 FROM markers k WHERE k.media_key = m.media_key AND k.source = 'server')",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await?;
    push(
        "Intro & credits markers",
        t,
        if eps == 0 {
            Ok("No TV episodes".into())
        } else if server_markers == 0 && kind == "plex" {
            Err(format!(
                "{intro}/{eps} episodes have saved intro markers, {credits}/{eps} credits (season rules apply on top) — none come from Plex. Turn on \"Generate intro video markers\" and \"Generate credits video markers\" in Plex (Plex Pass), or use auto-detect"
            ))
        } else {
            Ok(format!("{intro}/{eps} episodes have saved intro markers, {credits}/{eps} credits ({server_markers} from the server; season rules apply on top)"))
        },
    );

    let t = Instant::now();
    let muxers = Command::new(&st.cfg.ffmpeg).args(["-hide_banner", "-muxers"]).output().await;
    push(
        "Audio fingerprinting",
        t,
        match muxers {
            Ok(o) if String::from_utf8_lossy(&o.stdout).contains("chromaprint") => Ok("ffmpeg has chromaprint — auto-detect can run".into()),
            Ok(_) => Err("This ffmpeg lacks chromaprint, so auto-detect can't run (Plex and manual markers still work)".into()),
            Err(e) => Err(e.to_string()),
        },
    );

    let t = Instant::now();
    let ff = Command::new(&st.cfg.ffmpeg).arg("-version").output().await;
    push(
        "ffmpeg",
        t,
        match ff {
            Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or_default().chars().take(60).collect()),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).chars().take(200).collect()),
            Err(e) => Err(format!("{}: {e}", st.cfg.ffmpeg)),
        },
    );

    let ok = checks.iter().all(|c| c.ok);
    Ok(Json(Report { source: name, kind, item: item_label, ok, checks }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_summary() {
        let v = serde_json::json!({"streams": [
            {"codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080},
            {"codec_type": "audio", "codec_name": "aac", "channels": 2}
        ]});
        let p = parse_probe(&v);
        assert_eq!(p.video.as_deref(), Some("h264"));
        assert_eq!(p.summary, "h264 1920x1080, aac 2ch");
    }
}
