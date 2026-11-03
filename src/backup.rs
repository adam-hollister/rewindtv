//! Backup and restore: everything you set up (profiles, servers, channels,
//! markers, logos, settings) in one JSON file. The guide and caches are left
//! out; they rebuild on their own after a restore.
//!
//! Tables are dumped generically (column names come from SQLite itself) and
//! restored into whichever of those columns the current schema has, so older
//! and newer backups keep working as the app evolves.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use chrono::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::SqlitePool;

use crate::db::now_ms;
use crate::state::AppState;

const FORMAT: &str = "rewindtv-backup";
const VERSION: u32 = 1;
/// Restore order (parents before children). Deleted in reverse.
const TABLES: &[&str] = &[
    "users",
    "sources",
    "libraries",
    "channels",
    "channel_slots",
    "user_channels",
    "channel_state",
    "reminders",
    "settings",
    "marker_rules",
    "markers",
    "marker_ids",
];
const KEEP_NIGHTLY: usize = 14;
const KEEP_SAFETY: usize = 5;

#[derive(Debug, Serialize, Deserialize)]
pub struct Backup {
    pub format: String,
    pub version: u32,
    pub app_version: String,
    pub exported_at: i64,
    pub tables: BTreeMap<String, Vec<Map<String, Value>>>,
    /// Logo file name -> base64 bytes.
    #[serde(default)]
    pub logos: BTreeMap<String, String>,
}

async fn columns(db: &SqlitePool, table: &str) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{table}') ORDER BY cid")).fetch_all(db).await
}

/// Snapshot the database (and logo files) into a backup.
pub async fn export(db: &SqlitePool, logos_dir: &Path) -> anyhow::Result<Backup> {
    let mut tables = BTreeMap::new();
    for &table in TABLES {
        let cols = columns(db, table).await?;
        // json_object() renders each row (with proper types) inside SQLite.
        let pairs = cols.iter().map(|c| format!("'{c}', \"{c}\"")).collect::<Vec<_>>().join(", ");
        let rows: Vec<String> = sqlx::query_scalar(&format!("SELECT json_object({pairs}) FROM {table} ORDER BY rowid")).fetch_all(db).await?;
        let rows = rows.iter().map(|r| serde_json::from_str::<Map<String, Value>>(r)).collect::<Result<Vec<_>, _>>()?;
        tables.insert(table.to_string(), rows);
    }
    let mut logos = BTreeMap::new();
    let used: Vec<Option<String>> = sqlx::query_scalar("SELECT logo FROM channels").fetch_all(db).await?;
    for name in used.into_iter().flatten() {
        if let Ok(bytes) = tokio::fs::read(logos_dir.join(&name)).await {
            logos.insert(name, base64::engine::general_purpose::STANDARD.encode(bytes));
        }
    }
    Ok(Backup { format: FORMAT.into(), version: VERSION, app_version: env!("CARGO_PKG_VERSION").into(), exported_at: now_ms(), tables, logos })
}

pub fn parse(bytes: &[u8]) -> anyhow::Result<Backup> {
    let b: Backup = serde_json::from_slice(bytes).map_err(|e| anyhow::anyhow!("This isn't a RewindTV backup file ({e})"))?;
    if b.format != FORMAT {
        anyhow::bail!("This isn't a RewindTV backup file");
    }
    if b.version > VERSION {
        anyhow::bail!("This backup is from a newer RewindTV ({}); update RewindTV first", b.app_version);
    }
    for name in b.logos.keys() {
        if !crate::api::logo_name_ok(name) {
            anyhow::bail!("The backup contains an invalid logo name");
        }
    }
    Ok(b)
}

fn bind<'q>(q: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>, v: &Value) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    match v {
        Value::Null => q.bind(None::<String>),
        Value::Bool(b) => q.bind(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => q.bind(i),
            None => q.bind(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => q.bind(s.clone()),
        other => q.bind(other.to_string()),
    }
}

