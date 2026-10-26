mod api;
mod config;
mod db;
mod error;
mod guide;
mod media;
mod schedule;
mod state;
mod stream;
mod sync;
mod web;

use std::sync::Arc;
use std::time::Duration;

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
        guide_lock: Default::default(),
    };

    spawn_background(state.clone());

    let app = api::router()
        .fallback(web::static_handler)
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
            if let Err(e) = schedule::store::extend_all(&st.db, st.cfg.tz, db::now_ms()).await {
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
