//! One-click channel ideas derived from what's in the library.

use std::collections::HashMap;

use serde::Serialize;

use super::ordering::Ordering;
use super::rule::{Item, Rule};

#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    /// "genre", "collection", "decade", "show", "studio"
    pub kind: &'static str,
    pub name: String,
    pub rule: Rule,
    pub ordering: Ordering,
    pub items: usize,
    pub hours: f64,
}

/// Minimum hours of content for a suggestion to be worth a channel.
const MIN_HOURS: f64 = 6.0;

#[derive(Default)]
struct Tally {
    items: usize,
    ms: i64,
    shows: usize,
}

fn add(map: &mut HashMap<String, Tally>, key: &str, it: &Item) {
    let t = map.entry(key.to_string()).or_default();
    t.items += 1;
    t.ms += it.duration_ms;
}

pub fn suggest(items: &[Item]) -> Vec<Suggestion> {
    let mut genres = HashMap::new();
    let mut collections = HashMap::new();
    let mut decades = HashMap::new();
    let mut studios = HashMap::new();
    let mut shows: HashMap<String, (String, Tally)> = HashMap::new();
    for it in items {
        for g in &it.genres {
            add(&mut genres, g, it);
        }
        for c in &it.collections {
            add(&mut collections, c, it);
        }
        if let Some(y) = it.year {
            add(&mut decades, &(y / 10 * 10).to_string(), it);
        }
        if let Some(s) = &it.studio {
            add(&mut studios, s, it);
        }
        if let (false, Some(k)) = (it.is_movie, &it.show_key) {
            let e = shows.entry(k.clone()).or_insert_with(|| (it.show_title.clone().unwrap_or_default(), Tally::default()));
            e.1.items += 1;
            e.1.ms += it.duration_ms;
            e.1.shows = 1;
        }
    }

    let hours = |t: &Tally| t.ms as f64 / 3_600_000.0;
    let mut out = Vec::new();
    let mut push = |kind: &'static str, name: String, rule: Rule, ordering: Ordering, t: &Tally| {
        if hours(t) >= MIN_HOURS {
            out.push(Suggestion { kind, name, rule, ordering, items: t.items, hours: (hours(t) * 10.0).round() / 10.0 });
        }
    };

    for (g, t) in sorted(&genres) {
        push("genre", g.clone(), Rule { genres: vec![g.clone()], ..Default::default() }, Ordering::Shuffle, t);
    }
    for (c, t) in sorted(&collections) {
        push("collection", c.clone(), Rule { collections: vec![c.clone()], ..Default::default() }, Ordering::Shuffle, t);
    }
    let mut dec: Vec<_> = decades.iter().collect();
    dec.sort_by_key(|(d, _)| d.parse::<i64>().unwrap_or(0));
    for (d, t) in dec {
        let start: i64 = d.parse().unwrap_or(0);
        let rule = Rule { year_min: Some(start), year_max: Some(start + 9), ..Default::default() };
        push("decade", format!("The {}s", start % 100), rule, Ordering::Shuffle, t);
    }
    for (s, t) in sorted(&studios) {
        push("studio", s.clone(), Rule { studios: vec![s.clone()], ..Default::default() }, Ordering::Episodic, t);
    }
    let mut show_list: Vec<_> = shows.into_iter().collect();
    show_list.sort_by_key(|s| std::cmp::Reverse(s.1 .1.ms));
    for (key, (title, t)) in show_list {
        let rule = Rule { shows: vec![key], ..Default::default() };
        push("show", format!("{title} 24/7"), rule, Ordering::Marathon, &t);
    }
    out
}

fn sorted(map: &HashMap<String, Tally>) -> Vec<(&String, &Tally)> {
    let mut v: Vec<_> = map.iter().collect();
    v.sort_by(|a, b| b.1.ms.cmp(&a.1.ms).then(a.0.cmp(b.0)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_genres_decades_and_shows_with_enough_content() {
        let items: Vec<Item> = (0..40)
            .map(|i| Item {
                id: i,
                show_key: Some("s1".into()),
                show_title: Some("Cheers".into()),
                genres: vec!["Comedy".into()],
                year: Some(1985),
                duration_ms: 22 * 60_000,
                ..Default::default()
            })
            .collect();
        let s = suggest(&items);
        assert!(s.iter().any(|s| s.kind == "genre" && s.name == "Comedy"));
        assert!(s.iter().any(|s| s.kind == "decade" && s.name == "The 80s" && s.rule.year_min == Some(1980)));
        assert!(s.iter().any(|s| s.kind == "show" && s.name == "Cheers 24/7" && s.ordering == Ordering::Marathon));
        assert!(suggest(&items[..5]).is_empty(), "too little content");
    }
}