/// Replace the current setup with the backup's, in one transaction: on any
/// error nothing changes.
pub async fn restore_db(db: &SqlitePool, b: &Backup) -> anyhow::Result<()> {
    let mut schema: BTreeMap<&str, HashSet<String>> = BTreeMap::new();
    for &t in TABLES {
        schema.insert(t, columns(db, t).await?.into_iter().collect());
    }
    let mut tx = db.begin().await?;
    // The guide is regenerated; media items come back with the next sync.
    sqlx::query("DELETE FROM programs").execute(&mut *tx).await?;
    for &t in TABLES.iter().rev() {
        sqlx::query(&format!("DELETE FROM {t}")).execute(&mut *tx).await?;
    }
    for &t in TABLES {
        let known = &schema[t];
        for row in b.tables.get(t).map(Vec::as_slice).unwrap_or_default() {
            let cols: Vec<&String> = row.keys().filter(|c| known.contains(*c)).collect();
            if cols.is_empty() {
                continue;
            }
            let sql = format!(
                "INSERT INTO {t} ({}) VALUES ({})",
                cols.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", "),
                vec!["?"; cols.len()].join(", ")
            );
            let mut q = sqlx::query(&sql);
            for c in &cols {
                q = bind(q, &row[*c]);
            }
            q.execute(&mut *tx).await.map_err(|e| anyhow::anyhow!("restoring {t}: {e}"))?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Put logo files back and drop ones nothing uses any more.
async fn restore_logos(dir: &Path, b: &Backup) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    for (name, data) in &b.logos {
        let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
        if crate::api::logo_type_ok(&bytes) {
            tokio::fs::write(dir.join(name), bytes).await?;
        }
    }
    if let Ok(mut rd) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(e)) = rd.next_entry().await {
            let name = e.file_name().to_string_lossy().into_owned();
            if !b.logos.contains_key(&name) {
                let _ = tokio::fs::remove_file(e.path()).await;
            }
        }
    }
    Ok(())
}

/// Full restore: safety backup of the current setup, database, logos, marker
/// files, then a sync to bring the library and guides back.
pub async fn restore(st: &AppState, b: Backup) -> anyhow::Result<()> {
    let safety = write(st, &format!("rewindtv-before-restore-{}.json", stamp(st, "%Y-%m-%d-%H%M%S"))).await?;
    tracing::info!("saved the current setup to {} before restoring", safety.display());
    restore_db(&st.db, &b).await?;
    restore_logos(&st.cfg.logos_dir, &b).await?;
    st.invalidate_items().await;
    let s2 = st.clone();
    tokio::spawn(async move {
        crate::sync::sync_all(&s2).await;
        // Rewrite every marker file from what was restored, once the library
        // is back so files get proper show names.
        let mut keys: HashSet<String> = sqlx::query_scalar::<_, String>("SELECT DISTINCT media_key FROM markers")
            .fetch_all(&s2.db)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
        for show in sqlx::query_scalar::<_, String>("SELECT DISTINCT show_key FROM marker_rules").fetch_all(&s2.db).await.unwrap_or_default() {
            keys.insert(format!("{show}/rule"));
        }
        crate::sync::export_touched(&s2, keys).await;
    });
    Ok(())
}

fn stamp(st: &AppState, fmt: &str) -> String {
    st.cfg.tz.timestamp_millis_opt(now_ms()).single().map(|d| d.format(fmt).to_string()).unwrap_or_default()
}

/// Write a backup of the current setup into the backups folder.
pub async fn write(st: &AppState, name: &str) -> anyhow::Result<PathBuf> {
    let b = export(&st.db, &st.cfg.logos_dir).await?;
    tokio::fs::create_dir_all(&st.cfg.backups_dir).await?;
    let path = st.cfg.backups_dir.join(name);
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, serde_json::to_vec_pretty(&b)?).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(path)
}

/// A backup file name RewindTV wrote: "rewindtv-….json", no paths.
pub fn name_ok(name: &str) -> bool {
    name.starts_with("rewindtv-")
        && name.ends_with(".json")
        && name.len() < 80
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !name.contains("..")
}

#[derive(Debug, Serialize)]
pub struct BackupFile {
    pub name: String,
    pub bytes: u64,
    pub modified: i64,
    pub automatic: bool,
}

pub async fn list(dir: &Path) -> Vec<BackupFile> {
    let mut out = Vec::new();
    if let Ok(mut rd) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(e)) = rd.next_entry().await {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name_ok(&name) {
                continue;
            }
            if let Ok(meta) = e.metadata().await {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_millis() as i64);
                out.push(BackupFile { automatic: !name.contains("before-restore"), name, bytes: meta.len(), modified });
            }
        }
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.modified));
    out
}

/// Keep the newest nightly and safety backups; remove older ones.
async fn prune(dir: &Path) {
    let all = list(dir).await;
    for (automatic, keep) in [(true, KEEP_NIGHTLY), (false, KEEP_SAFETY)] {
        for old in all.iter().filter(|b| b.automatic == automatic).skip(keep) {
            let _ = tokio::fs::remove_file(dir.join(&old.name)).await;
        }
    }
}

async fn nightly_once(st: &AppState) {
    let name = format!("rewindtv-{}.json", stamp(st, "%Y-%m-%d"));
    if st.cfg.backups_dir.join(&name).exists() {
        return;
    }
    match write(st, &name).await {
        Ok(p) => tracing::info!("nightly backup saved to {}", p.display()),
        Err(e) => tracing::warn!("nightly backup failed: {e:#}"),
    }
    prune(&st.cfg.backups_dir).await;
}

