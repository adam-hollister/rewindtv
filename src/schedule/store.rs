//! Loading scheduler inputs from SQLite and persisting generated programs.

use chrono_tz::Tz;
use rand::SeedableRng;
use sqlx::SqlitePool;

use super::ordering::{ChannelState, Ordering};
use super::rule::{Item, Rule};
use super::slots::Slot;
use super::{generate, ChannelConfig};

const HALF_HOUR: i64 = 30 * 60_000;

/// How far ahead the guide is kept filled.
pub const HORIZON_MS: i64 = 7 * 24 * 3600 * 1000;

#[derive(sqlx::FromRow)]
struct ItemRow {
    id: i64,
    library_id: i64,
    kind: String,
    title: String,
    show_key: Option<String>,
    show_title: Option<String>,
    season: Option<i64>,
    episode: Option<i64>,
    year: Option<i64>,
    duration_ms: i64,
    content_rating: Option<String>,
    studio: Option<String>,
    genres: Option<String>,
    collections: Option<String>,
    media_key: Option<String>,
    provider_ids: Option<String>,
}

fn split(s: Option<String>) -> Vec<String> {
    s.map(|s| s.split('\u{1f}').filter(|x| !x.is_empty()).map(str::to_string).collect()).unwrap_or_default()
}

/// Every playable item from enabled sources and libraries.
pub async fn load_items(db: &SqlitePool) -> anyhow::Result<Vec<Item>> {
    let rows: Vec<ItemRow> = sqlx::query_as(
        "SELECT m.id, m.library_id, m.kind, m.title, m.show_key, m.show_title, m.season, m.episode,
                m.year, m.duration_ms, m.content_rating, m.studio,
                (SELECT group_concat(genre, char(31)) FROM item_genres g WHERE g.item_id = m.id) AS genres,
                (SELECT group_concat(collection, char(31)) FROM item_collections c WHERE c.item_id = m.id) AS collections,
                m.media_key, m.provider_ids
         FROM media_items m
         JOIN libraries l ON l.id = m.library_id
         JOIN sources s ON s.id = m.source_id
         WHERE m.missing = 0 AND l.enabled = 1 AND s.enabled = 1
         ORDER BY m.id",
    )
    .fetch_all(db)
    .await?;
    let refs: Vec<crate::markers::ItemRef> = rows
        .iter()
        .map(|r| crate::markers::ItemRef {
            id: r.id,
            media_key: r.media_key.clone(),
            provider_ids: r.provider_ids.as_deref().and_then(|p| serde_json::from_str(p).ok()).unwrap_or_default(),
            season: r.season,
            duration_ms: r.duration_ms,
        })
        .collect();
    let mut markers = crate::markers::resolved(db, &refs).await?;
    Ok(rows
        .into_iter()
        .map(|r| Item {
            markers: markers.remove(&r.id).unwrap_or_default(),
            id: r.id,
            library_id: r.library_id,
            is_movie: r.kind == "movie",
            title: r.title,
            show_key: r.show_key,
            show_title: r.show_title,
            season: r.season,
            episode: r.episode,
            year: r.year,
            duration_ms: r.duration_ms,
            content_rating: r.content_rating,
            studio: r.studio,
            genres: split(r.genres),
            collections: split(r.collections),
        })
        .collect())
}

pub async fn load_config(db: &SqlitePool, channel_id: i64) -> anyhow::Result<Option<ChannelConfig>> {
    let Some((rule, ordering, block_size, avoid, align, skip_intros, skip_credits)) = sqlx::query_as::<_, (String, String, i64, i64, i64, bool, bool)>(
        "SELECT rule, ordering, block_size, avoid_repeat_hours, align_minutes, skip_intros, skip_credits FROM channels WHERE id = ?",
    )
    .bind(channel_id)
    .fetch_optional(db)
    .await?
    else {
        return Ok(None);
    };
    let slots = sqlx::query_as::<_, (i64, String, i64, i64, i64, String, String)>(
        "SELECT id, title, days, start_min, end_min, rule, ordering FROM channel_slots
         WHERE channel_id = ? ORDER BY start_min, id",
    )
    .bind(channel_id)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|(id, title, days, start, end, rule, ordering)| Slot {
        id,
        title,
        days: days as u8,
        start_min: start as u32,
        end_min: end as u32,
        rule: serde_json::from_str(&rule).unwrap_or_default(),
        ordering: Ordering::parse(&ordering),
    })
    .collect();
    Ok(Some(ChannelConfig {
        rule: serde_json::from_str::<Rule>(&rule).unwrap_or_default(),
        ordering: Ordering::parse(&ordering),
        block_size: block_size.max(1) as usize,
        avoid_repeat_hours: avoid,
        align_minutes: align,
        slots,
        skip: super::cuts::Skip { intros: skip_intros, credits: skip_credits },
    }))
}

