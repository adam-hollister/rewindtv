//! Lightweight protection for an app with no login, meant for a home network.
//!
//! - Host check (DNS rebinding): only answer requests addressed to an IP,
//!   `localhost`, a single-label or local-domain name (`truenas`, `nas.local`,
//!   `*.lan`, `*.home.arpa`…), or a host listed in `REWINDTV_PUBLIC_URL` /
//!   `REWINDTV_ALLOWED_HOSTS`. A public website can then never drive the API
//!   through a visitor's browser by pointing its own domain at a LAN address.
//! - Origin check (CSRF): requests that change something are refused when a
//!   browser says they came from another site.

use std::net::IpAddr;

use axum::extract::{Request, State};
use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

const LOCAL_SUFFIXES: &[&str] = &[".local", ".lan", ".home", ".internal", ".localdomain", ".home.arpa", ".localhost"];

/// Host part of a Host/Origin value, lowercased, without port or brackets.
fn host_only(v: &str) -> String {
    let v = v.trim().to_ascii_lowercase();
    let v = v.split("://").last().unwrap_or(&v).split('/').next().unwrap_or("").to_string();
    if let Some(rest) = v.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_string();
    }
    match v.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h.to_string(),
        _ => v,
    }
}

pub fn host_allowed(host: &str, extra: &[String]) -> bool {
    let h = host_only(host);
    if h.is_empty() {
        return true; // HTTP/1.0 clients without Host (some IPTV boxes)
    }
    h.parse::<IpAddr>().is_ok()
        || h == "localhost"
        || !h.contains('.')
        || LOCAL_SUFFIXES.iter().any(|s| h.ends_with(s))
        || extra.iter().any(|e| host_only(e) == h)
}

pub async fn guard(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !host_allowed(host, &st.cfg.allowed_hosts) {
        let msg = format!(
            "RewindTV doesn't answer to the host name \"{}\". If that's how you reach it, add it to REWINDTV_ALLOWED_HOSTS (or REWINDTV_PUBLIC_URL).",
            host_only(host)
        );
        return (StatusCode::FORBIDDEN, msg).into_response();
    }
    let changes_state = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if changes_state {
        if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            if origin != "null" && host_only(origin) != host_only(host) {
                return (StatusCode::FORBIDDEN, "Cross-site request refused").into_response();
            }
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        let none: Vec<String> = vec![];
        for ok in ["192.168.1.20:8420", "[fe80::1]:8420", "localhost:8420", "truenas:8420", "nas.local", "tv.home.arpa", "box.lan:8420", ""] {
            assert!(host_allowed(ok, &none), "{ok}");
        }
        for bad in ["evil.com", "192.168.1.20.nip.io:8420", "rewindtv.example.org"] {
            assert!(!host_allowed(bad, &none), "{bad}");
        }
        assert!(host_allowed("tv.example.org:8420", &["http://tv.example.org:8420".to_string()]));
    }

    #[test]
    fn host_parsing() {
        assert_eq!(host_only("http://NAS.local:8420"), "nas.local");
        assert_eq!(host_only("[::1]:8420"), "::1");
        assert_eq!(host_only("https://a.b/c"), "a.b");
    }
}