/// Back up at startup if today has none yet, then every night at 3:30.
pub async fn nightly(st: AppState) {
    nightly_once(&st).await;
    loop {
        let now = st.cfg.tz.timestamp_millis_opt(now_ms()).single();
        let wait = now
            .and_then(|n| {
                let today = n.date_naive().and_hms_opt(3, 30, 0)?;
                let next = if n.naive_local() < today { today } else { today + chrono::Duration::days(1) };
                let next = st.cfg.tz.from_local_datetime(&next).earliest()?;
                (next - n).to_std().ok()
            })
            .unwrap_or(std::time::Duration::from_secs(3600));
        tokio::time::sleep(wait).await;
        nightly_once(&st).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn mem() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(true);
        let pool = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn seed(db: &SqlitePool) {
        for sql in [
            "INSERT INTO users (id, name, color, is_admin, created_at) VALUES (1, 'Adam', '#f59e0b', 1, 1), (2, 'Kids', '#10b981', 0, 2)",
            "INSERT INTO sources (id, kind, name, url, token, enabled, created_at) VALUES (3, 'plex', 'Basement', 'http://nas:32400', 'secret', 1, 1)",
            "INSERT INTO libraries (id, source_id, key, name, kind, enabled) VALUES (7, 3, '1', 'TV', 'show', 1), (8, 3, '2', 'Movies', 'movie', 0)",
            "INSERT INTO channels (id, number, name, color, logo, owner_user_id, rule, ordering, block_size, avoid_repeat_hours, align_minutes, created_at, updated_at, skip_intros, skip_credits)
             VALUES (5, 40, 'Anime', '#ef4444', '5-00aa.png', NULL, '{\"genres\":[\"Anime\"],\"libraries\":[7]}', 'episodic', 3, 24, 30, 1, 1, 1, 1)",
            "INSERT INTO channel_slots (id, channel_id, title, days, start_min, end_min, rule, ordering) VALUES (9, 5, 'Saturday', 32, 420, 720, '{}', 'block')",
            "INSERT INTO user_channels (user_id, channel_id, hidden, favorite) VALUES (2, 5, 0, 1)",
            "INSERT INTO channel_state (channel_id, state) VALUES (5, '{\"pools\":{}}')",
            "INSERT INTO settings (key, value) VALUES ('markers.detect', '1')",
            "INSERT INTO marker_rules (show_key, season, kind, start_ms, end_ms, from_end, updated_at) VALUES ('tv/bebop', 1, 'intro', 5000, 95000, 0, 1)",
            "INSERT INTO markers (media_key, kind, source, start_ms, end_ms, duration_ms, confidence, updated_at) VALUES ('tv/bebop/s01e01', 'credits', 'detected', 1300000, 1390000, 1440000, 0.81, 1)",
            "INSERT INTO marker_ids (provider_id, media_key) VALUES ('tvdb:76885', 'tv/bebop/s01e01')",
        ] {
            sqlx::query(sql).execute(db).await.unwrap();
        }
    }

    #[tokio::test]
    async fn round_trip_into_a_fresh_install() {
        let a = mem().await;
        seed(&a).await;
        let dir = std::env::temp_dir();
        let backup = export(&a, &dir).await.unwrap();
        let json = serde_json::to_vec(&backup).unwrap();

        let b = mem().await;
        // The fresh install already has something that must be replaced.
        sqlx::query("INSERT INTO users (id, name, color, is_admin, created_at) VALUES (1, 'Old', '#000', 0, 0)").execute(&b).await.unwrap();
        restore_db(&b, &parse(&json).unwrap()).await.unwrap();
        let again = export(&b, &dir).await.unwrap();
        assert_eq!(backup.tables, again.tables, "everything comes back exactly");
        let (skip, logo): (bool, String) = sqlx::query_as("SELECT skip_credits, logo FROM channels WHERE id = 5").fetch_one(&b).await.unwrap();
        assert!(skip);
        assert_eq!(logo, "5-00aa.png");
    }

    #[tokio::test]
    async fn bad_backups_change_nothing() {
        let db = mem().await;
        seed(&db).await;
        assert!(parse(b"{\"hello\": 1}").is_err());
        assert!(parse(b"not json").is_err());
        // A row that breaks a constraint midway: the whole restore rolls back.
        let mut broken = export(&db, &std::env::temp_dir()).await.unwrap();
        broken.tables.get_mut("channels").unwrap()[0].insert("number".into(), Value::Null);
        assert!(restore_db(&db, &broken).await.is_err());
        let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(&db).await.unwrap();
        assert_eq!(users, 2, "the old setup is untouched");
    }

    #[test]
    fn backup_names_cant_escape_the_folder() {
        assert!(name_ok("rewindtv-2026-11-02.json"));
        assert!(name_ok("rewindtv-before-restore-2026-11-02-031500.json"));
        for bad in ["../rewindtv-x.json", "rewindtv-../../etc.json", "rewindtv-x.json/..", "other.json", "rewindtv-a b.json"] {
            assert!(!name_ok(bad), "{bad}");
        }
    }
}
