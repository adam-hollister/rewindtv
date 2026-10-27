//! Intro/credits markers. Markers belong to what the media *is* (a
//! `media_key` such as `tv/cowboy-bebop/s01e05`, plus provider ids), not to a
//! server's item id, so markers set once apply on every server that syncs the
//! same episode. They are mirrored to JSON files in the markers folder
//! (`files`), which survive reinstalls and can be shared.

pub mod detect;
pub mod files;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::db::now_ms;
use crate::schedule::cuts::{self, Kind, Marker, Source, Sources};

// --- Identity ---------------------------------------------------------------------

/// Lowercase alphanumerics joined by dashes, without a leading "the".
pub fn norm(s: &str) -> String {
    let mut out = String::new();
    for word in s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        if out.is_empty() && word == "the" {
            continue;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(word);
    }
    out
}

pub fn show_key(show_title: &str) -> String {
    format!("tv/{}", norm(show_title))
}

/// Server-independent identity of an episode or movie.
pub fn media_key(kind: &str, title: &str, show_title: Option<&str>, season: Option<i64>, episode: Option<i64>, year: Option<i64>) -> Option<String> {
    match kind {
        "episode" => Some(format!("{}/s{:02}e{:02}", show_key(show_title?), season?, episode?)),
        "movie" => Some(format!("movie/{}/{}", norm(title), year.unwrap_or(0))),
        _ => None,
    }
}

// --- Settings ---------------------------------------------------------------------

pub async fn sources(db: &SqlitePool) -> Sources {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings WHERE key LIKE 'markers.%'")
        .fetch_all(db)
        .await
        .unwrap_or_default();
    let mut s = Sources::default();
    for (k, v) in rows {
        let on = v == "1";
        match k.as_str() {
            "markers.server" => s.server = on,
            "markers.manual" => s.manual = on,
            "markers.detect" => s.detect = on,
            _ => {}
        }
    }
    s
}

pub async fn set_sources(db: &SqlitePool, s: Sources) -> sqlx::Result<()> {
    for (k, v) in [("markers.server", s.server), ("markers.manual", s.manual), ("markers.detect", s.detect)] {
        sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(k)
            .bind(if v { "1" } else { "0" })
            .execute(db)
            .await?;
    }
    Ok(())
}

// --- Stored markers and rules -------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct StoredMarker {
    pub kind: String,
    pub source: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub duration_ms: i64,
    pub confidence: Option<f64>,
    pub updated_at: i64,
}

/// Credits sit relative to the end, so when a server's copy is a little
/// longer or shorter than the file the marker was measured on, shift them.
/// A very different length (another cut of the film) isn't trusted.
pub fn adjust(m: &StoredMarker, item_duration_ms: i64) -> Option<Marker> {
    let kind = Kind::parse(&m.kind)?;
    let source = Source::parse(&m.source)?;
    let diff = item_duration_ms - m.duration_ms;
    if m.duration_ms > 0 && diff.abs() as f64 > m.duration_ms as f64 * 0.05 + 3_000.0 {
        return None;
    }
    let shift = if kind == Kind::Credits && diff.abs() > 1_000 { diff } else { 0 };
    Some(Marker { kind, start_ms: m.start_ms + shift, end_ms: m.end_ms + shift, source })
}

/// A manual season/show rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub show_key: String,
    /// None = every season.
    pub season: Option<i64>,
    pub kind: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// Count back from the end of each episode ("credits = last 1:30":
    /// start_ms = 90000, end_ms = 0).
    pub from_end: bool,
    #[serde(default)]
    pub updated_at: i64,
}

impl Rule {
    pub fn apply(&self, duration_ms: i64) -> Option<Marker> {
        let kind = Kind::parse(&self.kind)?;
        let (start_ms, end_ms) = if self.from_end { (duration_ms - self.start_ms, duration_ms - self.end_ms) } else { (self.start_ms, self.end_ms) };
        Some(Marker { kind, start_ms, end_ms, source: Source::Rule })
    }
}

