use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use super::channels::ProgramOut;
use crate::db::now_ms;
use crate::error::{ApiError, ApiResult};
use crate::guide;
use crate::schedule::store;
use crate::state::AppState;

#[derive(Serialize)]
pub struct Status {
    version: &'static str,
    timezone: String,
    sources: i64,
    items: i64,
    channels: i64,
    users: i64,
    sessions: usize,
}

pub async fn status(State(st): State<AppState>) -> ApiResult<Json<Status>> {
    let count = |sql: &'static str| sqlx::query_scalar::<_, i64>(sql).fetch_one(&st.db);
    Ok(Json(Status {
        version: env!("CARGO_PKG_VERSION"),
        timezone: st.cfg.tz.name().to_string(),
        sources: count("SELECT COUNT(*) FROM sources").await?,
        items: count("SELECT COUNT(*) FROM media_items WHERE missing = 0").await?,
        channels: count("SELECT COUNT(*) FROM channels").await?,
        users: count("SELECT COUNT(*) FROM users").await?,
        sessions: st.sessions.len().await,
    }))
}

#[derive(Deserialize, Default)]
pub struct TuneInput {
    previous: Option<String>,
}

#[derive(Serialize)]
pub struct Tune {
    /// "stream" or "filler"
    kind: &'static str,
    session: Option<String>,
    url: Option<String>,
    /// Position in the media file (seconds) that "live" is right now —
    /// skipped intros/credits already accounted for.
    offset: f64,
    /// Parts of the file that air, in seconds; the player jumps the gaps.
    kept: Vec<(f64, f64)>,
    /// Full program length in seconds, so the player can tell whether the
    /// server honored the offset or handed back a full-length playlist.
    duration: f64,
    server_time: i64,
    program: ProgramOut,
    next: Option<ProgramOut>,
}

/// Start watching a channel live.
pub async fn tune(State(st): State<AppState>, Path(id): Path<i64>, body: Option<Json<TuneInput>>) -> ApiResult<Json<Tune>> {
    if let Some(prev) = body.and_then(|b| b.0.previous) {
        let sessions = st.sessions.clone();
        tokio::spawn(async move { sessions.stop(&prev).await });
    }
    let now = now_ms();
    let program = match guide::at(&st.db, id, now).await? {
        Some(p) => p,
        None => {
            {
                let _guard = st.guide_lock.lock().await;
                let items = st.items().await?;
                store::extend_channel(&st.db, &items, id, st.cfg.tz, now).await?;
            }
            guide::at(&st.db, id, now).await?.ok_or(ApiError::NotFound)?
        }
    };
    let next = guide::upcoming(&st.db, id, program.end_ms, 1).await?.into_iter().next();
    let offset = (now - program.start_ms) as f64 / 1000.0;
    let duration = (program.end_ms - program.start_ms) as f64 / 1000.0;
    let mut out = Tune {
        kind: "filler",
        session: None,
        url: None,
        offset,
        kept: vec![(0.0, duration)],
        duration,
        server_time: now,
        program: (&program).into(),
        next: next.as_ref().map(Into::into),
    };
    if program.is_filler() {
        return Ok(Json(out));
    }

    let src = st.source(program.source_id.unwrap_or_default()).await?;
    let sid = uuid::Uuid::new_v4().simple().to_string();
    let cuts = program.cut_ranges();
    let (offset_ms, _) = crate::schedule::cuts::media_pos(program.media_len(), &cuts, now - program.start_ms);
    out.offset = offset_ms as f64 / 1000.0;
    out.kept = crate::schedule::cuts::kept_ranges(program.media_len(), &cuts)
        .into_iter()
        .map(|(s, e)| (s as f64 / 1000.0, e as f64 / 1000.0))
        .collect();
    let master = src.hls_url(
        program.item_key.as_deref().unwrap_or_default(),
        program.part_key.as_deref(),
        program.video_codec.as_deref(),
        offset_ms,
        &sid,
    );
    src.prepare(&master).await;
    st.sessions.insert(sid.clone(), src, master).await;
    out.kind = "stream";
    out.url = Some(format!("/stream/{sid}/master.m3u8"));
    out.session = Some(sid);
    // Duration of the media itself (programs never exceed it, but be precise).
    out.duration = program.duration_ms.unwrap_or(program.end_ms - program.start_ms) as f64 / 1000.0;
    Ok(Json(out))
}

pub async fn stop_session(State(st): State<AppState>, Path(sid): Path<String>) -> Json<()> {
    st.sessions.stop(&sid).await;
    Json(())
}

#[derive(Deserialize)]
pub struct ImgQuery {
    p: String,
    w: Option<u32>,
}

/// Artwork proxy so media-server tokens stay server-side.
pub async fn image(State(st): State<AppState>, Path(source): Path<i64>, Query(q): Query<ImgQuery>) -> ApiResult<Response> {
    let src = st.source(source).await?;
    // Only paths on the media server itself (requests carry its credentials).
    let original = src.local_url(&q.p).ok_or_else(|| ApiError::BadRequest("bad image path".into()))?;
    let resized = src.image_url(&q.p, q.w.unwrap_or(400).clamp(64, 1920));
    let mut res = st.http.get(resized).headers(src.auth_headers()).timeout(std::time::Duration::from_secs(30)).send().await.map_err(|e| ApiError::BadGateway(e.to_string()))?;
    if !res.status().is_success() {
        // Fall back to the original image if the server's resizer fails.
        res = st.http.get(original).headers(src.auth_headers()).timeout(std::time::Duration::from_secs(30)).send().await.map_err(|e| ApiError::BadGateway(e.to_string()))?;
        if !res.status().is_success() {
            return Ok(StatusCode::NOT_FOUND.into_response());
        }
    }
    let ctype = res.headers().get(header::CONTENT_TYPE).cloned();
    let mut out = Response::builder().header(header::CACHE_CONTROL, "public, max-age=86400");
    if let Some(c) = ctype {
        out = out.header(header::CONTENT_TYPE, c);
    }
    Ok(out.body(Body::from_stream(res.bytes_stream())).map_err(anyhow::Error::from)?)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A generated channel "bug" used as the logo when none is set.
pub async fn logo(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Response> {
    let (number, name, color): (i64, String, String) = sqlx::query_as("SELECT number, name, color FROM channels WHERE id = ?")
        .bind(id)
        .fetch_optional(&st.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    let short: String = name.chars().take(14).collect();
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 300" width="400" height="300">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{color}"/><stop offset="1" stop-color="{color}" stop-opacity="0.55"/></linearGradient></defs>
<rect width="400" height="300" rx="36" fill="#111318"/>
<rect x="14" y="14" width="372" height="272" rx="26" fill="url(#g)"/>
<text x="200" y="170" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-weight="800" font-size="130" fill="#fff">{number}</text>
<text x="200" y="240" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-weight="700" font-size="36" letter-spacing="2" fill="#fff" fill-opacity="0.92">{name}</text>
</svg>"##,
        color = esc(&color),
        name = esc(&short.to_uppercase()),
    );
    Ok(([(header::CONTENT_TYPE, "image/svg+xml"), (header::CACHE_CONTROL, "no-cache")], svg).into_response())
}