/// Fill the channel's guide out to `now + HORIZON_MS`.
pub async fn extend_channel(db: &SqlitePool, items: &[Item], channel_id: i64, tz: Tz, now: i64) -> anyhow::Result<()> {
    let Some(cfg) = load_config(db, channel_id).await? else { return Ok(()) };
    let last_end: Option<i64> = sqlx::query_scalar("SELECT MAX(end_ms) FROM programs WHERE channel_id = ?")
        .bind(channel_id)
        .fetch_one(db)
        .await?;
    // Start a fresh guide on the previous half hour so something is already "on".
    let from = match last_end {
        Some(e) if e > now => e,
        _ => now.div_euclid(HALF_HOUR) * HALF_HOUR,
    };
    let until = now + HORIZON_MS;
    if from >= until {
        return Ok(());
    }

    let mut state: ChannelState = sqlx::query_scalar::<_, String>("SELECT state FROM channel_state WHERE channel_id = ?")
        .bind(channel_id)
        .fetch_optional(db)
        .await?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut rng = rand::rngs::StdRng::from_entropy();
    let programs = generate(&cfg, items, &mut state, tz, from, until, &mut rng);

    let mut tx = db.begin().await?;
    for p in &programs {
        sqlx::query("INSERT INTO programs (channel_id, start_ms, end_ms, item_id, slot_id, cuts) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(channel_id)
            .bind(p.start_ms)
            .bind(p.end_ms)
            .bind(p.item.map(|ix| items[ix].id))
            .bind(p.slot_id)
            .bind((!p.cuts.is_empty()).then(|| serde_json::to_string(&p.cuts).unwrap_or_default()))
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("INSERT INTO channel_state (channel_id, state) VALUES (?, ?) ON CONFLICT(channel_id) DO UPDATE SET state = excluded.state")
        .bind(channel_id)
        .bind(serde_json::to_string(&state)?)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Throw away everything that hasn't started yet and regenerate. The program
/// currently on air keeps playing.
pub async fn rebuild_channel(db: &SqlitePool, items: &[Item], channel_id: i64, tz: Tz, now: i64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM programs WHERE channel_id = ? AND start_ms > ?").bind(channel_id).bind(now).execute(db).await?;
    // Drop a trailing filler block that would delay the new lineup.
    sqlx::query("DELETE FROM programs WHERE channel_id = ? AND item_id IS NULL AND end_ms > ?")
        .bind(channel_id)
        .bind(now)
        .execute(db)
        .await?;
    extend_channel(db, items, channel_id, tz, now).await
}

/// Extend every channel and prune old history.
pub async fn extend_all(db: &SqlitePool, tz: Tz, now: i64) -> anyhow::Result<()> {
    let items = load_items(db).await?;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM channels").fetch_all(db).await?;
    for id in ids {
        if let Err(e) = extend_channel(db, &items, id, tz, now).await {
            tracing::warn!("extending channel {id}: {e:#}");
        }
    }
    sqlx::query("DELETE FROM programs WHERE end_ms < ?").bind(now - 24 * 3600 * 1000).execute(db).await?;
    Ok(())
}

/// Regenerate programs starting after `keep_until` on channels that skip
/// intros or credits (after their markers changed).
pub async fn refresh_skipping(db: &SqlitePool, tz: Tz, keep_until: i64) -> anyhow::Result<()> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM channels WHERE skip_intros = 1 OR skip_credits = 1").fetch_all(db).await?;
    if ids.is_empty() {
        return Ok(());
    }
    let items = load_items(db).await?;
    for id in ids {
        sqlx::query("DELETE FROM programs WHERE channel_id = ? AND start_ms > ?").bind(id).bind(keep_until).execute(db).await?;
        extend_channel(db, &items, id, tz, crate::db::now_ms()).await?;
    }
    Ok(())
}

pub async fn rebuild_all(db: &SqlitePool, tz: Tz, now: i64) -> anyhow::Result<()> {
    let items = load_items(db).await?;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM channels").fetch_all(db).await?;
    for id in ids {
        rebuild_channel(db, &items, id, tz, now).await?;
    }
    Ok(())
}