pub async fn rules(db: &SqlitePool, show: Option<&str>) -> sqlx::Result<Vec<Rule>> {
    let rows: Vec<(String, i64, String, i64, i64, bool, i64)> = sqlx::query_as(
        "SELECT show_key, season, kind, start_ms, end_ms, from_end, updated_at FROM marker_rules
         WHERE ?1 IS NULL OR show_key = ?1 ORDER BY show_key, season, kind",
    )
    .bind(show)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(show_key, season, kind, start_ms, end_ms, from_end, updated_at)| Rule {
            show_key,
            season: (season >= 0).then_some(season),
            kind,
            start_ms,
            end_ms,
            from_end,
            updated_at,
        })
        .collect())
}

pub async fn save_rule(db: &SqlitePool, r: &Rule) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO marker_rules (show_key, season, kind, start_ms, end_ms, from_end, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(show_key, season, kind) DO UPDATE SET start_ms = excluded.start_ms, end_ms = excluded.end_ms,
            from_end = excluded.from_end, updated_at = excluded.updated_at",
    )
    .bind(&r.show_key)
    .bind(r.season.unwrap_or(-1))
    .bind(&r.kind)
    .bind(r.start_ms)
    .bind(r.end_ms)
    .bind(r.from_end)
    .bind(if r.updated_at > 0 { r.updated_at } else { now_ms() })
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_rule(db: &SqlitePool, show_key: &str, season: Option<i64>, kind: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM marker_rules WHERE show_key = ? AND season = ? AND kind = ?")
        .bind(show_key)
        .bind(season.unwrap_or(-1))
        .bind(kind)
        .execute(db)
        .await?;
    Ok(())
}

