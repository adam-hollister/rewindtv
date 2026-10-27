//! The markers folder: one JSON file per show (`tv/Cowboy Bebop.json`) or
//! movie (`movies/Akira (1988).json`), kept apart from the media. RewindTV
//! writes a file whenever its markers change and merges every file back in at
//! startup and before each sync (newest `updated_at` wins), so markers survive
//! reinstalls, can be backed up, and can be shared between installs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::{rules, save_rule, Rule, StoredMarker};

const VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    #[serde(default)]
    pub markers: Vec<StoredMarker>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MarkerFile {
    pub version: u32,
    /// "tv/<show>" or "movie/<title>/<year>".
    pub key: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// Episodes by "s01e05"; a movie uses the single key "movie".
    #[serde(default)]
    pub items: BTreeMap<String, Entry>,
}

/// Which file a media key lives in: (file key, entry name).
pub fn file_key(media_key: &str) -> (String, String) {
    if media_key.starts_with("tv/") {
        if let Some((show, ep)) = media_key.rsplit_once('/') {
            return (show.to_string(), ep.to_string());
        }
    }
    (media_key.to_string(), "movie".to_string())
}

fn safe(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() { "untitled".into() } else { trimmed }
}

fn path_for(dir: &Path, key: &str, title: &str) -> PathBuf {
    let sub = if key.starts_with("tv/") { "tv" } else { "movies" };
    dir.join(sub).join(format!("{}.json", safe(title)))
}

/// A human title for a file key, from whatever the servers call it.
async fn title_for(db: &SqlitePool, key: &str) -> String {
    let row: Option<(String, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT title, show_title, year FROM media_items WHERE media_key = ?1 OR media_key LIKE ?1 || '/%' LIMIT 1",
    )
    .bind(key)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    match row {
        Some((_, Some(show), _)) if key.starts_with("tv/") => show,
        Some((title, _, Some(year))) => format!("{title} ({year})"),
        Some((title, _, None)) => title,
        None => key.rsplit('/').next().unwrap_or(key).to_string(),
    }
}

/// Write (or remove) the file holding `media_key`'s markers. `media_key` is
/// an episode/movie key (or `tv/<show>/rule` for a show's rules).
pub async fn export(db: &SqlitePool, dir: &Path, media_key: &str) -> anyhow::Result<()> {
    let (key, _) = file_key(media_key);
    let rows: Vec<(String, String, String, i64, i64, i64, Option<f64>, i64)> = sqlx::query_as(
        "SELECT media_key, kind, source, start_ms, end_ms, duration_ms, confidence, updated_at FROM markers
         WHERE media_key = ?1 OR media_key LIKE ?1 || '/%' ORDER BY media_key, kind, source",
    )
    .bind(&key)
    .fetch_all(db)
    .await?;
    let ids: Vec<(String, String)> = sqlx::query_as(
        "SELECT provider_id, media_key FROM marker_ids WHERE media_key = ?1 OR media_key LIKE ?1 || '/%' ORDER BY provider_id",
    )
    .bind(&key)
    .fetch_all(db)
    .await?;
    let show_rules = if key.starts_with("tv/") { rules(db, Some(&key)).await? } else { Vec::new() };

    let mut items: BTreeMap<String, Entry> = BTreeMap::new();
    for (mk, kind, source, start_ms, end_ms, duration_ms, confidence, updated_at) in rows {
        let (_, name) = file_key(&mk);
        items.entry(name).or_default().markers.push(StoredMarker { kind, source, start_ms, end_ms, duration_ms, confidence, updated_at });
    }
    for (pid, mk) in ids {
        let (_, name) = file_key(&mk);
        if let Some(e) = items.get_mut(&name) {
            e.ids.push(pid);
        }
    }
    let title = title_for(db, &key).await;
    let path = path_for(dir, &key, &title);
    if items.is_empty() && show_rules.is_empty() {
        let _ = tokio::fs::remove_file(&path).await;
        return Ok(());
    }
    let file = MarkerFile { version: VERSION, key, title, rules: show_rules, items };
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Write atomically so a crash never leaves half a file.
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, serde_json::to_vec_pretty(&file)?).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

