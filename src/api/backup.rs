//! Backup & restore endpoints.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use crate::backup::{self, BackupFile};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub const MAX_BACKUP_BYTES: usize = 50 * 1024 * 1024;

fn download(name: &str, body: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        body,
    )
        .into_response()
}

/// GET /api/backup — download a backup of the current setup.
pub async fn export(State(st): State<AppState>) -> ApiResult<Response> {
    let b = backup::export(&st.db, &st.cfg.logos_dir).await?;
    let date = chrono::Utc::now().with_timezone(&st.cfg.tz).format("%Y-%m-%d");
    Ok(download(&format!("rewindtv-{date}.json"), serde_json::to_vec_pretty(&b)?))
}

#[derive(Serialize)]
pub struct Restored {
    ok: bool,
}

async fn restore_bytes(st: &AppState, bytes: &[u8]) -> ApiResult<Json<Restored>> {
    let b = backup::parse(bytes).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    backup::restore(st, b).await.map_err(|e| ApiError::BadRequest(format!("Restore failed, nothing was changed: {e:#}")))?;
    Ok(Json(Restored { ok: true }))
}

/// POST /api/backup/restore — restore from an uploaded backup file.
pub async fn restore_upload(State(st): State<AppState>, body: Bytes) -> ApiResult<Json<Restored>> {
    restore_bytes(&st, &body).await
}

#[derive(Serialize)]
pub struct Listing {
    dir: String,
    files: Vec<BackupFile>,
}

/// GET /api/backups — automatic and safety backups in the backups folder.
pub async fn list(State(st): State<AppState>) -> Json<Listing> {
    Json(Listing { dir: st.cfg.backups_dir.display().to_string(), files: backup::list(&st.cfg.backups_dir).await })
}

async fn read_named(st: &AppState, name: &str) -> ApiResult<Vec<u8>> {
    // Only names that are actually in the folder listing: no paths.
    if !backup::name_ok(name) || !backup::list(&st.cfg.backups_dir).await.iter().any(|b| b.name == name) {
        return Err(ApiError::NotFound);
    }
    Ok(tokio::fs::read(st.cfg.backups_dir.join(name)).await?)
}

/// GET /api/backups/:name — download one of them.
pub async fn get_named(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Response> {
    let body = read_named(&st, &name).await?;
    Ok(download(&name, body))
}

/// POST /api/backups/:name/restore
pub async fn restore_named(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Json<Restored>> {
    let body = read_named(&st, &name).await?;
    restore_bytes(&st, &body).await
}
