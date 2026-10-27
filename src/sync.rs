//! Copies library metadata from media servers into the local cache.
//! Upserts in place, so a failed sync never leaves the guide without content.

use std::collections::{HashMap, HashSet};

use sqlx::SqlitePool;

use crate::db::now_ms;
use crate::markers;
use crate::schedule::cuts::Source as MarkerSource;
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
            season, episode, year, duration_ms, summary, content_rating, studio, thumb, art, video_codec,
            media_key, provider_ids, file_path, source_updated_at, missing, synced_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?)
         ON CONFLICT(source_id, item_key) DO UPDATE SET
            library_id = excluded.library_id, part_key = excluded.part_key, kind = excluded.kind,
            title = excluded.title, show_key = excluded.show_key, show_title = excluded.show_title,
            season = excluded.season, episode = excluded.episode, year = excluded.year,
            duration_ms = excluded.duration_ms, summary = excluded.summary,
            content_rating = excluded.content_rating, studio = excluded.studio,
            thumb = excluded.thumb, art = excluded.art, video_codec = excluded.video_codec,
            media_key = excluded.media_key, provider_ids = excluded.provider_ids, file_path = excluded.file_path,
            detect_checked_at = CASE WHEN excluded.source_updated_at IS media_items.source_updated_at
                                     THEN media_items.detect_checked_at ELSE NULL END,
            -- Changed on the server: look for markers again.
            markers_checked_at = CASE WHEN excluded.source_updated_at IS media_items.source_updated_at
                                      THEN media_items.markers_checked_at ELSE NULL END,
            source_updated_at = excluded.source_updated_at,
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
    .bind(markers::media_key(it.kind, &it.title, it.show_title.as_deref(), it.season, it.episode, it.year))
    .bind(serde_json::to_string(&it.provider_ids)?)
    .bind(&it.file_path)
    .bind(it.updated_at)
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

/// Sync one source. Returns the number of items seen, plus any markers the
/// library listings already carried (by item key).
pub async fn sync_source(db: &SqlitePool, source_id: i64, src: &Source) -> anyhow::Result<(usize, Vec<(String, Vec<crate::media::ServerMarker>)>)> {
    let mut listed_markers = Vec::new();
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
        listed_markers.extend(items.into_iter().filter_map(|it| Some((it.item_key, it.markers?))));
    }
    Ok((total, listed_markers))
}

/// Re-check items without server markers this often (Plex detects markers
/// some time after an item is added).
const MARKER_RECHECK_MS: i64 = 7 * 24 * 3600 * 1000;

/// Pull intro/credits markers from the server into the shared marker store,
/// for items that are new, changed, or due a re-check.
pub async fn sync_markers(state: &AppState, source_id: i64, src: &Source, listed: Vec<(String, Vec<crate::media::ServerMarker>)>) -> anyhow::Result<usize> {
    let db = &state.db;
    let now = now_ms();
    let listed: HashMap<String, Vec<crate::media::ServerMarker>> = listed.into_iter().collect();
    let due: Vec<(i64, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT m.id, m.item_key, m.media_key, m.duration_ms, m.provider_ids FROM media_items m
         WHERE m.source_id = ? AND m.missing = 0 AND m.media_key IS NOT NULL
           AND (m.markers_checked_at IS NULL
                OR (m.markers_checked_at < ? AND NOT EXISTS
                      (SELECT 1 FROM markers k WHERE k.media_key = m.media_key AND k.source = 'server')))",
    )
    .bind(source_id)
    .bind(now - MARKER_RECHECK_MS)
    .fetch_all(db)
    .await?;

    let mut supported = true;
    let mut found = 0;
    let mut touched: HashSet<String> = HashSet::new();
    for chunk in due.chunks(6) {
        let fetched = futures::future::join_all(chunk.iter().map(|(_, key, ..)| async {
            match listed.get(key) {
                Some(m) if !m.is_empty() => Ok(Some(m.clone())),
                _ if !supported => Ok(None),
                _ => src.markers(key).await,
            }
        }))
        .await;
        for ((id, _, media_key, duration, ids), res) in chunk.iter().zip(fetched) {
            match res {
                Ok(Some(list)) => {
                    let ids: Vec<String> = ids.as_deref().and_then(|p| serde_json::from_str(p).ok()).unwrap_or_default();
                    markers::link_ids(db, media_key, &ids).await?;
                    // An empty answer keeps markers another server already gave us.
                    if !list.is_empty() {
                        let rows: Vec<_> = list.iter().map(|m| (m.kind, m.start_ms, m.end_ms, None)).collect();
                        markers::save(db, media_key, MarkerSource::Server, *duration, &rows).await?;
                        touched.insert(media_key.clone());
                        found += 1;
                    }
                }
                Ok(None) => supported = false,
                Err(e) => {
                    tracing::debug!("markers for {media_key}: {e:#}");
                    continue;
                }
            }
            sqlx::query("UPDATE media_items SET markers_checked_at = ? WHERE id = ?").bind(now).bind(id).execute(db).await?;
        }
    }
    export_touched(state, touched).await;
    Ok(found)
}

/// Rewrite the marker files for these media keys.
pub async fn export_touched(state: &AppState, keys: HashSet<String>) {
    // One export per file; `export` takes any media key inside that file.
    let files: HashMap<String, String> = keys.into_iter().map(|k| (markers::files::file_key(&k).0, k)).collect();
    for (f, key) in files {
        if let Err(e) = markers::files::export(&state.db, &state.cfg.markers_dir, &key).await {
            tracing::warn!("writing marker file for {f}: {e:#}");
        }
    }
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
    // Markers saved in the markers folder (by this or another install) first.
    let mut marker_changes = match markers::files::import_all(&state.db, &state.cfg.markers_dir).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("reading marker files: {e:#}");
            0
        }
    };
    let use_server_markers = markers::sources(&state.db).await.server;
    for (id, name) in sources {
        let result = match state.source(id).await {
            Ok(src) => match sync_source(&state.db, id, &src).await {
                Ok((n, listed)) if use_server_markers => {
                    let found = sync_markers(state, id, &src, listed).await.unwrap_or_else(|e| {
                        tracing::warn!("markers from {name}: {e:#}");
                        0
                    });
                    marker_changes += found;
                    Ok(n)
                }
                Ok((n, _)) => Ok(n),
                Err(e) => Err(e),
            },
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
        if marker_changes > 0 {
            // New markers change program lengths; refresh skipping channels
            // beyond the next few hours so what's coming up soon stays put.
            if let Err(e) = store::refresh_skipping(&state.db, state.cfg.tz, now_ms() + 6 * 3_600_000).await {
                tracing::warn!("refreshing skipping channels: {e:#}");
            }
        }
        if let Err(e) = store::extend_all(&state.db, state.cfg.tz, now_ms()).await {
            tracing::warn!("extending guides: {e:#}");
        }
    }
    if markers::sources(&state.db).await.detect {
        tokio::spawn(markers::detect::run(state.clone(), None));
    }
    let mut st = state.sync_status.write().await;
    st.running = false;
    st.message = Some(if summary.is_empty() { "No media servers configured".into() } else { summary.join(" · ") });
    st.finished_at = Some(now_ms());
}
