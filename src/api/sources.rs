use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::now_ms;
use crate::error::{ApiError, ApiResult};
use crate::media::Source;
use crate::state::{AppState, SyncStatus};
use crate::sync;

#[derive(Serialize, sqlx::FromRow)]
pub struct LibraryRow {
    id: i64,
    source_id: i64,
    name: String,
    kind: String,
    enabled: bool,
    items: i64,
}

#[derive(Serialize)]
pub struct SourceOut {
    id: i64,
    kind: String,
    name: String,
    url: String,
    enabled: bool,
    last_sync: Option<i64>,
    last_error: Option<String>,
    libraries: Vec<LibraryRow>,
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Json<Vec<SourceOut>>> {
    let rows: Vec<(i64, String, String, String, bool, Option<i64>, Option<String>)> =
        sqlx::query_as("SELECT id, kind, name, url, enabled, last_sync, last_error FROM sources ORDER BY id").fetch_all(&st.db).await?;
    let libs: Vec<LibraryRow> = sqlx::query_as(
        "SELECT l.id, l.source_id, l.name, l.kind, l.enabled,
                (SELECT COUNT(*) FROM media_items m WHERE m.library_id = l.id AND m.missing = 0) AS items
         FROM libraries l ORDER BY l.name",
    )
    .fetch_all(&st.db)
    .await?;
    let mut out: Vec<SourceOut> = rows
        .into_iter()
        .map(|(id, kind, name, url, enabled, last_sync, last_error)| SourceOut { id, kind, name, url, enabled, last_sync, last_error, libraries: vec![] })
        .collect();
    for l in libs {
        if let Some(s) = out.iter_mut().find(|s| s.id == l.source_id) {
            s.libraries.push(l);
        }
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct SourceInput {
    kind: Option<String>,
    name: Option<String>,
    url: Option<String>,
    token: Option<String>,
    enabled: Option<bool>,
}

fn normalize_url(u: &str) -> String {
    let u = u.trim().trim_end_matches('/');
    if u.starts_with("http://") || u.starts_with("https://") {
        u.to_string()
    } else {
        format!("http://{u}")
    }
}

#[derive(Serialize)]
pub struct TestResult {
    ok: bool,
    name: Option<String>,
    error: Option<String>,
}

pub async fn test(State(st): State<AppState>, Json(s): Json<SourceInput>) -> ApiResult<Json<TestResult>> {
    let (Some(kind), Some(url), Some(token)) = (s.kind, s.url, s.token) else {
        return Err(ApiError::BadRequest("kind, url and token are required".into()));
    };
    let src = Source::new(st.http.clone(), &kind, &normalize_url(&url), token.trim()).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(match src.test().await {
        Ok(name) => TestResult { ok: true, name: Some(name), error: None },
        Err(e) => TestResult { ok: false, name: None, error: Some(format!("{e:#}")) },
    }))
}

pub async fn create(State(st): State<AppState>, Json(s): Json<SourceInput>) -> ApiResult<Json<serde_json::Value>> {
    let (Some(kind), Some(url), Some(token)) = (s.kind, s.url, s.token) else {
        return Err(ApiError::BadRequest("kind, url and token are required".into()));
    };
    let url = normalize_url(&url);
    let src = Source::new(st.http.clone(), &kind, &url, token.trim()).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let server_name = src.test().await.map_err(|e| ApiError::BadRequest(format!("Could not connect: {e:#}")))?;
    let name = s.name.filter(|n| !n.trim().is_empty()).unwrap_or(server_name);
    let id: i64 = sqlx::query_scalar("INSERT INTO sources (kind, name, url, token, created_at) VALUES (?, ?, ?, ?, ?) RETURNING id")
        .bind(&kind)
        .bind(&name)
        .bind(&url)
        .bind(token.trim())
        .bind(now_ms())
        .fetch_one(&st.db)
        .await?;
    sync::refresh_libraries(&st.db, id, &src).await?;
    st.invalidate_items().await;
    Ok(Json(serde_json::json!({ "id": id, "name": name })))
}

pub async fn update(State(st): State<AppState>, Path(id): Path<i64>, Json(s): Json<SourceInput>) -> ApiResult<Json<()>> {
    sqlx::query(
        "UPDATE sources SET name = COALESCE(?, name), url = COALESCE(?, url), token = COALESCE(?, token),
            enabled = COALESCE(?, enabled) WHERE id = ?",
    )
    .bind(s.name)
    .bind(s.url.map(|u| normalize_url(&u)))
    .bind(s.token.filter(|t| !t.trim().is_empty()))
    .bind(s.enabled)
    .bind(id)
    .execute(&st.db)
    .await?;
    st.invalidate_items().await;
    Ok(Json(()))
}

pub async fn remove(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<()>> {
    sqlx::query("DELETE FROM sources WHERE id = ?").bind(id).execute(&st.db).await?;
    st.invalidate_items().await;
    Ok(Json(()))
}

#[derive(Deserialize)]
pub struct LibraryInput {
    enabled: bool,
}

pub async fn set_library(State(st): State<AppState>, Path(id): Path<i64>, Json(l): Json<LibraryInput>) -> ApiResult<Json<()>> {
    sqlx::query("UPDATE libraries SET enabled = ? WHERE id = ?").bind(l.enabled).bind(id).execute(&st.db).await?;
    st.invalidate_items().await;
    Ok(Json(()))
}

pub async fn sync_status(State(st): State<AppState>) -> Json<SyncStatus> {
    Json(st.sync_status.read().await.clone())
}

pub async fn sync_now(State(st): State<AppState>) -> Json<SyncStatus> {
    let s2 = st.clone();
    tokio::spawn(async move { sync::sync_all(&s2).await });
    // Report as running right away so the UI can start polling.
    let mut status = st.sync_status.read().await.clone();
    status.running = true;
    Json(status)
}