/// Replace one source's markers for a media key. `None` for a kind deletes it.
pub async fn save(db: &SqlitePool, media_key: &str, source: Source, duration_ms: i64, markers: &[(Kind, i64, i64, Option<f64>)]) -> sqlx::Result<()> {
    let now = now_ms();
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM markers WHERE media_key = ? AND source = ?").bind(media_key).bind(source.as_str()).execute(&mut *tx).await?;
    for (kind, s, e, conf) in markers {
        sqlx::query(
            "INSERT OR REPLACE INTO markers (media_key, kind, source, start_ms, end_ms, duration_ms, confidence, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(media_key)
        .bind(kind.as_str())
        .bind(source.as_str())
        .bind(s)
        .bind(e)
        .bind(duration_ms)
        .bind(conf)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Set (or with `None`, clear) one kind of marker from one source.
pub async fn set_one(db: &SqlitePool, media_key: &str, kind: Kind, source: Source, range: Option<(i64, i64)>, duration_ms: i64) -> sqlx::Result<()> {
    match range {
        Some((s, e)) => {
            sqlx::query(
                "INSERT OR REPLACE INTO markers (media_key, kind, source, start_ms, end_ms, duration_ms, confidence, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, NULL, ?)",
            )
            .bind(media_key)
            .bind(kind.as_str())
            .bind(source.as_str())
            .bind(s)
            .bind(e)
            .bind(duration_ms)
            .bind(now_ms())
            .execute(db)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM markers WHERE media_key = ? AND kind = ? AND source = ?")
                .bind(media_key)
                .bind(kind.as_str())
                .bind(source.as_str())
                .execute(db)
                .await?;
        }
    }
    Ok(())
}

/// Remember which provider ids identify a media key.
pub async fn link_ids(db: &SqlitePool, media_key: &str, ids: &[String]) -> sqlx::Result<()> {
    for id in ids {
        sqlx::query("INSERT OR IGNORE INTO marker_ids (provider_id, media_key) VALUES (?, ?)").bind(id).bind(media_key).execute(db).await?;
    }
    Ok(())
}

// --- Lookup for scheduling -----------------------------------------------------------

pub struct ItemRef {
    pub id: i64,
    pub media_key: Option<String>,
    pub provider_ids: Vec<String>,
    pub season: Option<i64>,
    pub duration_ms: i64,
}

/// Every marker candidate for each item, from every source.
pub async fn candidates(db: &SqlitePool, items: &[ItemRef]) -> sqlx::Result<HashMap<i64, Vec<Marker>>> {
    let mut by_key: HashMap<String, Vec<StoredMarker>> = HashMap::new();
    let rows: Vec<(String, String, String, i64, i64, i64, Option<f64>, i64)> =
        sqlx::query_as("SELECT media_key, kind, source, start_ms, end_ms, duration_ms, confidence, updated_at FROM markers").fetch_all(db).await?;
    for (key, kind, source, start_ms, end_ms, duration_ms, confidence, updated_at) in rows {
        by_key.entry(key).or_default().push(StoredMarker { kind, source, start_ms, end_ms, duration_ms, confidence, updated_at });
    }
    let ids: HashMap<String, String> = sqlx::query_as::<_, (String, String)>("SELECT provider_id, media_key FROM marker_ids")
        .fetch_all(db)
        .await?
        .into_iter()
        .collect();
    let all_rules = rules(db, None).await?;

    let mut out: HashMap<i64, Vec<Marker>> = HashMap::new();
    for it in items {
        // The item's own key, plus any key its provider ids point at (a title
        // renamed on one server still matches).
        let mut keys: Vec<&String> = it.media_key.iter().collect();
        for pid in &it.provider_ids {
            if let Some(k) = ids.get(pid) {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        let mut list: Vec<Marker> = keys
            .iter()
            .filter_map(|k| by_key.get(*k))
            .flatten()
            .filter_map(|m| adjust(m, it.duration_ms))
            .collect();
        if let Some(show) = it.media_key.as_deref().and_then(|k| k.rsplit_once('/')).map(|(s, _)| s) {
            for kind in ["intro", "credits"] {
                // A season-specific rule beats a whole-show one.
                let rule = all_rules
                    .iter()
                    .filter(|r| r.show_key == show && r.kind == kind && (r.season.is_none() || r.season == it.season))
                    .max_by_key(|r| r.season.is_some());
                if let Some(m) = rule.and_then(|r| r.apply(it.duration_ms)) {
                    list.push(m);
                }
            }
        }
        if !list.is_empty() {
            out.insert(it.id, list);
        }
    }
    Ok(out)
}

/// Resolved (best valid, enabled source) markers for each item.
pub async fn resolved(db: &SqlitePool, items: &[ItemRef]) -> sqlx::Result<HashMap<i64, Vec<Marker>>> {
    let sources = sources(db).await;
    let durations: HashMap<i64, i64> = items.iter().map(|i| (i.id, i.duration_ms)).collect();
    Ok(candidates(db, items)
        .await?
        .into_iter()
        .filter_map(|(id, ms)| {
            let r = cuts::resolve(&ms, *durations.get(&id)?, sources);
            (!r.is_empty()).then_some((id, r))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_server_independent() {
        assert_eq!(norm("The Office (US)"), "office-us");
        assert_eq!(media_key("episode", "Pilot", Some("Cowboy Bebop"), Some(1), Some(5), None).unwrap(), "tv/cowboy-bebop/s01e05");
        assert_eq!(media_key("movie", "Akira", None, None, None, Some(1988)).unwrap(), "movie/akira/1988");
        assert!(media_key("episode", "x", Some("Show"), None, Some(1), None).is_none());
    }

    #[test]
    fn rule_from_end() {
        let r = Rule { show_key: "tv/s".into(), season: Some(1), kind: "credits".into(), start_ms: 90_000, end_ms: 0, from_end: true, updated_at: 0 };
        let m = r.apply(1_440_000).unwrap();
        assert_eq!((m.start_ms, m.end_ms), (1_350_000, 1_440_000));
    }

    #[test]
    fn markers_follow_small_length_differences() {
        let credits = StoredMarker {
            kind: "credits".into(),
            source: "manual".into(),
            start_ms: 1_300_000,
            end_ms: 1_390_000,
            duration_ms: 1_400_000,
            confidence: None,
            updated_at: 0,
        };
        // Another server's copy is 4 s longer: credits move with the end.
        let m = adjust(&credits, 1_404_000).unwrap();
        assert_eq!((m.start_ms, m.end_ms), (1_304_000, 1_394_000));
        let intro = StoredMarker { kind: "intro".into(), start_ms: 60_000, end_ms: 150_000, ..credits.clone() };
        assert_eq!(adjust(&intro, 1_404_000).unwrap().start_ms, 60_000);
        // A 20-minute-longer extended cut is a different file: ignore.
        assert!(adjust(&credits, 2_600_000).is_none());
    }
}
