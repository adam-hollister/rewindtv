//! Guide search ("when is Seinfeld on?") and per-profile reminders.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use super::channels::img;
use crate::db::now_ms;
use crate::error::{ApiError, ApiResult};
use crate::guide;
use crate::state::AppState;

const WEEK_MS: i64 = 7 * 24 * 3_600_000;

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    user: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Hit {
    program_id: i64,
    channel_id: i64,
    start: i64,
    end: i64,
    kind: Option<String>,
    title: String,
    show_title: Option<String>,
    season: Option<i64>,
    episode: Option<i64>,
    source_id: Option<i64>,
    thumb: Option<String>,
}

#[derive(Serialize)]
pub struct HitOut {
    program_id: i64,
    channel_id: i64,
    channel_number: i64,
    channel_name: String,
    channel_color: String,
    channel_logo: Option<String>,
    start: i64,
    end: i64,
    /// Show name for episodes, title for movies.
    title: String,
    subtitle: Option<String>,
    thumb: Option<String>,
}

/// GET /api/guide/search?q=…&user=… — upcoming (and current) airings in the
/// profile's lineup over the next week, matching show, movie or episode titles.
pub async fn search(State(st): State<AppState>, Query(q): Query<SearchQuery>) -> ApiResult<Json<Vec<HitOut>>> {
    let term = q.q.trim().to_lowercase();
    if term.chars().count() < 2 {
        return Ok(Json(Vec::new()));
    }
    let lineup = guide::lineup(&st.db, q.user, false).await?;
    if lineup.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let ids = lineup.iter().map(|c| c.id.to_string()).collect::<Vec<_>>().join(",");
    // LIKE with the wildcards escaped, so "100%" means a literal percent.
    let pattern = format!("%{}%", term.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let now = now_ms();
    let hits: Vec<Hit> = sqlx::query_as(&format!(
        "SELECT p.id AS program_id, p.channel_id, p.start_ms AS start, p.end_ms AS end, m.kind, m.title, m.show_title,
                m.season, m.episode, m.source_id, m.thumb
         FROM programs p JOIN media_items m ON m.id = p.item_id
         WHERE p.channel_id IN ({ids}) AND p.kind = 'show' AND p.end_ms > ? AND p.start_ms < ?
           AND (lower(m.title) LIKE ?3 ESCAPE '\\' OR lower(coalesce(m.show_title, '')) LIKE ?3 ESCAPE '\\')
         ORDER BY p.start_ms LIMIT 200"
    ))
    .bind(now)
    .bind(now + WEEK_MS)
    .bind(&pattern)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(
        hits.into_iter()
            .filter_map(|h| {
                let ch = lineup.iter().find(|c| c.id == h.channel_id)?;
                let episode = h.kind.as_deref() == Some("episode");
                Some(HitOut {
                    program_id: h.program_id,
                    channel_id: h.channel_id,
                    channel_number: ch.number,
                    channel_name: ch.name.clone(),
                    channel_color: ch.color.clone(),
                    channel_logo: ch.logo.clone(),
                    start: h.start,
                    end: h.end,
                    subtitle: if episode {
                        Some(match (h.season, h.episode) {
                            (Some(s), Some(e)) => format!("S{s} E{e} · {}", h.title),
                            _ => h.title.clone(),
                        })
                    } else {
                        None
                    },
                    title: if episode { h.show_title.unwrap_or(h.title) } else { h.title },
                    thumb: img(h.source_id, &h.thumb, 200),
                })
            })
            .collect(),
    ))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Reminder {
    program_id: i64,
    channel_id: i64,
    start_ms: i64,
    title: String,
}

#[derive(Deserialize)]
pub struct UserQuery {
    user: i64,
}

/// GET /api/reminders?user=… — the profile's upcoming reminders.
pub async fn list(State(st): State<AppState>, Query(q): Query<UserQuery>) -> ApiResult<Json<Vec<Reminder>>> {
    Ok(Json(
        sqlx::query_as("SELECT program_id, channel_id, start_ms, title FROM reminders WHERE user_id = ? AND start_ms > ? ORDER BY start_ms")
            .bind(q.user)
            .bind(now_ms() - 10 * 60_000)
            .fetch_all(&st.db)
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct NewReminder {
    user_id: i64,
    program_id: i64,
}

/// POST /api/reminders — remind a profile when a program starts.
pub async fn add(State(st): State<AppState>, Json(r): Json<NewReminder>) -> ApiResult<Json<()>> {
    let p = guide::by_id(&st.db, r.program_id).await?.ok_or(ApiError::NotFound)?;
    if p.start_ms <= now_ms() {
        return Err(ApiError::BadRequest("That's already on — just tune in".into()));
    }
    sqlx::query("INSERT OR REPLACE INTO reminders (user_id, program_id, channel_id, start_ms, title) VALUES (?, ?, ?, ?, ?)")
        .bind(r.user_id)
        .bind(r.program_id)
        .bind(p.channel_id)
        .bind(p.start_ms)
        .bind(p.headline())
        .execute(&st.db)
        .await?;
    Ok(Json(()))
}

/// DELETE /api/reminders/:user/:program
pub async fn remove(State(st): State<AppState>, Path((user, program)): Path<(i64, i64)>) -> ApiResult<Json<()>> {
    sqlx::query("DELETE FROM reminders WHERE user_id = ? AND program_id = ?").bind(user).bind(program).execute(&st.db).await?;
    Ok(Json(()))
}

/// Drop reminders for shows that started over an hour ago.
pub async fn prune(db: &sqlx::SqlitePool) {
    let _ = sqlx::query("DELETE FROM reminders WHERE start_ms < ?").bind(now_ms() - 3_600_000).execute(db).await;
}
