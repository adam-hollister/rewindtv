//! Read-side queries over the generated programs.

use serde::Serialize;
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ProgramInfo {
    pub id: i64,
    pub channel_id: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub slot_id: Option<i64>,
    pub slot_title: Option<String>,
    pub item_id: Option<i64>,
    pub source_id: Option<i64>,
    #[serde(skip)]
    pub item_key: Option<String>,
    #[serde(skip)]
    pub part_key: Option<String>,
    #[serde(skip)]
    pub video_codec: Option<String>,
    pub kind: Option<String>,
    pub title: Option<String>,
    pub show_title: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub year: Option<i64>,
    pub duration_ms: Option<i64>,
    pub summary: Option<String>,
    pub content_rating: Option<String>,
    pub thumb: Option<String>,
    pub art: Option<String>,
    pub missing: Option<i64>,
    #[serde(skip)]
    pub cuts: Option<String>,
}

impl ProgramInfo {
    /// File-time ranges skipped while this program airs.
    pub fn cut_ranges(&self) -> Vec<(i64, i64)> {
        self.cuts.as_deref().and_then(|c| serde_json::from_str(c).ok()).unwrap_or_default()
    }

    /// Length of the media file (falls back to the slot length).
    pub fn media_len(&self) -> i64 {
        self.duration_ms.unwrap_or(self.end_ms - self.start_ms)
    }

    pub fn is_filler(&self) -> bool {
        self.item_id.is_none() || self.missing == Some(1)
    }

    /// What a TV guide shows as the headline: the show name for episodes.
    pub fn headline(&self) -> String {
        if self.is_filler() {
            return "Coming Up Next".into();
        }
        self.show_title.clone().or_else(|| self.title.clone()).unwrap_or_default()
    }

    /// "S2 E5 · The One With…" for episodes, the year for movies.
    pub fn subtitle(&self) -> Option<String> {
        if self.kind.as_deref() == Some("episode") {
            let se = match (self.season, self.episode) {
                (Some(s), Some(e)) => format!("S{s} E{e} · "),
                _ => String::new(),
            };
            return Some(format!("{se}{}", self.title.clone().unwrap_or_default()));
        }
        self.year.map(|y| y.to_string())
    }
}

const SELECT: &str = "SELECT p.id, p.channel_id, p.start_ms, p.end_ms, p.slot_id, cs.title AS slot_title,
        p.item_id, m.source_id, m.item_key, m.part_key, m.video_codec, m.kind, m.title, m.show_title, m.season, m.episode,
        m.year, m.duration_ms, m.summary, m.content_rating, m.thumb, m.art, m.missing, p.cuts
    FROM programs p
    LEFT JOIN media_items m ON m.id = p.item_id
    LEFT JOIN channel_slots cs ON cs.id = p.slot_id";

pub async fn at(db: &SqlitePool, channel_id: i64, t_ms: i64) -> sqlx::Result<Option<ProgramInfo>> {
    sqlx::query_as(&format!("{SELECT} WHERE p.channel_id = ? AND p.start_ms <= ? AND p.end_ms > ? ORDER BY p.start_ms DESC LIMIT 1"))
        .bind(channel_id)
        .bind(t_ms)
        .bind(t_ms)
        .fetch_optional(db)
        .await
}

pub async fn upcoming(db: &SqlitePool, channel_id: i64, after_ms: i64, limit: i64) -> sqlx::Result<Vec<ProgramInfo>> {
    sqlx::query_as(&format!("{SELECT} WHERE p.channel_id = ? AND p.start_ms >= ? ORDER BY p.start_ms LIMIT ?"))
        .bind(channel_id)
        .bind(after_ms)
        .bind(limit)
        .fetch_all(db)
        .await
}

/// Programs overlapping [from, to) for the given channels, ordered by channel then time.
pub async fn range(db: &SqlitePool, channel_ids: &[i64], from_ms: i64, to_ms: i64) -> sqlx::Result<Vec<ProgramInfo>> {
    if channel_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids = channel_ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    sqlx::query_as(&format!(
        "{SELECT} WHERE p.channel_id IN ({ids}) AND p.end_ms > ? AND p.start_ms < ? ORDER BY p.channel_id, p.start_ms"
    ))
    .bind(from_ms)
    .bind(to_ms)
    .fetch_all(db)
    .await
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ChannelRow {
    pub id: i64,
    pub number: i64,
    pub name: String,
    pub color: String,
    pub logo: Option<String>,
    pub owner_user_id: Option<i64>,
    pub favorite: i64,
    pub hidden: i64,
}

/// Channels a user can see: shared ones plus their own. `include_hidden` is for
/// the lineup editor.
pub async fn lineup(db: &SqlitePool, user_id: Option<i64>, include_hidden: bool) -> sqlx::Result<Vec<ChannelRow>> {
    let uid = user_id.unwrap_or(-1);
    sqlx::query_as(
        "SELECT c.id, c.number, c.name, c.color, c.logo, c.owner_user_id,
                COALESCE(uc.favorite, 0) AS favorite, COALESCE(uc.hidden, 0) AS hidden
         FROM channels c
         LEFT JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = ?
         WHERE (c.owner_user_id IS NULL OR c.owner_user_id = ?)
           AND (? OR COALESCE(uc.hidden, 0) = 0)
         ORDER BY c.number",
    )
    .bind(uid)
    .bind(uid)
    .bind(include_hidden)
    .fetch_all(db)
    .await
}
