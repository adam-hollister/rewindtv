use std::sync::Arc;

use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::{Mutex, RwLock};

use crate::config::Config;
use crate::error::{ApiError, ApiResult};
use crate::media::Source;
use crate::stream::sessions::Sessions;

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncStatus {
    pub running: bool,
    pub message: Option<String>,
    pub finished_at: Option<i64>,
}

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub db: SqlitePool,
    pub http: reqwest::Client,
    pub sessions: Sessions,
    pub sync_status: Arc<RwLock<SyncStatus>>,
    pub detect_status: Arc<RwLock<crate::markers::detect::Status>>,
    /// Serializes guide generation so concurrent edits don't interleave programs.
    pub guide_lock: Arc<Mutex<()>>,
}

impl AppState {
    pub async fn source(&self, id: i64) -> ApiResult<Source> {
        let (kind, url, token): (String, String, String) =
            sqlx::query_as("SELECT kind, url, token FROM sources WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.db)
                .await?
                .ok_or(ApiError::NotFound)?;
        Ok(Source::new(self.http.clone(), &kind, &url, &token)?)
    }
}
