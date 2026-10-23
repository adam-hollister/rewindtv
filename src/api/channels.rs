use axum::extract::{Path, Query, State};
use axum::Json;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};

use crate::db::now_ms;
use crate::error::{ApiError, ApiResult};
use crate::guide::{self, ChannelRow, ProgramInfo};
use crate::schedule::ordering::{ChannelState, Ordering};
use crate::schedule::rule::Rule;
use crate::schedule::slots::Slot;
use crate::schedule::{generate, store, ChannelConfig};
use crate::state::AppState;

const PALETTE: &[&str] = &["#f59e0b", "#ef4444", "#10b981", "#3b82f6", "#8b5cf6", "#ec4899", "#14b8a6", "#f97316", "#84cc16", "#06b6d4"];

#[derive(Debug, Serialize)]
pub struct ProgramOut {
    id: i64,
    start: i64,
    end: i64,
    title: String,
    subtitle: Option<String>,
    episode_title: Option<String>,
    kind: Option<String>,
    filler: bool,
    thumb: Option<String>,
    art: Option<String>,
    summary: Option<String>,
    rating: Option<String>,
    year: Option<i64>,
    season: Option<i64>,
    episode: Option<i64>,
    block: Option<String>,
}

pub fn img(source: Option<i64>, path: &Option<String>, w: u32) -> Option<String> {
    let enc: String = url::form_urlencoded::byte_serialize(path.as_ref()?.as_bytes()).collect();
    Some(format!("/api/img/{}?w={w}&p={enc}", source?))
}

