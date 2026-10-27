//! Intro/credits marker settings, scanning, coverage and manual editing.

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::now_ms;
use crate::error::{ApiError, ApiResult};
use crate::markers::{self, detect, Rule};
use crate::schedule::cuts::{self, Kind, Marker, Source, Sources};
use crate::schedule::store;
use crate::state::AppState;

#[derive(Serialize, Default)]
pub struct Coverage {
    episodes: usize,
    intro: usize,
    credits: usize,
    /// How many resolved markers come from each source.
    by_source: BTreeMap<&'static str, usize>,
}

fn tally(markers: &[Vec<Marker>]) -> Coverage {
    let mut c = Coverage { episodes: markers.len(), ..Default::default() };
    for list in markers {
        for m in list {
            match m.kind {
                Kind::Intro => c.intro += 1,
                Kind::Credits => c.credits += 1,
            }
            *c.by_source.entry(m.source.as_str()).or_default() += 1;
        }
    }
    c
}

#[derive(Serialize)]
pub struct MarkerSettings {
    sources: Sources,
    detect: detect::Status,
    coverage: Coverage,
    markers_dir: String,
    local_media: bool,
}

/// Re-apply markers to channels that skip, keeping what's on air now.
async fn refresh(st: &AppState) -> ApiResult<()> {
    st.invalidate_items().await;
    let _guard = st.guide_lock.lock().await;
    let items = st.items().await?;
    store::refresh_skipping(&st.db, &items, st.cfg.tz, now_ms()).await?;
    Ok(())
}

pub async fn settings(State(st): State<AppState>) -> ApiResult<Json<MarkerSettings>> {
    let items = st.items().await?;
    let eps: Vec<Vec<Marker>> = items.iter().filter(|i| !i.is_movie).map(|i| i.markers.clone()).collect();
    Ok(Json(MarkerSettings {
        sources: markers::sources(&st.db).await,
        detect: st.detect_status.read().await.clone(),
        coverage: tally(&eps),
        markers_dir: st.cfg.markers_dir.display().to_string(),
        local_media: !st.cfg.path_maps.is_empty(),
    }))
}

pub async fn set_settings(State(st): State<AppState>, Json(s): Json<Sources>) -> ApiResult<Json<()>> {
    markers::set_sources(&st.db, s).await?;
    if let Err(e) = markers::files::save_sources(&st.cfg.markers_dir, s).await {
        tracing::warn!("saving marker settings file: {e:#}");
    }
    refresh(&st).await?;
    if s.detect {
        tokio::spawn(detect::run(st.clone(), None));
    }
    Ok(Json(()))
}

#[derive(Deserialize, Default)]
pub struct ScanInput {
    channel_id: Option<i64>,
}

pub async fn scan(State(st): State<AppState>, body: Option<Json<ScanInput>>) -> Json<detect::Status> {
    let channel = body.and_then(|b| b.0.channel_id);
    tokio::spawn(detect::run(st.clone(), channel));
    let mut s = st.detect_status.read().await.clone();
    s.running = true;
    Json(s)
}

pub async fn scan_status(State(st): State<AppState>) -> Json<detect::Status> {
    Json(st.detect_status.read().await.clone())
}

