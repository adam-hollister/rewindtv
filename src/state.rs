use std::sync::Arc;

use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::{Mutex, RwLock};

use crate::config::Config;
use crate::error::{ApiError, ApiResult};
use crate::media::Source;
use crate::schedule::rule::Item;
use crate::schedule::store;
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
    /// Caps simultaneous Live TV / IPTV streams (each runs ffmpeg and a transcode).
    pub tuner_slots: Arc<tokio::sync::Semaphore>,
    /// The playable library with resolved markers, shared by every request
    /// and rebuilt only after something changes (sync, settings, markers).
    pub items: Arc<RwLock<Option<Arc<Vec<Item>>>>>,
}

impl AppState {
    pub async fn items(&self) -> anyhow::Result<Arc<Vec<Item>>> {
        if let Some(items) = self.items.read().await.as_ref() {
            return Ok(items.clone());
        }
        let mut slot = self.items.write().await;
        if let Some(items) = slot.as_ref() {
            return Ok(items.clone()); // loaded while we waited
        }
        let loaded = Arc::new(store::load_items(&self.db).await?);
        *slot = Some(loaded.clone());
        Ok(loaded)
    }

    /// Call after anything that changes the library, its markers or settings.
    pub async fn invalidate_items(&self) {
        *self.items.write().await = None;
    }

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
