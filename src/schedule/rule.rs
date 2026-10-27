//! Channel content filters.

use serde::{Deserialize, Serialize};

/// An item as seen by the scheduler: just the fields filters and ordering need.
#[derive(Debug, Clone, Default)]
pub struct Item {
    pub id: i64,
    pub library_id: i64,
    pub is_movie: bool,
    pub title: String,
    pub show_key: Option<String>,
    pub show_title: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub year: Option<i64>,
    pub duration_ms: i64,
    pub content_rating: Option<String>,
    pub studio: Option<String>,
    pub genres: Vec<String>,
    pub collections: Vec<String>,
    /// Intro/credits markers, already filtered to the enabled sources.
    pub markers: Vec<super::cuts::Marker>,
}

impl Item {
    /// Key used to group episodes of the same show. Movies are their own group.
    pub fn group_key(&self) -> String {
        match &self.show_key {
            Some(k) if !self.is_movie => format!("show:{k}"),
            _ => format!("item:{}", self.id),
        }
    }
}

/// What a channel (or a time slot on it) plays. Empty lists mean "no restriction".
/// All text comparisons are exact (case-insensitive) — no substring matching.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub libraries: Vec<i64>,
    /// "movie" and/or "episode"
    pub kinds: Vec<String>,
    pub genres: Vec<String>,
    pub exclude_genres: Vec<String>,
    pub collections: Vec<String>,
    /// Show keys (as stored in media_items.show_key)
    pub shows: Vec<String>,
    pub studios: Vec<String>,
    pub ratings: Vec<String>,
    pub year_min: Option<i64>,
    pub year_max: Option<i64>,
    pub include_items: Vec<i64>,
    pub exclude_items: Vec<i64>,
}

fn any_eq(wanted: &[String], have: &[String]) -> bool {
    wanted.iter().any(|w| have.iter().any(|h| h.eq_ignore_ascii_case(w)))
}

fn opt_eq(wanted: &[String], have: &Option<String>) -> bool {
    have.as_ref().is_some_and(|h| wanted.iter().any(|w| w.eq_ignore_ascii_case(h)))
}

impl Rule {
    pub fn matches(&self, it: &Item) -> bool {
        if self.exclude_items.contains(&it.id) {
            return false;
        }
        if self.include_items.contains(&it.id) {
            return true;
        }
        if !self.include_items.is_empty() && self.is_only_items() {
            return false;
        }
        let kind = if it.is_movie { "movie" } else { "episode" };
        (self.libraries.is_empty() || self.libraries.contains(&it.library_id))
            && (self.kinds.is_empty() || self.kinds.iter().any(|k| k == kind))
            && (self.genres.is_empty() || any_eq(&self.genres, &it.genres))
            && !any_eq(&self.exclude_genres, &it.genres)
            && (self.collections.is_empty() || any_eq(&self.collections, &it.collections))
            && (self.shows.is_empty() || it.show_key.as_ref().is_some_and(|k| self.shows.contains(k)))
            && (self.studios.is_empty() || opt_eq(&self.studios, &it.studio))
            && (self.ratings.is_empty() || opt_eq(&self.ratings, &it.content_rating))
            && self.year_min.is_none_or(|min| it.year.is_some_and(|y| y >= min))
            && self.year_max.is_none_or(|max| it.year.is_some_and(|y| y <= max))
    }

    /// True when the rule is just a hand-picked list of items.
    fn is_only_items(&self) -> bool {
        self.libraries.is_empty()
            && self.kinds.is_empty()
            && self.genres.is_empty()
            && self.exclude_genres.is_empty()
            && self.collections.is_empty()
            && self.shows.is_empty()
            && self.studios.is_empty()
            && self.ratings.is_empty()
            && self.year_min.is_none()
            && self.year_max.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(genres: &[&str], rating: &str, year: i64) -> Item {
        Item {
            id: 1,
            library_id: 1,
            show_key: Some("s".into()),
            genres: genres.iter().map(|g| g.to_string()).collect(),
            content_rating: Some(rating.into()),
            year: Some(year),
            duration_ms: 1,
            ..Default::default()
        }
    }

    #[test]
    fn exact_matching() {
        let r = Rule { ratings: vec!["PG".into()], ..Default::default() };
        assert!(!r.matches(&ep(&[], "TV-PG", 1990)), "PG must not match TV-PG");
        assert!(r.matches(&ep(&[], "pg", 1990)));
    }

    #[test]
    fn empty_rule_matches_everything() {
        assert!(Rule::default().matches(&ep(&[], "R", 2001)));
    }

    #[test]
    fn decade_and_genre() {
        let r = Rule { genres: vec!["Comedy".into()], year_min: Some(1990), year_max: Some(1999), ..Default::default() };
        assert!(r.matches(&ep(&["Comedy", "Family"], "TV-PG", 1994)));
        assert!(!r.matches(&ep(&["Comedy"], "TV-PG", 2004)));
        assert!(!r.matches(&ep(&["Drama"], "TV-PG", 1994)));
    }

    #[test]
    fn hand_picked_items_only() {
        let r = Rule { include_items: vec![7], ..Default::default() };
        assert!(!r.matches(&ep(&[], "R", 2000)));
        let mut it = ep(&[], "R", 2000);
        it.id = 7;
        assert!(r.matches(&it));
    }
}