/// GET /api/channels/:id/markers — how much of a channel's lineup has markers.
pub async fn channel_coverage(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Coverage>> {
    let cfg = store::load_config(&st.db, id).await?.ok_or(ApiError::NotFound)?;
    let items = st.items().await?;
    let lists: Vec<Vec<Marker>> = items
        .iter()
        .filter(|it| !it.is_movie && (cfg.rule.matches(it) || cfg.slots.iter().any(|s| s.rule.matches(it))))
        .map(|it| it.markers.clone())
        .collect();
    Ok(Json(tally(&lists)))
}

#[derive(Serialize)]
pub struct ShowRow {
    key: String,
    title: String,
    episodes: usize,
    intro: usize,
    credits: usize,
}

/// GET /api/markers/shows — every show with its marker coverage.
pub async fn shows(State(st): State<AppState>) -> ApiResult<Json<Vec<ShowRow>>> {
    let items = st.items().await?;
    let mut rows: HashMap<String, ShowRow> = HashMap::new();
    for it in items.iter().filter(|i| !i.is_movie) {
        let Some(title) = &it.show_title else { continue };
        let key = markers::show_key(title);
        let r = rows.entry(key.clone()).or_insert_with(|| ShowRow { key, title: title.clone(), episodes: 0, intro: 0, credits: 0 });
        r.episodes += 1;
        r.intro += it.markers.iter().any(|m| m.kind == Kind::Intro) as usize;
        r.credits += it.markers.iter().any(|m| m.kind == Kind::Credits) as usize;
    }
    let mut out: Vec<ShowRow> = rows.into_values().collect();
    out.sort_by_key(|r| r.title.to_lowercase());
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct ShowQuery {
    key: String,
}

#[derive(Serialize)]
pub struct MarkerOut {
    kind: &'static str,
    source: &'static str,
    start_ms: i64,
    end_ms: i64,
}

#[derive(Serialize)]
pub struct EpisodeOut {
    media_key: String,
    item_id: i64,
    season: i64,
    episode: i64,
    title: String,
    duration_ms: i64,
    /// Every marker we know about (all sources), and the ones in effect.
    candidates: Vec<MarkerOut>,
    resolved: Vec<MarkerOut>,
}

#[derive(Serialize)]
pub struct ShowDetail {
    key: String,
    title: String,
    rules: Vec<Rule>,
    episodes: Vec<EpisodeOut>,
}

fn out(m: &Marker) -> MarkerOut {
    MarkerOut { kind: m.kind.as_str(), source: m.source.as_str(), start_ms: m.start_ms, end_ms: m.end_ms }
}

/// GET /api/markers/show?key=tv/… — a show's episodes and their markers.
pub async fn show(State(st): State<AppState>, Query(q): Query<ShowQuery>) -> ApiResult<Json<ShowDetail>> {
    let rows: Vec<(i64, String, i64, i64, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, media_key, season, episode, title, show_title, duration_ms, provider_ids FROM media_items
         WHERE media_key >= ?1 || '/' AND media_key < ?1 || '0' AND missing = 0 AND season IS NOT NULL AND episode IS NOT NULL
         ORDER BY season, episode, id",
    )
    .bind(&q.key)
    .fetch_all(&st.db)
    .await?;
    if rows.is_empty() {
        return Err(ApiError::NotFound);
    }
    let title = rows[0].5.clone();
    // Several servers may carry the same episode; show it once.
    let mut seen: HashSet<String> = HashSet::new();
    let rows: Vec<_> = rows.into_iter().filter(|r| seen.insert(r.1.clone())).collect();
    let refs: Vec<markers::ItemRef> = rows
        .iter()
        .map(|r| markers::ItemRef {
            id: r.0,
            media_key: Some(r.1.clone()),
            provider_ids: r.7.as_deref().and_then(|p| serde_json::from_str(p).ok()).unwrap_or_default(),
            season: Some(r.2),
            duration_ms: r.6,
        })
        .collect();
    let cands = markers::candidates(&st.db, &refs).await?;
    let sources = markers::sources(&st.db).await;
    let episodes = rows
        .into_iter()
        .map(|(id, media_key, season, episode, title, _, duration_ms, _)| {
            let c = cands.get(&id).cloned().unwrap_or_default();
            let resolved = cuts::resolve(&c, duration_ms, sources);
            EpisodeOut {
                media_key,
                item_id: id,
                season,
                episode,
                title,
                duration_ms,
                candidates: c.iter().map(out).collect(),
                resolved: resolved.iter().map(out).collect(),
            }
        })
        .collect();
    Ok(Json(ShowDetail { rules: markers::rules(&st.db, Some(&q.key)).await?, key: q.key, title, episodes }))
}

#[derive(Deserialize)]
pub struct EpisodeEdit {
    media_key: String,
    kind: String,
    /// None clears the manual marker.
    range: Option<(i64, i64)>,
}

/// PUT /api/markers/episode — set or clear a manual marker for one episode.
pub async fn edit_episode(State(st): State<AppState>, Json(e): Json<EpisodeEdit>) -> ApiResult<Json<()>> {
    let kind = Kind::parse(&e.kind).ok_or_else(|| ApiError::BadRequest("kind must be intro or credits".into()))?;
    let duration: i64 = sqlx::query_scalar("SELECT duration_ms FROM media_items WHERE media_key = ? LIMIT 1")
        .bind(&e.media_key)
        .fetch_optional(&st.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    if let Some((s, end)) = e.range {
        if s < 0 || end <= s || end > duration + 1_000 {
            return Err(ApiError::BadRequest("The end must come after the start, within the episode".into()));
        }
    }
    markers::set_one(&st.db, &e.media_key, kind, Source::Manual, e.range, duration).await?;
    crate::sync::export_touched(&st, [e.media_key].into()).await;
    refresh(&st).await?;
    Ok(Json(()))
}

#[derive(Deserialize)]
pub struct RuleDelete {
    show_key: String,
    season: Option<i64>,
    kind: String,
}

/// PUT /api/markers/rule — a season (or whole-show) rule.
pub async fn save_rule(State(st): State<AppState>, Json(mut r): Json<Rule>) -> ApiResult<Json<()>> {
    if Kind::parse(&r.kind).is_none() || r.start_ms < 0 || r.end_ms < 0 || (!r.from_end && r.end_ms <= r.start_ms) || (r.from_end && r.start_ms <= r.end_ms) {
        return Err(ApiError::BadRequest("Check the times: the marker must have a positive length".into()));
    }
    r.updated_at = now_ms();
    markers::save_rule(&st.db, &r).await?;
    crate::sync::export_touched(&st, [format!("{}/rule", r.show_key)].into()).await;
    refresh(&st).await?;
    Ok(Json(()))
}

pub async fn delete_rule(State(st): State<AppState>, Json(r): Json<RuleDelete>) -> ApiResult<Json<()>> {
    markers::delete_rule(&st.db, &r.show_key, r.season, &r.kind).await?;
    crate::sync::export_touched(&st, [format!("{}/rule", r.show_key)].into()).await;
    refresh(&st).await?;
    Ok(Json(()))
}

/// GET /api/markers/episode/:item_id/kept — not used by the UI directly; handy
/// for checking what an episode airs as on a skipping channel.
pub async fn preview_cuts(State(st): State<AppState>, Path(item_id): Path<i64>) -> ApiResult<Json<Vec<(i64, i64)>>> {
    let items = st.items().await?;
    let it = items.iter().find(|i| i.id == item_id).ok_or(ApiError::NotFound)?;
    let c = cuts::cuts_for(&it.markers, it.duration_ms, cuts::Skip { intros: true, credits: true });
    Ok(Json(cuts::kept_ranges(it.duration_ms, &c)))
}

#[derive(Deserialize)]
pub struct PlayQuery {
    at: Option<i64>,
}

#[derive(Serialize)]
pub struct Play {
    url: String,
    session: String,
    /// File position (seconds) the stream was asked to start at.
    offset: f64,
    duration: f64,
}

/// POST /api/items/:id/play?at=ms — on-demand playback of one episode, for
/// checking and setting markers.
pub async fn play_item(State(st): State<AppState>, Path(id): Path<i64>, Query(q): Query<PlayQuery>) -> ApiResult<Json<Play>> {
    let (source_id, item_key, part_key, video_codec, duration): (i64, String, Option<String>, Option<String>, i64) =
        sqlx::query_as("SELECT source_id, item_key, part_key, video_codec, duration_ms FROM media_items WHERE id = ?")
            .bind(id)
            .fetch_optional(&st.db)
            .await?
            .ok_or(ApiError::NotFound)?;
    let src = st.source(source_id).await?;
    let sid = uuid::Uuid::new_v4().simple().to_string();
    let at = q.at.unwrap_or(0).clamp(0, duration);
    let master = src.hls_url(&item_key, part_key.as_deref(), video_codec.as_deref(), at, &sid);
    src.prepare(&master).await;
    st.sessions.insert(sid.clone(), src, master).await;
    Ok(Json(Play { url: format!("/stream/{sid}/master.m3u8"), session: sid, offset: at as f64 / 1000.0, duration: duration as f64 / 1000.0 }))
}
