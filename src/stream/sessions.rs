//! Live browser playback sessions. Each tune opens one transcode on the media
//! server; sessions that stop fetching segments are torn down so channel
//! surfing never piles up transcodes.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use url::Url;

use crate::media::Source;

const IDLE: Duration = Duration::from_secs(45);

#[derive(Clone)]
pub struct Session {
    pub source: Source,
    pub master: Url,
    pub last_seen: Instant,
}

#[derive(Clone, Default)]
pub struct Sessions(Arc<Mutex<HashMap<String, Session>>>);

impl Sessions {
    pub async fn insert(&self, id: String, source: Source, master: Url) {
        self.0.lock().await.insert(id, Session { source, master, last_seen: Instant::now() });
    }

    /// Looks up a session and marks it as active.
    pub async fn touch(&self, id: &str) -> Option<Session> {
        let mut map = self.0.lock().await;
        let s = map.get_mut(id)?;
        s.last_seen = Instant::now();
        Some(s.clone())
    }

    pub async fn stop(&self, id: &str) {
        let removed = self.0.lock().await.remove(id);
        if let Some(s) = removed {
            s.source.stop(id).await;
        }
    }

    pub async fn reap(&self) {
        let idle: Vec<(String, Session)> = {
            let mut map = self.0.lock().await;
            let ids: Vec<String> = map.iter().filter(|(_, s)| s.last_seen.elapsed() > IDLE).map(|(k, _)| k.clone()).collect();
            ids.into_iter().filter_map(|k| map.remove(&k).map(|s| (k, s))).collect()
        };
        for (id, s) in idle {
            tracing::debug!("reaping idle session {id}");
            s.source.stop(&id).await;
        }
    }

    pub async fn len(&self) -> usize {
        self.0.lock().await.len()
    }
}
