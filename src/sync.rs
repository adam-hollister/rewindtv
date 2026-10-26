//! Copies library metadata from media servers into the local cache.
//! Upserts in place, so a failed sync never leaves the guide without content.

use sqlx::SqlitePool;

use crate::db::now_ms;
use crate::media::{RemoteItem, Source};
use crate::schedule::store;
use crate::state::AppState;

pub async fn refresh_libraries(db: &SqlitePool, source_id: i64, src: &Source) -> anyhow::Result<()> {
    for lib in src.libraries().await? {
        sqlx::query(
            "INSERT INTO libraries (source_id, key, name, kind) VALUES (?, ?, ?, ?)
             ON CONFLICT(source_id, key) DO UPDATE SET name = excluded.name, kind = excluded.kind",
        )
        .bind(source_id)
        .bind(&lib.key)
        .bind(&lib.name)
        .bind(&lib.kind)
        .execute(db)
        .await?;
    }
    Ok(())
}

async fn upsert(tx: &mut sqlx::SqliteConnection, source_id: i64, library_id: i64, it: &RemoteItem, run: i64) -> anyhow::Result<()> {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO media_items (source_id, library_id, item_key, part_key, kind, title, show_key, show_title,
            season, episode, year, duration_ms, summary, content_rating, studio, thumb, art, video_codec, missing, synced_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?)
         ON CONFLICT(source_id, item_key) DO UPDATE SET
            library_id = excluded.library_id, part_key = excluded.part_key, kind = excluded.kind,
            title = excluded.title, show_key = excluded.show_key, show_title = excluded.show_title,
            season = excluded.season, episode = excluded.episode, year = excluded.year,
            duration_ms = excluded.duration_ms, summary = excluded.summary,
            content_rating = excluded.content_rating, studio = excluded.studio,
            thumb = excluded.thumb, art = excluded.art, video_codec = excluded.video_codec,
            missing = 0, synced_at = excluded.synced_at
         RETURNING id",
    )
    .bind(source_id)
    .bind(library_id)
    .bind(&it.item_key)
    .bind(&it.part_key)
    .bind(it.kind)
    .bind(&it.title)
    .bind(&it.show_key)
    .bind(&it.show_title)
    .bind(it.season)
    .bind(it.episode)
    .bind(it.year)
    .bind(it.duration_ms)
    .bind(&it.summary)
    .bind(&it.content_rating)
    .bind(&it.studio)
    .bind(&it.thumb)
    .bind(&it.art)
    .bind(&it.video_codec)
    .bind(run)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM item_genres WHERE item_id = ?").bind(id).execute(&mut *tx).await?;
    for g in &it.genres {
        sqlx::query("INSERT OR IGNORE INTO item_genres (item_id, genre) VALUES (?, ?)").bind(id).bind(g).execute(&mut *tx).await?;
    }
    sqlx::query("DELETE FROM item_collections WHERE item_id = ?").bind(id).execute(&mut *tx).await?;
    for c in &it.collections {
        sqlx::query("INSERT OR IGNORE INTO item_collections (item_id, collection) VALUES (?, ?)").bind(id).bind(c).execute(&mut *tx).await?;
    }
    Ok(())
}

/// Sync one source. Returns the number of items seen.
pub async fn sync_source(db: &SqlitePool, source_id: i64, src: &Source) -> anyhow::Result<usize> {
    refresh_libraries(db, source_id, src).await?;
    let libs: Vec<(i64, String, String, String)> =
        sqlx::query_as("SELECT id, key, name, kind FROM libraries WHERE source_id = ? AND enabled = 1")
            .bind(source_id)
            .fetch_all(db)
            .await?;
    let mut total = 0;
    for (lib_id, key, name, kind) in libs {
        let run = now_ms();
        let items = src.items(&crate::media::Library { key, name: name.clone(), kind }).await?;
        let mut tx = db.begin().await?;
        for it in &items {
            upsert(&mut tx, source_id, lib_id, it, run).await?;
        }
        // Anything in this library we didn't see this time is gone from the server.
        sqlx::query("UPDATE media_items SET missing = 1 WHERE library_id = ? AND synced_at < ?")
            .bind(lib_id)
            .bind(run)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        tracing::info!("synced {} items from library {name}", items.len());
        total += items.len();
    }
    Ok(total)
}

/// Sync every enabled source, then top up all guides.
pub async fn sync_all(state: &AppState) {
    {
        let mut st = state.sync_status.write().await;
        if st.running {
            return;
        }
        st.running = true;
        st.message = Some("Syncing…".into());
    }
    let sources: Vec<(i64, String)> = sqlx::query_as("SELECT id, name FROM sources WHERE enabled = 1")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    let mut summary = Vec::new();
    for (id, name) in sources {
        let result = match state.source(id).await {
            Ok(src) => sync_source(&state.db, id, &src).await,
            Err(e) => Err(anyhow::anyhow!("{e:?}")),
        };
        let (msg, err) = match result {
            Ok(n) => (format!("{name}: {n} items"), None),
            Err(e) => {
                tracing::warn!("sync {name}: {e:#}");
                (format!("{name}: failed"), Some(format!("{e:#}")))
            }
        };
        let _ = sqlx::query("UPDATE sources SET last_sync = ?, last_error = ? WHERE id = ?")
            .bind(now_ms())
            .bind(err)
            .bind(id)
            .execute(&state.db)
            .await;
        summary.push(msg);
    }
    {
        let _guard = state.guide_lock.lock().await;
        if let Err(e) = store::extend_all(&state.db, state.cfg.tz, now_ms()).await {
            tracing::warn!("extending guides: {e:#}");
        }
    }
    let mut st = state.sync_status.write().await;
    st.running = false;
    st.message = Some(if summary.is_empty() { "No media servers configured".into() } else { summary.join(" · ") });
    st.finished_at = Some(now_ms());
}
