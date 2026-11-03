//! Phone as a remote: a TV page holds a 4-digit code and listens for commands
//! over server-sent events; a phone on the same network posts commands to the
//! code. Purely in memory: a code lives as long as a TV is listening on it.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use axum::extract::Path;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures::{Stream, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

use crate::error::{ApiError, ApiResult};

static RELAY: LazyLock<Mutex<HashMap<String, broadcast::Sender<String>>>> = LazyLock::new(Default::default);

fn valid_code(code: &str) -> bool {
    code.len() == 4 && code.bytes().all(|b| b.is_ascii_digit())
}

/// Forget codes no TV is listening on any more.
fn prune(map: &mut HashMap<String, broadcast::Sender<String>>) {
    map.retain(|_, tx| tx.receiver_count() > 0);
}

#[derive(Deserialize, Default)]
pub struct PairIn {
    /// The code this TV had before (kept across page reloads), if any.
    code: Option<String>,
}

#[derive(Serialize)]
pub struct PairOut {
    code: String,
}

/// POST /api/remote — a code for this TV: the one it had before, or a new one.
pub async fn pair(body: Option<Json<PairIn>>) -> ApiResult<Json<PairOut>> {
    let wanted = body.and_then(|b| b.0.code).filter(|c| valid_code(c));
    let mut map = RELAY.lock().expect("relay lock");
    prune(&mut map);
    // Reloads keep their code, even while the old page's feed is still closing.
    if let Some(c) = wanted {
        map.entry(c.clone()).or_insert_with(|| broadcast::channel(16).0);
        return Ok(Json(PairOut { code: c }));
    }
    if map.len() > 5000 {
        return Err(ApiError::BadRequest("Too many TVs".into()));
    }
    let mut rng = rand::thread_rng();
    loop {
        let code = format!("{:04}", rng.gen_range(1000..10000));
        if !map.contains_key(&code) {
            map.insert(code.clone(), broadcast::channel(16).0);
            return Ok(Json(PairOut { code }));
        }
    }
}

/// GET /api/remote/:code/events — the TV's command feed.
pub async fn events(Path(code): Path<String>) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    if !valid_code(&code) {
        return Err(ApiError::NotFound);
    }
    let rx = {
        let mut map = RELAY.lock().expect("relay lock");
        map.entry(code).or_insert_with(|| broadcast::channel(16).0).subscribe()
    };
    let stream = BroadcastStream::new(rx).filter_map(|m| async move { m.ok().map(|data| Ok(Event::default().data(data))) });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

#[derive(Deserialize, Serialize, Debug, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    Up,
    Down,
    Number { number: u32 },
    Tune { channel_id: i64 },
    Last,
    Mute,
    Info,
    Guide,
    StartOver,
}

#[derive(Serialize)]
pub struct Status {
    connected: bool,
}

/// GET /api/remote/:code — is a TV listening on this code?
pub async fn status(Path(code): Path<String>) -> Json<Status> {
    let map = RELAY.lock().expect("relay lock");
    Json(Status { connected: map.get(&code).is_some_and(|tx| tx.receiver_count() > 0) })
}

/// POST /api/remote/:code/command
pub async fn command(Path(code): Path<String>, Json(cmd): Json<Command>) -> ApiResult<Json<Status>> {
    let tx = RELAY.lock().expect("relay lock").get(&code).cloned();
    let sent = match tx {
        Some(tx) => tx.send(serde_json::to_string(&cmd).map_err(|e| ApiError::Internal(e.into()))?).is_ok(),
        None => false,
    };
    if !sent {
        return Err(ApiError::BadRequest("That TV isn't listening any more — check the code on screen".into()));
    }
    Ok(Json(Status { connected: true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn relays_commands_to_the_listening_tv() {
        let Json(PairOut { code }) = pair(None).await.unwrap();
        assert!(valid_code(&code));
        // Nobody listening yet: the phone hears so.
        assert!(command(Path(code.clone()), Json(Command::Up)).await.is_err());

        let rx = RELAY.lock().unwrap().entry(code.clone()).or_insert_with(|| broadcast::channel(16).0).subscribe();
        let mut feed = BroadcastStream::new(rx);
        assert!(status(Path(code.clone())).await.connected);
        let _ = command(Path(code.clone()), Json(Command::Number { number: 42 })).await.unwrap();
        let got = feed.next().await.unwrap().unwrap();
        assert_eq!(serde_json::from_str::<Command>(&got).unwrap(), Command::Number { number: 42 });

        // The TV keeps its code across a reload.
        drop(feed);
        let Json(again) = pair(Some(Json(PairIn { code: Some(code.clone()) }))).await.unwrap();
        assert_eq!(again.code, code);
        assert!(!valid_code("12a4") && !valid_code("123") && !valid_code("../x"));
    }
}
