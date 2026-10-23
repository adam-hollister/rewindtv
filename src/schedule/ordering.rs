//! How a channel picks the next item from its pool, with persisted cursors so
//! the schedule continues naturally each time the guide is extended.

use std::collections::{HashMap, HashSet, VecDeque};

use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};

use super::rule::Item;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ordering {
    /// Random order; every item airs once before any repeats.
    Shuffle,
    /// One episode per show, rotating between shows, each show in airing order.
    Episodic,
    /// `block_size` consecutive episodes of a show, then the next show.
    Block,
    /// A whole show start to finish, then the next show.
    Marathon,
}

impl Ordering {
    pub fn parse(s: &str) -> Self {
        serde_json::from_value(serde_json::Value::String(s.into())).unwrap_or(Ordering::Shuffle)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PoolState {
    deck: Vec<i64>,
    cursors: HashMap<String, usize>,
    show_order: Vec<String>,
    show_pos: usize,
    current: Option<String>,
    block_left: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelState {
    pub pools: HashMap<String, PoolState>,
    /// (item id, start ms) of recent airings, oldest first.
    pub recent: VecDeque<(i64, i64)>,
}

impl ChannelState {
    pub fn record(&mut self, item: i64, at_ms: i64, window_ms: i64) {
        self.recent.push_back((item, at_ms));
        while self.recent.front().is_some_and(|(_, t)| *t < at_ms - window_ms) {
            self.recent.pop_front();
        }
    }

    fn recent_set(&self, now_ms: i64, window_ms: i64) -> HashSet<i64> {
        self.recent.iter().filter(|(_, t)| *t >= now_ms - window_ms).map(|(i, _)| *i).collect()
    }

    fn last(&self) -> Option<i64> {
        self.recent.back().map(|(i, _)| *i)
    }
}

pub struct Picker<'a> {
    pub ordering: Ordering,
    pub block_size: usize,
    pub items: &'a [Item],
    /// Indices into `items` making up this pool.
    pub pool: &'a [usize],
    pub avoid_repeat_ms: i64,
}

impl Picker<'_> {
    /// Returns the index (into `items`) of the next item to air.
    pub fn next<R: Rng>(&self, pool_key: &str, state: &mut ChannelState, now_ms: i64, rng: &mut R) -> Option<usize> {
        if self.pool.is_empty() {
            return None;
        }
        let recent = state.recent_set(now_ms, self.avoid_repeat_ms);
        let last = state.last();
        let ps = state.pools.entry(pool_key.to_string()).or_default();
        match self.ordering {
            Ordering::Shuffle => self.shuffle(ps, &recent, last, rng),
            _ => self.sequential(ps, rng),
        }
    }

    fn shuffle<R: Rng>(&self, ps: &mut PoolState, recent: &HashSet<i64>, last: Option<i64>, rng: &mut R) -> Option<usize> {
        let by_id: HashMap<i64, usize> = self.pool.iter().map(|&ix| (self.items[ix].id, ix)).collect();
        loop {
            while let Some(id) = ps.deck.pop() {
                if let Some(&ix) = by_id.get(&id) {
                    return Some(ix);
                }
            }
            // Deal a fresh deck. The deck is popped from the end, so put
            // recently aired items at the front (they come out last).
            let mut ids: Vec<i64> = by_id.keys().copied().collect();
            ids.shuffle(rng);
            ids.sort_by_key(|id| !recent.contains(id));
            if ids.len() > 1 && ids.last() == last.as_ref() {
                let n = ids.len();
                ids.swap(n - 1, n - 2);
            }
            ps.deck = ids;
        }
    }

    fn sequential<R: Rng>(&self, ps: &mut PoolState, rng: &mut R) -> Option<usize> {
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for &ix in self.pool {
            groups.entry(self.items[ix].group_key()).or_default().push(ix);
        }
        for g in groups.values_mut() {
            g.sort_by_key(|&ix| {
                let it = &self.items[ix];
                (it.season.unwrap_or(0) == 0, it.season, it.episode, it.year, it.id)
            });
        }

        // Rebuild the rotation if the set of shows changed.
        let keys: HashSet<&String> = groups.keys().collect();
        if ps.show_order.len() != keys.len() || ps.show_order.iter().any(|k| !keys.contains(k)) {
            ps.show_order = groups.keys().cloned().collect();
            ps.show_order.shuffle(rng);
            ps.show_pos = 0;
            ps.current = None;
            ps.block_left = 0;
        }

        let current_valid = ps.current.as_ref().is_some_and(|c| groups.contains_key(c));
        if ps.block_left == 0 || !current_valid {
            if current_valid {
                ps.show_pos += 1;
            }
            if ps.show_pos >= ps.show_order.len() {
                let prev = ps.current.clone();
                ps.show_order.shuffle(rng);
                if ps.show_order.len() > 1 && ps.show_order.first() == prev.as_ref() {
                    ps.show_order.swap(0, 1);
                }
                ps.show_pos = 0;
            }
            let key = ps.show_order[ps.show_pos].clone();
            let len = groups[&key].len();
            ps.block_left = match self.ordering {
                Ordering::Episodic => 1,
                Ordering::Block => self.block_size.max(1),
                // Rest of the show from wherever its cursor is.
                _ => len - ps.cursors.get(&key).copied().unwrap_or(0) % len,
            };
            ps.current = Some(key);
        }

        let key = ps.current.clone()?;
        let group = &groups[&key];
        let pos = ps.cursors.get(&key).copied().unwrap_or(0) % group.len();
        ps.cursors.insert(key, (pos + 1) % group.len());
        ps.block_left -= 1;
        Some(group[pos])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn library() -> Vec<Item> {
        let mut items = Vec::new();
        for (show, eps) in [("a", 5), ("b", 3), ("c", 4)] {
            for e in 1..=eps {
                items.push(Item {
                    id: items.len() as i64 + 1,
                    show_key: Some(show.into()),
                    season: Some(1),
                    episode: Some(e),
                    duration_ms: 1_000,
                    ..Default::default()
                });
            }
        }
        items
    }

    fn run(ordering: Ordering, block: usize, n: usize) -> Vec<Item> {
        let items = library();
        let pool: Vec<usize> = (0..items.len()).collect();
        let p = Picker { ordering, block_size: block, items: &items, pool: &pool, avoid_repeat_ms: 10_000 };
        let mut st = ChannelState::default();
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        (0..n)
            .map(|i| {
                let ix = p.next("default", &mut st, i as i64, &mut rng).unwrap();
                st.record(items[ix].id, i as i64, 10_000);
                items[ix].clone()
            })
            .collect()
    }

    #[test]
    fn shuffle_airs_everything_before_repeating() {
        let out = run(Ordering::Shuffle, 0, 24);
        let first: HashSet<i64> = out[..12].iter().map(|i| i.id).collect();
        assert_eq!(first.len(), 12);
        for w in out.windows(2) {
            assert_ne!(w[0].id, w[1].id, "no back-to-back repeats");
        }
    }

    #[test]
    fn episodic_rotates_shows_in_episode_order() {
        let out = run(Ordering::Episodic, 0, 9);
        for w in out.windows(2) {
            assert_ne!(w[0].show_key, w[1].show_key);
        }
        for show in ["a", "b", "c"] {
            let eps: Vec<i64> = out.iter().filter(|i| i.show_key.as_deref() == Some(show)).filter_map(|i| i.episode).collect();
            let mut sorted = eps.clone();
            sorted.sort();
            assert_eq!(eps, sorted, "show {show} airs in order");
        }
    }

    #[test]
    fn block_airs_consecutive_runs() {
        let out = run(Ordering::Block, 2, 6);
        for chunk in out.chunks(2) {
            assert_eq!(chunk[0].show_key, chunk[1].show_key);
            assert_eq!(chunk[1].episode.unwrap(), chunk[0].episode.unwrap() + 1);
        }
        assert_ne!(out[1].show_key, out[2].show_key);
    }

    #[test]
    fn marathon_plays_whole_show() {
        let out = run(Ordering::Marathon, 0, 12);
        let mut runs = vec![(out[0].show_key.clone(), 0)];
        for it in &out {
            if runs.last().unwrap().0 != it.show_key {
                runs.push((it.show_key.clone(), 0));
            }
            runs.last_mut().unwrap().1 += 1;
        }
        let lens: HashMap<_, _> = [("a", 5), ("b", 3), ("c", 4)].into_iter().collect();
        for (show, len) in runs {
            assert_eq!(len, lens[show.as_deref().unwrap()]);
        }
    }

    #[test]
    fn cursor_continues_after_reload() {
        let items = library();
        let pool: Vec<usize> = (0..items.len()).collect();
        let p = Picker { ordering: Ordering::Marathon, block_size: 0, items: &items, pool: &pool, avoid_repeat_ms: 0 };
        let mut st = ChannelState::default();
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let a = p.next("d", &mut st, 0, &mut rng).unwrap();
        let mut st: ChannelState = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        let b = p.next("d", &mut st, 0, &mut rng).unwrap();
        assert_eq!(items[a].show_key, items[b].show_key);
        assert_eq!(items[b].episode.unwrap(), items[a].episode.unwrap() + 1);
    }
}
