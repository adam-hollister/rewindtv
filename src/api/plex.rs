//! "Sign in with Plex": link this install via plex.tv/link and pick a server,
//! instead of digging a token out of Plex Web.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use reqwest::header::ACCEPT;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ApiError, ApiResult};
use crate::media::Source;
use crate::state::AppState;

const PLEX_TV: &str = "https://plex.tv";

/// Stable per-install client id that plex.tv shows in "Authorized Devices".
async fn client_id(st: &AppState) -> ApiResult<String> {
    if let Some(id) = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = 'plex_client_id'")
        .fetch_optional(&st.db)
        .await?
    {
        return Ok(id);
    }
    let id = format!("rewindtv-{}", uuid::Uuid::new_v4().simple());
    sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES ('plex_client_id', ?)").bind(&id).execute(&st.db).await?;
    Ok(id)
}

async fn plex_tv(st: &AppState, req: reqwest::RequestBuilder) -> ApiResult<Value> {
    let res = req
        .header(ACCEPT, "application/json")
        .header("X-Plex-Product", "RewindTV")
        .header("X-Plex-Client-Identifier", client_id(st).await?)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("Could not reach plex.tv: {e}")))?;
    if !res.status().is_success() {
        return Err(ApiError::BadGateway(format!("plex.tv answered {}", res.status())));
    }
    res.json().await.map_err(|e| ApiError::BadGateway(e.to_string()))
}

#[derive(Serialize)]
pub struct Pin {
    id: i64,
    code: String,
    link_url: &'static str,
}

/// POST /api/plex/pin — start linking; the user enters `code` at plex.tv/link.
pub async fn create_pin(State(st): State<AppState>) -> ApiResult<Json<Pin>> {
    let v = plex_tv(&st, st.http.post(format!("{PLEX_TV}/api/v2/pins?strong=false"))).await?;
    let id = v.get("id").and_then(Value::as_i64).ok_or_else(|| ApiError::BadGateway("plex.tv sent no PIN".into()))?;
    let code = v.get("code").and_then(Value::as_str).unwrap_or_default().to_string();
    Ok(Json(Pin { id, code, link_url: "https://plex.tv/link" }))
}

#[derive(Serialize)]
pub struct PinStatus {
    token: Option<String>,
}

/// GET /api/plex/pin/:id — poll until the user has entered the code.
pub async fn check_pin(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<PinStatus>> {
    let v = plex_tv(&st, st.http.get(format!("{PLEX_TV}/api/v2/pins/{id}"))).await?;
    let token = v.get("authToken").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string);
    Ok(Json(PinStatus { token }))
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Connection {
    pub uri: String,
    pub local: bool,
    pub reachable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Server {
    pub name: String,
    pub owned: bool,
    pub token: String,
    pub connections: Vec<Connection>,
    /// The first reachable connection, local ones first.
    pub best: Option<String>,
}

/// Servers in a plex.tv resources response, with candidate addresses ordered
/// best-first: local before remote, never relays. For local connections the
/// plain http://ip:port form is tried too, since *.plex.direct names need DNS
/// rebinding to resolve on some home routers.
pub fn parse_resources(v: &Value) -> Vec<Server> {
    let empty = Vec::new();
    v.as_array()
        .unwrap_or(&empty)
        .iter()
        .filter(|r| r.get("provides").and_then(Value::as_str).is_some_and(|p| p.split(',').any(|x| x == "server")))
        .filter_map(|r| {
            let token = r.get("accessToken").and_then(Value::as_str)?.to_string();
            let mut conns: Vec<Connection> = Vec::new();
            let mut add = |uri: String, local: bool| {
                if !conns.iter().any(|c| c.uri == uri) {
                    conns.push(Connection { uri, local, reachable: false });
                }
            };
            let list = r.get("connections").and_then(Value::as_array).cloned().unwrap_or_default();
            for local_pass in [true, false] {
                for c in &list {
                    let local = c.get("local").and_then(Value::as_bool).unwrap_or(false);
                    let relay = c.get("relay").and_then(Value::as_bool).unwrap_or(false);
                    if relay || local != local_pass {
                        continue;
                    }
                    if let (true, Some(addr), Some(port)) =
                        (local, c.get("address").and_then(Value::as_str), c.get("port").and_then(Value::as_i64))
                    {
                        add(format!("http://{addr}:{port}"), true);
                    }
                    if let Some(uri) = c.get("uri").and_then(Value::as_str) {
                        add(uri.trim_end_matches('/').to_string(), local);
                    }
                }
            }
            Some(Server {
                name: r.get("name").and_then(Value::as_str).unwrap_or("Plex").to_string(),
                owned: r.get("owned").and_then(Value::as_bool).unwrap_or(false),
                token,
                connections: conns,
                best: None,
            })
        })
        .collect()
}

#[derive(Deserialize)]
pub struct TokenQuery {
    token: String,
}

/// POST /api/plex/servers {token} — your Plex servers, each address tested
/// from here. (POST so the token never lands in URLs or access logs.)
pub async fn servers(State(st): State<AppState>, Json(q): Json<TokenQuery>) -> ApiResult<Json<Vec<Server>>> {
    let v = plex_tv(
        &st,
        st.http
            .get(format!("{PLEX_TV}/api/v2/resources?includeHttps=1&includeRelay=0&includeIPv6=0"))
            .header("X-Plex-Token", &q.token),
    )
    .await?;
    let mut list = parse_resources(&v);
    for server in &mut list {
        let checks = server.connections.iter().map(|c| {
            let src = Source::new(st.http.clone(), "plex", &c.uri, &server.token);
            async move {
                match src {
                    Ok(s) => tokio::time::timeout(Duration::from_secs(4), s.test()).await.is_ok_and(|r| r.is_ok()),
                    Err(_) => false,
                }
            }
        });
        let results = futures::future::join_all(checks).await;
        for (c, ok) in server.connections.iter_mut().zip(results) {
            c.reachable = ok;
        }
        server.best = server.connections.iter().find(|c| c.reachable).map(|c| c.uri.clone());
    }
    // Owned, reachable servers first.
    list.sort_by_key(|s| (!s.owned, s.best.is_none()));
    Ok(Json(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_servers_and_orders_local_connections_first() {
        let v = serde_json::json!([
            {
                "name": "Basement", "provides": "server", "owned": true, "accessToken": "tok",
                "connections": [
                    {"protocol": "https", "address": "203.0.113.5", "port": 32400, "uri": "https://203-0-113-5.abc.plex.direct:32400", "local": false, "relay": false},
                    {"protocol": "https", "address": "10.0.0.9", "port": 32400, "uri": "https://10-0-0-9.abc.plex.direct:32400", "local": true, "relay": false},
                    {"protocol": "https", "address": "1.2.3.4", "port": 8443, "uri": "https://relay.plex.direct:8443", "local": false, "relay": true}
                ]
            },
            {"name": "My iPhone", "provides": "client,player", "accessToken": "x", "connections": []}
        ]);
        let servers = parse_resources(&v);
        assert_eq!(servers.len(), 1);
        let uris: Vec<&str> = servers[0].connections.iter().map(|c| c.uri.as_str()).collect();
        assert_eq!(
            uris,
            vec!["http://10.0.0.9:32400", "https://10-0-0-9.abc.plex.direct:32400", "https://203-0-113-5.abc.plex.direct:32400"]
        );
        assert_eq!(servers[0].token, "tok");
        assert!(servers[0].owned);
    }
}