impl From<&ProgramInfo> for ProgramOut {
    fn from(p: &ProgramInfo) -> Self {
        let filler = p.is_filler();
        ProgramOut {
            id: p.id,
            start: p.start_ms,
            end: p.end_ms,
            title: p.headline(),
            subtitle: if filler { None } else { p.subtitle() },
            episode_title: (p.kind.as_deref() == Some("episode")).then(|| p.title.clone()).flatten(),
            kind: p.kind.clone(),
            filler,
            thumb: img(p.source_id, &p.thumb, 300),
            art: img(p.source_id, &p.art, 1280),
            summary: p.summary.clone(),
            rating: p.content_rating.clone(),
            year: p.year,
            season: p.season,
            episode: p.episode,
            block: p.slot_title.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotIo {
    #[serde(default)]
    id: Option<i64>,
    title: String,
    days: u8,
    start_min: u32,
    end_min: u32,
    #[serde(default)]
    rule: Rule,
    ordering: Ordering,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelIo {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    number: Option<i64>,
    name: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    logo: Option<String>,
    #[serde(default)]
    owner_user_id: Option<i64>,
    #[serde(default)]
    rule: Rule,
    ordering: Ordering,
    #[serde(default = "default_block")]
    block_size: i64,
    #[serde(default = "default_avoid")]
    avoid_repeat_hours: i64,
    #[serde(default)]
    align_minutes: i64,
    #[serde(default)]
    slots: Vec<SlotIo>,
}

fn default_block() -> i64 {
    3
}
fn default_avoid() -> i64 {
    24
}

#[derive(Deserialize)]
pub struct UserQuery {
    user: Option<i64>,
}

#[derive(Serialize)]
pub struct ChannelListItem {
    #[serde(flatten)]
    channel: ChannelRow,
    now: Option<ProgramOut>,
}

pub async fn list(State(st): State<AppState>, Query(q): Query<UserQuery>) -> ApiResult<Json<Vec<ChannelListItem>>> {
    let now = now_ms();
    let rows = guide::lineup(&st.db, q.user, true).await?;
    let mut out = Vec::with_capacity(rows.len());
    for c in rows {
        let current = guide::at(&st.db, c.id, now).await?;
        out.push(ChannelListItem { now: current.as_ref().map(ProgramOut::from), channel: c });
    }
    Ok(Json(out))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<ChannelIo>> {
    let (number, name, color, logo, owner, rule, ordering, block, avoid, align): (i64, String, String, Option<String>, Option<i64>, String, String, i64, i64, i64) =
        sqlx::query_as(
            "SELECT number, name, color, logo, owner_user_id, rule, ordering, block_size, avoid_repeat_hours, align_minutes
             FROM channels WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&st.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    let cfg = store::load_config(&st.db, id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(ChannelIo {
        id: Some(id),
        number: Some(number),
        name,
        color: Some(color),
        logo,
        owner_user_id: owner,
        rule: serde_json::from_str(&rule).unwrap_or_default(),
        ordering: Ordering::parse(&ordering),
        block_size: block,
        avoid_repeat_hours: avoid,
        align_minutes: align,
        slots: cfg
            .slots
            .into_iter()
            .map(|s| SlotIo { id: Some(s.id), title: s.title, days: s.days, start_min: s.start_min, end_min: s.end_min, rule: s.rule, ordering: s.ordering })
            .collect(),
    }))
}

fn validate(c: &ChannelIo) -> ApiResult<()> {
    if c.name.trim().is_empty() {
        return Err(ApiError::BadRequest("Channel name is required".into()));
    }
    for s in &c.slots {
        if s.start_min >= 1440 || s.end_min > 1440 || s.days == 0 {
            return Err(ApiError::BadRequest(format!("Block \"{}\" has invalid times or no days", s.title)));
        }
    }
    Ok(())
}

async fn next_number(st: &AppState) -> ApiResult<i64> {
    let max: Option<i64> = sqlx::query_scalar("SELECT MAX(number) FROM channels").fetch_one(&st.db).await?;
    Ok(max.unwrap_or(0) + 1)
}

async fn save_slots(tx: &mut sqlx::SqliteConnection, channel_id: i64, slots: &[SlotIo]) -> ApiResult<()> {
    sqlx::query("DELETE FROM channel_slots WHERE channel_id = ?").bind(channel_id).execute(&mut *tx).await?;
    for s in slots {
        sqlx::query(
            "INSERT INTO channel_slots (id, channel_id, title, days, start_min, end_min, rule, ordering) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(s.id)
        .bind(channel_id)
        .bind(&s.title)
        .bind(s.days as i64)
        .bind(s.start_min as i64)
        .bind((s.end_min % 1440) as i64)
        .bind(serde_json::to_string(&s.rule)?)
        .bind(ordering_str(s.ordering))
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

fn ordering_str(o: Ordering) -> String {
    serde_json::to_value(o).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "shuffle".into())
}

fn conflict(e: sqlx::Error) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.message().contains("UNIQUE") => ApiError::Conflict("That channel number is already taken".into()),
        _ => e.into(),
    }
}

async fn insert_channel(st: &AppState, c: &ChannelIo) -> ApiResult<i64> {
    validate(c)?;
    let number = match c.number {
        Some(n) => n,
        None => next_number(st).await?,
    };
    let now = now_ms();
    let color = c.color.clone().unwrap_or_else(|| PALETTE[(number as usize) % PALETTE.len()].to_string());
    let mut tx = st.db.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO channels (number, name, color, logo, owner_user_id, rule, ordering, block_size, avoid_repeat_hours, align_minutes, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(number)
    .bind(c.name.trim())
    .bind(color)
    .bind(&c.logo)
    .bind(c.owner_user_id)
    .bind(serde_json::to_string(&c.rule)?)
    .bind(ordering_str(c.ordering))
    .bind(c.block_size.max(1))
    .bind(c.avoid_repeat_hours.max(0))
    .bind(c.align_minutes.clamp(0, 60))
    .bind(now)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .map_err(conflict)?;
    save_slots(&mut tx, id, &c.slots).await?;
    tx.commit().await?;
    Ok(id)
}

async fn build(st: &AppState, ids: &[i64], rebuild: bool) -> ApiResult<()> {
    let _guard = st.guide_lock.lock().await;
    let items = store::load_items(&st.db).await?;
    for &id in ids {
        if rebuild {
            store::rebuild_channel(&st.db, &items, id, st.cfg.tz, now_ms()).await?;
        } else {
            store::extend_channel(&st.db, &items, id, st.cfg.tz, now_ms()).await?;
        }
    }
    Ok(())
}

pub async fn create(State(st): State<AppState>, Json(c): Json<ChannelIo>) -> ApiResult<Json<serde_json::Value>> {
    let id = insert_channel(&st, &c).await?;
    build(&st, &[id], false).await?;
    Ok(Json(serde_json::json!({ "id": id })))
}

pub async fn update(State(st): State<AppState>, Path(id): Path<i64>, Json(c): Json<ChannelIo>) -> ApiResult<Json<()>> {
    validate(&c)?;
    let mut tx = st.db.begin().await?;
    let res = sqlx::query(
        "UPDATE channels SET number = COALESCE(?, number), name = ?, color = COALESCE(?, color), logo = ?, owner_user_id = ?,
            rule = ?, ordering = ?, block_size = ?, avoid_repeat_hours = ?, align_minutes = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(c.number)
    .bind(c.name.trim())
    .bind(&c.color)
    .bind(&c.logo)
    .bind(c.owner_user_id)
    .bind(serde_json::to_string(&c.rule)?)
    .bind(ordering_str(c.ordering))
    .bind(c.block_size.max(1))
    .bind(c.avoid_repeat_hours.max(0))
    .bind(c.align_minutes.clamp(0, 60))
    .bind(now_ms())
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(conflict)?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    save_slots(&mut tx, id, &c.slots).await?;
    tx.commit().await?;
    build(&st, &[id], true).await?;
    Ok(Json(()))
}

pub async fn remove(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<()>> {
    sqlx::query("DELETE FROM channels WHERE id = ?").bind(id).execute(&st.db).await?;
    Ok(Json(()))
}

pub async fn rebuild(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<()>> {
    build(&st, &[id], true).await?;
    Ok(Json(()))
}

pub async fn rebuild_all(State(st): State<AppState>) -> ApiResult<Json<()>> {
    let _guard = st.guide_lock.lock().await;
    store::rebuild_all(&st.db, st.cfg.tz, now_ms()).await?;
    Ok(Json(()))
}

#[derive(Serialize)]
pub struct PreviewProgram {
    start: i64,
    end: i64,
    title: String,
    subtitle: Option<String>,
    block: Option<String>,
    filler: bool,
}

/// The next 24 hours of a channel that hasn't been saved yet.
pub async fn preview(State(st): State<AppState>, Json(c): Json<ChannelIo>) -> ApiResult<Json<Vec<PreviewProgram>>> {
    let items = store::load_items(&st.db).await?;
    let slots: Vec<Slot> = c
        .slots
        .iter()
        .enumerate()
        .map(|(i, s)| Slot {
            id: s.id.unwrap_or(-(i as i64) - 1),
            title: s.title.clone(),
            days: s.days,
            start_min: s.start_min,
            end_min: s.end_min % 1440,
            rule: s.rule.clone(),
            ordering: s.ordering,
        })
        .collect();
    let cfg = ChannelConfig {
        rule: c.rule.clone(),
        ordering: c.ordering,
        block_size: c.block_size.max(1) as usize,
        avoid_repeat_hours: c.avoid_repeat_hours,
        align_minutes: c.align_minutes,
        slots: slots.clone(),
    };
    let now = now_ms();
    let from = now.div_euclid(1_800_000) * 1_800_000;
    let mut rng = rand::rngs::StdRng::from_entropy();
    let programs = generate(&cfg, &items, &mut ChannelState::default(), st.cfg.tz, from, from + 24 * 3_600_000, &mut rng);
    Ok(Json(
        programs
            .into_iter()
            .map(|p| {
                let block = p.slot_id.and_then(|id| slots.iter().find(|s| s.id == id)).map(|s| s.title.clone());
                match p.item.map(|ix| &items[ix]) {
                    Some(it) => PreviewProgram {
                        start: p.start_ms,
                        end: p.end_ms,
                        title: it.show_title.clone().filter(|_| !it.is_movie).unwrap_or_else(|| it.title.clone()),
                        subtitle: if it.is_movie {
                            it.year.map(|y| y.to_string())
                        } else {
                            Some(format!("S{} E{} · {}", it.season.unwrap_or(0), it.episode.unwrap_or(0), it.title))
                        },
                        block,
                        filler: false,
                    },
                    None => PreviewProgram { start: p.start_ms, end: p.end_ms, title: "Coming Up Next".into(), subtitle: None, block, filler: true },
                }
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct GuideQuery {
    user: Option<i64>,
    from: Option<i64>,
    hours: Option<i64>,
}

#[derive(Serialize)]
pub struct GuideChannel {
    #[serde(flatten)]
    channel: ChannelRow,
    programs: Vec<ProgramOut>,
}

#[derive(Serialize)]
pub struct Guide {
    now: i64,
    from: i64,
    to: i64,
    channels: Vec<GuideChannel>,
}

pub async fn guide(State(st): State<AppState>, Query(q): Query<GuideQuery>) -> ApiResult<Json<Guide>> {
    let now = now_ms();
    let from = q.from.unwrap_or_else(|| now.div_euclid(1_800_000) * 1_800_000);
    let to = from + q.hours.unwrap_or(6).clamp(1, 48) * 3_600_000;
    let channels = guide::lineup(&st.db, q.user, false).await?;
    let ids: Vec<i64> = channels.iter().map(|c| c.id).collect();
    let programs = guide::range(&st.db, &ids, from, to).await?;
    let mut out: Vec<GuideChannel> = channels.into_iter().map(|c| GuideChannel { channel: c, programs: vec![] }).collect();
    for p in &programs {
        if let Some(c) = out.iter_mut().find(|c| c.channel.id == p.channel_id) {
            c.programs.push(p.into());
        }
    }
    Ok(Json(Guide { now, from, to, channels: out }))
}

#[derive(Deserialize)]
pub struct AutogenPick {
    name: String,
    rule: Rule,
    ordering: Ordering,
}

#[derive(Deserialize)]
pub struct AutogenInput {
    channels: Vec<AutogenPick>,
    owner_user_id: Option<i64>,
}

pub async fn create_from_suggestions(State(st): State<AppState>, Json(input): Json<AutogenInput>) -> ApiResult<Json<Vec<i64>>> {
    let mut ids = Vec::new();
    for pick in input.channels {
        let c = ChannelIo {
            id: None,
            number: None,
            name: pick.name,
            color: None,
            logo: None,
            owner_user_id: input.owner_user_id,
            rule: pick.rule,
            ordering: pick.ordering,
            block_size: default_block(),
            avoid_repeat_hours: default_avoid(),
            align_minutes: 0,
            slots: vec![],
        };
        ids.push(insert_channel(&st, &c).await?);
    }
    build(&st, &ids, false).await?;
    Ok(Json(ids))
}
