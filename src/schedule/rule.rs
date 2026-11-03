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
    /// Playlists the item is on, with its position in each.
    pub playlists: Vec<(String, i64)>,
    /// Intro/credits markers, already filtered to the enabled sources.
    pub markers: Vec<super::cuts::Marker>,
    /// What the item's library is for.
    pub role: Role,
}

/// A library's job: regular programming, commercials, or station IDs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Content,
    Commercials,
    Bumpers,
}

impl Role {
    pub fn parse(s: &str) -> Self {
        match s {
            "commercials" => Role::Commercials,
            "bumpers" => Role::Bumpers,
            _ => Role::Content,
        }
    }
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
    /// Words that must appear in the episode or show title (any of them),
    /// e.g. "halloween" for a holiday block.
    pub keywords: Vec<String>,
    /// Server playlists by name.
    pub playlists: Vec<String>,
}

fn any_eq(wanted: &[String], have: &[String]) -> bool {
    wanted.iter().any(|w| have.iter().any(|h| h.eq_ignore_ascii_case(w)))
}

fn opt_eq(wanted: &[String], have: &Option<String>) -> bool {
    have.as_ref().is_some_and(|h| wanted.iter().any(|w| w.eq_ignore_ascii_case(h)))
}

impl Rule {
    pub fn matches(&self, it: &Item) -> bool {
        // Commercials and station IDs only ever air in breaks.
        if it.role != Role::Content {
            return false;
        }
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
            && (self.keywords.is_empty() || self.keywords_match(it))
            && (self.playlists.is_empty() || self.playlists.iter().any(|w| it.playlists.iter().any(|(p, _)| p.eq_ignore_ascii_case(w))))
    }

    /// True when the rule is just a hand-picked list of items.
    fn keywords_match(&self, it: &Item) -> bool {
        let title = it.title.to_lowercase();
        let show = it.show_title.as_deref().unwrap_or_default().to_lowercase();
        self.keywords
            .iter()
            .map(|k| k.trim().to_lowercase())
            .filter(|k| !k.is_empty())
            .any(|k| title.contains(&k) || show.contains(&k))
    }

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
            && self.keywords.is_empty()
            && self.playlists.is_empty()
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
    fn keywords_match_episode_or_show_titles() {
        let r = Rule { keywords: vec!["Halloween".into(), "haunted".into()], ..Default::default() };
        let mut it = ep(&[], "TV-PG", 1990);
        it.title = "Treehouse of Horror: A Halloween Special".into();
        assert!(r.matches(&it));
        it.title = "Pilot".into();
        it.show_title = Some("The Haunted Hathaways".into());
        assert!(r.matches(&it));
        it.show_title = Some("Cheers".into());
        assert!(!r.matches(&it));
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
