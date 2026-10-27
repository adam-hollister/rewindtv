use std::path::Path;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

pub async fn open(dir: &Path) -> anyhow::Result<SqlitePool> {
    std::fs::create_dir_all(dir).map_err(|e| writable_hint(dir, e))?;
    // Fail early with a clear message rather than a cryptic SQLite error.
    let probe = dir.join(".write-test");
    std::fs::write(&probe, b"ok").map_err(|e| writable_hint(dir, e))?;
    let _ = std::fs::remove_file(probe);
    let path = dir.join("rewindtv.db");
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // Durable across crashes with WAL, without an fsync on every commit.
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(10));
    let pool = SqlitePoolOptions::new().max_connections(8).connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

fn writable_hint(dir: &Path, e: std::io::Error) -> anyhow::Error {
    anyhow::anyhow!(
        "can't write to {} ({e}). The container runs as uid/gid 568 (TrueNAS's \"apps\" user): \
         create the dataset with the Apps permission preset, or chown it to 568:568.",
        dir.display()
    )
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
