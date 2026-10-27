// sqlx row tuples are clearer inline than as one-off type aliases.
#![allow(clippy::type_complexity)]

mod api;
mod config;
mod db;
mod error;
mod guide;
mod markers;
mod media;
mod schedule;
mod security;
mod state;
mod stream;
mod sync;
mod web;

use std::sync::Arc;
use std::time::Duration;

use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=warn")))
        .init();

    let cfg = Config::from_env()?;
    if std::env::args().nth(1).as_deref() == Some("health") {
        return health(cfg.port).await;
    }
    let db = db::open(&cfg.data_dir).await?;
    db::ensure_writable(&cfg.markers_dir)?;
    tracing::info!("database in {}, intro/credits markers in {}", cfg.data_dir.display(), cfg.markers_dir.display());
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(60))
        .build()?;
    let state = AppState {
        cfg: Arc::new(cfg.clone()),
        db,
        http,
        sessions: Default::default(),
        sync_status: Default::default(),
        detect_status: Default::default(),
        guide_lock: Default::default(),
        // Plex honours the advertised tuner count; leave headroom for other apps.
        items: Default::default(),
        tuner_slots: Arc::new(tokio::sync::Semaphore::new(cfg.tuners as usize + 2)),
    };

    if let Err(e) = markers::files::restore_sources(&state.db, &state.cfg.markers_dir).await {
        tracing::warn!("reading marker settings: {e:#}");
    }
    match markers::files::import_all(&state.db, &state.cfg.markers_dir).await {
        Ok(n) if n > 0 => tracing::info!("loaded {n} markers from {}", state.cfg.markers_dir.display()),
        Ok(_) => {}
        Err(e) => tracing::warn!("reading marker files: {e:#}"),
    }
    spawn_background(state.clone());

    let app = api::router()
        .fallback(web::static_handler)
        .layer(axum::middleware::from_fn_with_state(state.clone(), security::guard))
        // Gzip JSON, XMLTV and M3U; never live video, which must stream unbuffered.
        .layer(CompressionLayer::new().compress_when(DefaultPredicate::new().and(NotForContentType::const_new("video/"))))
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.port));
    tracing::info!("RewindTV listening on http://{addr} (timezone {})", cfg.tz);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).with_graceful_shutdown(shutdown()).await?;
    Ok(())
}

/// `rewindtv health` — used by the container HEALTHCHECK.
async fn health(port: u16) -> anyhow::Result<()> {
    reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/api/status"))
        .timeout(Duration::from_secs(4))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

fn spawn_background(state: AppState) {
    // Keep every guide filled a week ahead.
    let st = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let _guard = st.guide_lock.lock().await;
            let items = match st.items().await {
                Ok(i) => i,
                Err(e) => {
                    tracing::warn!("loading library: {e:#}");
                    continue;
                }
            };
            if let Err(e) = schedule::store::extend_all(&st.db, &items, st.cfg.tz, db::now_ms()).await {
                tracing::warn!("guide refresh: {e:#}");
            }
        }
    });

    // Periodic library sync (the first run happens shortly after startup).
    let st = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        let mut tick = tokio::time::interval(Duration::from_secs(st.cfg.sync_hours.max(1) * 3600));
        loop {
            tick.tick().await;
            sync::sync_all(&st).await;
        }
    });

    tokio::spawn(stream::hdhr::serve(state.clone()));

    // Stop transcodes nobody is watching any more.
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        loop {
            tick.tick().await;
            state.sessions.reap().await;
        }
    });
}

async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
        tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    ctrl_c.await.ok();
}
