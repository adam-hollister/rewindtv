use std::collections::{BTreeMap, HashMap};

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::error::ApiResult;
use crate::schedule::autogen::{self, Suggestion};
use crate::schedule::rule::Rule;
use crate::schedule::store;
use crate::state::AppState;

#[derive(Serialize)]
pub struct Count {
    name: String,
    count: usize,
}

#[derive(Serialize)]
pub struct Show {
    key: String,
    title: String,
    episodes: usize,
}

#[derive(Serialize)]
pub struct Lib {
    id: i64,
    name: String,
    source: String,
}

#[derive(Serialize)]
pub struct Facets {
    libraries: Vec<Lib>,
    genres: Vec<Count>,
    collections: Vec<Count>,
    studios: Vec<Count>,
    ratings: Vec<Count>,
    shows: Vec<Show>,
    year_min: Option<i64>,
    year_max: Option<i64>,
}

fn counts(map: HashMap<String, usize>) -> Vec<Count> {
    let mut v: Vec<Count> = map.into_iter().map(|(name, count)| Count { name, count }).collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    v
}

/// Everything the channel editor needs to offer as filter chips.
pub async fn facets(State(st): State<AppState>) -> ApiResult<Json<Facets>> {
    let items = store::load_items(&st.db).await?;
    let (mut genres, mut collections, mut studios, mut ratings) = (HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());
    let mut shows: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for it in &items {
        for g in &it.genres {
            *genres.entry(g.clone()).or_insert(0) += 1;
        }
        for c in &it.collections {
            *collections.entry(c.clone()).or_insert(0) += 1;
        }
        if let Some(s) = &it.studio {
            *studios.entry(s.clone()).or_insert(0) += 1;
        }
        if let Some(r) = &it.content_rating {
            *ratings.entry(r.clone()).or_insert(0) += 1;
        }
        if let (false, Some(k)) = (it.is_movie, &it.show_key) {
            shows.entry(k.clone()).or_insert_with(|| (it.show_title.clone().unwrap_or_default(), 0)).1 += 1;
        }
    }
    let mut shows: Vec<Show> = shows.into_iter().map(|(key, (title, episodes))| Show { key, title, episodes }).collect();
    shows.sort_by_key(|a| a.title.to_lowercase());
    let libraries = sqlx::query_as::<_, (i64, String, String)>(
        "SELECT l.id, l.name, s.name FROM libraries l JOIN sources s ON s.id = l.source_id WHERE l.enabled = 1 ORDER BY s.name, l.name",
    )
    .fetch_all(&st.db)
    .await?
    .into_iter()
    .map(|(id, name, source)| Lib { id, name, source })
    .collect();
    Ok(Json(Facets {
        libraries,
        genres: counts(genres),
        collections: counts(collections),
        studios: counts(studios),
        ratings: counts(ratings),
        shows,
        year_min: items.iter().filter_map(|i| i.year).min(),
        year_max: items.iter().filter_map(|i| i.year).max(),
    }))
}

#[derive(Serialize)]
pub struct Preview {
    items: usize,
    hours: f64,
    shows: usize,
    movies: usize,
    sample: Vec<String>,
}

/// Live "matches N items / H hours" counter for the editor.
pub async fn preview(State(st): State<AppState>, Json(rule): Json<Rule>) -> ApiResult<Json<Preview>> {
    let items = store::load_items(&st.db).await?;
    let matched: Vec<_> = items.iter().filter(|i| rule.matches(i)).collect();
    let mut shows: Vec<&str> = matched.iter().filter(|i| !i.is_movie).filter_map(|i| i.show_title.as_deref()).collect();
    shows.sort_unstable();
    shows.dedup();
    let movies: Vec<&str> = matched.iter().filter(|i| i.is_movie).map(|i| i.title.as_str()).collect();
    let mut sample: Vec<String> = shows.iter().take(8).map(|s| s.to_string()).collect();
    sample.extend(movies.iter().take(8usize.saturating_sub(sample.len())).map(|s| s.to_string()));
    let ms: i64 = matched.iter().map(|i| i.duration_ms).sum();
    Ok(Json(Preview {
        items: matched.len(),
        hours: (ms as f64 / 360_000.0).round() / 10.0,
        shows: shows.len(),
        movies: movies.len(),
        sample,
    }))
}

pub async fn suggestions(State(st): State<AppState>) -> ApiResult<Json<Vec<Suggestion>>> {
    let items = store::load_items(&st.db).await?;
    Ok(Json(autogen::suggest(&items)))
}
