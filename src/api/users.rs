use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::now_ms;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Serialize, sqlx::FromRow)]
pub struct User {
    id: i64,
    name: String,
    color: String,
    is_admin: bool,
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Json<Vec<User>>> {
    Ok(Json(sqlx::query_as("SELECT id, name, color, is_admin FROM users ORDER BY id").fetch_all(&st.db).await?))
}

#[derive(Deserialize)]
pub struct UserInput {
    name: Option<String>,
    color: Option<String>,
    is_admin: Option<bool>,
}

pub async fn create(State(st): State<AppState>, Json(u): Json<UserInput>) -> ApiResult<Json<User>> {
    let name = u.name.filter(|n| !n.trim().is_empty()).ok_or_else(|| ApiError::BadRequest("name required".into()))?;
    let first: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(&st.db).await?;
    let user = sqlx::query_as(
        "INSERT INTO users (name, color, is_admin, created_at) VALUES (?, ?, ?, ?) RETURNING id, name, color, is_admin",
    )
    .bind(name.trim())
    .bind(u.color.unwrap_or_else(|| "#f59e0b".into()))
    .bind(u.is_admin.unwrap_or(first == 0))
    .bind(now_ms())
    .fetch_one(&st.db)
    .await?;
    Ok(Json(user))
}

pub async fn update(State(st): State<AppState>, Path(id): Path<i64>, Json(u): Json<UserInput>) -> ApiResult<Json<User>> {
    let user = sqlx::query_as(
        "UPDATE users SET name = COALESCE(?, name), color = COALESCE(?, color), is_admin = COALESCE(?, is_admin)
         WHERE id = ? RETURNING id, name, color, is_admin",
    )
    .bind(u.name)
    .bind(u.color)
    .bind(u.is_admin)
    .bind(id)
    .fetch_optional(&st.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(user))
}

pub async fn remove(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<()>> {
    sqlx::query("DELETE FROM users WHERE id = ?").bind(id).execute(&st.db).await?;
    Ok(Json(()))
}

#[derive(Deserialize)]
pub struct Prefs {
    hidden: Option<bool>,
    favorite: Option<bool>,
}

pub async fn set_channel_prefs(State(st): State<AppState>, Path((uid, cid)): Path<(i64, i64)>, Json(p): Json<Prefs>) -> ApiResult<Json<()>> {
    sqlx::query(
        "INSERT INTO user_channels (user_id, channel_id, hidden, favorite) VALUES (?, ?, COALESCE(?, 0), COALESCE(?, 0))
         ON CONFLICT(user_id, channel_id) DO UPDATE SET
            hidden = COALESCE(?, hidden), favorite = COALESCE(?, favorite)",
    )
    .bind(uid)
    .bind(cid)
    .bind(p.hidden)
    .bind(p.favorite)
    .bind(p.hidden)
    .bind(p.favorite)
    .execute(&st.db)
    .await?;
    Ok(Json(()))
}