const SETTINGS_FILE: &str = "settings.json";

/// Remember which marker sources are on, next to the markers themselves.
pub async fn save_sources(dir: &Path, s: crate::schedule::cuts::Sources) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    tokio::fs::write(dir.join(SETTINGS_FILE), serde_json::to_vec_pretty(&s)?).await?;
    Ok(())
}

/// Restore the marker source choices into a database that has none yet
/// (a fresh install pointed at an existing markers folder).
pub async fn restore_sources(db: &SqlitePool, dir: &Path) -> anyhow::Result<()> {
    let has: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings WHERE key LIKE 'markers.%'").fetch_one(db).await?;
    if has > 0 {
        return Ok(());
    }
    if let Ok(bytes) = tokio::fs::read(dir.join(SETTINGS_FILE)).await {
        let s: crate::schedule::cuts::Sources = serde_json::from_slice(&bytes)?;
        super::set_sources(db, s).await?;
    }
    Ok(())
}

/// Merge every file in the folder into the database. Newer entries win;
/// returns how many markers/rules changed.
pub async fn import_all(db: &SqlitePool, dir: &Path) -> anyhow::Result<usize> {
    let mut changed = 0;
    for sub in ["tv", "movies"] {
        let Ok(mut rd) = tokio::fs::read_dir(dir.join(sub)).await else { continue };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match tokio::fs::read(&path).await.map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice::<MarkerFile>(&b)?)) {
                Ok(f) => changed += import_file(db, &f).await?,
                Err(e) => tracing::warn!("skipping marker file {}: {e:#}", path.display()),
            }
        }
    }
    Ok(changed)
}

pub async fn import_file(db: &SqlitePool, f: &MarkerFile) -> anyhow::Result<usize> {
    let mut changed = 0;
    for (name, entry) in &f.items {
        let media_key = if name == "movie" { f.key.clone() } else { format!("{}/{}", f.key, name) };
        super::link_ids(db, &media_key, &entry.ids).await?;
        for m in &entry.markers {
            let res = sqlx::query(
                "INSERT INTO markers (media_key, kind, source, start_ms, end_ms, duration_ms, confidence, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(media_key, kind, source) DO UPDATE SET start_ms = excluded.start_ms, end_ms = excluded.end_ms,
                    duration_ms = excluded.duration_ms, confidence = excluded.confidence, updated_at = excluded.updated_at
                 WHERE excluded.updated_at > markers.updated_at",
            )
            .bind(&media_key)
            .bind(&m.kind)
            .bind(&m.source)
            .bind(m.start_ms)
            .bind(m.end_ms)
            .bind(m.duration_ms)
            .bind(m.confidence)
            .bind(m.updated_at)
            .execute(db)
            .await?;
            changed += res.rows_affected() as usize;
        }
    }
    let existing = rules(db, Some(&f.key)).await?;
    for r in &f.rules {
        let newer = existing
            .iter()
            .find(|e| e.season == r.season && e.kind == r.kind)
            .is_none_or(|e| r.updated_at > e.updated_at);
        if newer {
            let mut r = r.clone();
            r.show_key = f.key.clone();
            save_rule(db, &r).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_layout() {
        assert_eq!(file_key("tv/cowboy-bebop/s01e05"), ("tv/cowboy-bebop".into(), "s01e05".into()));
        assert_eq!(file_key("movie/akira/1988"), ("movie/akira/1988".into(), "movie".into()));
        let dir = Path::new("/markers");
        assert_eq!(path_for(dir, "tv/x", "Marvel's Agents of S.H.I.E.L.D."), Path::new("/markers/tv/Marvel's Agents of S.H.I.E.L.D.json"));
        assert_eq!(path_for(dir, "movie/x/1", "AC/DC: Live (1992)"), Path::new("/markers/movies/AC_DC_ Live (1992).json"));
    }
}
