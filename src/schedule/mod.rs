//! Builds each channel's program guide: a rolling window of back-to-back
//! programs generated server-side, so every viewer sees the same thing.

pub mod autogen;
pub mod cuts;
pub mod ordering;
pub mod rule;
pub mod slots;
pub mod store;

use std::collections::HashMap;

use chrono::{Offset, TimeZone};
use chrono_tz::Tz;
use rand::Rng;

use ordering::{ChannelState, Ordering, Picker};
use rule::{Item, Rule};
use slots::Slot;

const MINUTE: i64 = 60_000;
/// Don't start a program in a block's last few minutes; air filler instead.
const MIN_SLOT_REMAINDER: i64 = 5 * MINUTE;
/// Filler length when nothing matches a channel's rules.
const EMPTY_FILLER: i64 = 30 * MINUTE;

#[derive(Debug, Clone)]
pub struct ChannelConfig {
    pub rule: Rule,
    pub ordering: Ordering,
    pub block_size: usize,
    pub avoid_repeat_hours: i64,
    pub align_minutes: i64,
    pub slots: Vec<Slot>,
    pub skip: cuts::Skip,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub start_ms: i64,
    pub end_ms: i64,
    /// Index into the items slice; `None` is filler.
    pub item: Option<usize>,
    pub slot_id: Option<i64>,
    /// File-time ranges skipped while this program airs.
    pub cuts: Vec<(i64, i64)>,
}

/// Round `t_ms` up to the next multiple of `minutes` in local time.
fn align_up(t_ms: i64, minutes: i64, tz: Tz) -> i64 {
    if minutes <= 0 {
        return t_ms;
    }
    let step = minutes * MINUTE;
    let offset = tz
        .timestamp_millis_opt(t_ms)
        .single()
        .map(|d| d.offset().fix().local_minus_utc() as i64 * 1000)
        .unwrap_or(0);
    let local = t_ms + offset;
    let aligned = (local + step - 1).div_euclid(step) * step;
    aligned - offset
}

fn push(out: &mut Vec<Program>, p: Program) {
    // Merge adjacent filler so the guide shows one block.
    if p.item.is_none() {
        if let Some(last) = out.last_mut() {
            if last.item.is_none() && last.end_ms == p.start_ms && last.slot_id == p.slot_id {
                last.end_ms = p.end_ms;
                return;
            }
        }
    }
    out.push(p);
}

/// Generate programs covering [from_ms, until_ms). The last program may run past `until_ms`.
pub fn generate<R: Rng>(
    cfg: &ChannelConfig,
    items: &[Item],
    state: &mut ChannelState,
    tz: Tz,
    from_ms: i64,
    until_ms: i64,
    rng: &mut R,
) -> Vec<Program> {
    let mut pools: HashMap<Option<i64>, Vec<usize>> = HashMap::new();
    let mut pool_for = |slot: Option<&Slot>| -> Vec<usize> {
        let key = slot.map(|s| s.id);
        let rule = slot.map(|s| &s.rule).unwrap_or(&cfg.rule);
        pools
            .entry(key)
            .or_insert_with(|| (0..items.len()).filter(|&ix| rule.matches(&items[ix])).collect())
            .clone()
    };
    let avoid_ms = cfg.avoid_repeat_hours.max(0) * 60 * MINUTE;
    // What each item loses to intro/credits skipping on this channel.
    let item_cuts: Vec<Vec<(i64, i64)>> = items
        .iter()
        .map(|it| if cfg.skip.any() { cuts::cuts_for(&it.markers, it.duration_ms, cfg.skip) } else { Vec::new() })
        .collect();
    let air_len = |ix: usize| cuts::effective_len(items[ix].duration_ms, &item_cuts[ix]);

    let mut out = Vec::new();
    let mut t = from_ms;
    while t < until_ms {
        let slot = slots::slot_at(&cfg.slots, tz, t);
        let slot_id = slot.map(|s| s.id);
        let boundary = slots::next_change(&cfg.slots, tz, t);
        let pool = pool_for(slot);

        if let Some(b) = boundary {
            if b - t < MIN_SLOT_REMAINDER {
                push(&mut out, Program { start_ms: t, end_ms: b, item: None, slot_id, cuts: Vec::new() });
                t = b;
                continue;
            }
        }

        let picker = Picker {
            ordering: slot.map(|s| s.ordering).unwrap_or(cfg.ordering),
            block_size: cfg.block_size,
            items,
            pool: &pool,
            avoid_repeat_ms: avoid_ms,
        };
        let pool_key = slot_id.map(|id| format!("slot:{id}")).unwrap_or_else(|| "default".into());
        let snapshot = state.pools.get(&pool_key).cloned();
        let Some(ix) = picker.next(&pool_key, state, t, rng) else {
            let end = boundary.map_or(t + EMPTY_FILLER, |b| b.min(t + EMPTY_FILLER));
            push(&mut out, Program { start_ms: t, end_ms: end, item: None, slot_id, cuts: Vec::new() });
            t = end;
            continue;
        };
        let end = t + air_len(ix);

        // Blocks start on time: if this pick would run into the next block,
        // give it back and air something shorter from the pool instead.
        let ix = match boundary {
            Some(b) if end > b => {
                match snapshot {
                    Some(s) => state.pools.insert(pool_key, s),
                    None => state.pools.remove(&pool_key),
                };
                let recent: Vec<i64> = state.recent.iter().map(|(id, _)| *id).collect();
                let fits: Vec<usize> = pool.iter().copied().filter(|&i| air_len(i) <= b - t).collect();
                let fresh: Vec<usize> = fits.iter().copied().filter(|i| !recent.contains(&items[*i].id)).collect();
                let choices = if fresh.is_empty() { &fits } else { &fresh };
                if choices.is_empty() {
                    push(&mut out, Program { start_ms: t, end_ms: b, item: None, slot_id, cuts: Vec::new() });
                    t = b;
                    continue;
                }
                choices[rng.gen_range(0..choices.len())]
            }
            _ => ix,
        };
        let end = t + air_len(ix);

        state.record(items[ix].id, t, avoid_ms.max(MINUTE));
        push(&mut out, Program { start_ms: t, end_ms: end, item: Some(ix), slot_id, cuts: item_cuts[ix].clone() });
        t = end;

        let aligned = align_up(t, cfg.align_minutes, tz);
        let aligned = boundary.filter(|&b| b > t).map_or(aligned, |b| aligned.min(b));
        if aligned > t {
            push(&mut out, Program { start_ms: t, end_ms: aligned, item: None, slot_id, cuts: Vec::new() });
            t = aligned;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn items() -> Vec<Item> {
        let mut v = Vec::new();
        for (show, genre, n, mins) in [("toons", "Animation", 20, 22), ("sit", "Comedy", 20, 23), ("movies", "Action", 6, 117)] {
            for e in 0..n {
                v.push(Item {
                    id: v.len() as i64 + 1,
                    is_movie: show == "movies",
                    show_key: (show != "movies").then(|| show.to_string()),
                    season: Some(1),
                    episode: Some(e + 1),
                    duration_ms: mins * MINUTE + 13_000,
                    genres: vec![genre.into()],
                    ..Default::default()
                });
            }
        }
        v
    }

    fn cfg(align: i64, slots: Vec<Slot>) -> ChannelConfig {
        ChannelConfig {
            rule: Rule::default(),
            ordering: Ordering::Shuffle,
            block_size: 2,
            avoid_repeat_hours: 6,
            align_minutes: align,
            slots,
            skip: cuts::Skip::default(),
        }
    }

    fn gen(c: &ChannelConfig, tz: Tz, from: i64, hours: i64) -> (Vec<Item>, Vec<Program>) {
        let items = items();
        let mut st = ChannelState::default();
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let p = generate(c, &items, &mut st, tz, from, from + hours * 60 * MINUTE, &mut rng);
        (items, p)
    }

    #[test]
    fn contiguous_with_no_overlaps() {
        let tz: Tz = "UTC".parse().unwrap();
        let (_, progs) = gen(&cfg(0, vec![]), tz, 0, 24 * 7);
        assert!(progs.last().unwrap().end_ms >= 24 * 7 * 60 * MINUTE);
        for w in progs.windows(2) {
            assert_eq!(w[0].end_ms, w[1].start_ms);
            assert!(w[0].end_ms > w[0].start_ms);
        }
    }

    #[test]
    fn aligned_programs_start_on_the_half_hour() {
        let tz: Tz = "Asia/Kolkata".parse().unwrap(); // +05:30
        let from = tz.with_ymd_and_hms(2026, 10, 23, 18, 0, 0).unwrap().timestamp_millis();
        let (_, progs) = gen(&cfg(30, vec![]), tz, from, 24);
        for p in progs.iter().filter(|p| p.item.is_some()) {
            let local = tz.timestamp_millis_opt(p.start_ms).unwrap();
            assert_eq!(chrono::Timelike::minute(&local) % 30, 0, "starts at {local}");
        }
    }

    #[test]
    fn slots_play_their_own_content_and_start_on_time() {
        let tz: Tz = "America/New_York".parse().unwrap();
        let cartoons = Slot {
            id: 9,
            title: "Saturday Morning".into(),
            days: 1 << 5,
            start_min: 7 * 60,
            end_min: 12 * 60,
            rule: Rule { genres: vec!["Animation".into()], ..Default::default() },
            ordering: Ordering::Episodic,
        };
        let mut c = cfg(0, vec![cartoons]);
        c.rule = Rule { genres: vec!["Action".into(), "Comedy".into()], ..Default::default() };
        let from = tz.with_ymd_and_hms(2026, 10, 23, 18, 0, 0).unwrap().timestamp_millis();
        let (items, progs) = gen(&c, tz, from, 48);
        let seven = tz.with_ymd_and_hms(2026, 10, 24, 7, 0, 0).unwrap().timestamp_millis();
        let noon = tz.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap().timestamp_millis();
        assert!(progs.iter().any(|p| p.start_ms == seven && p.item.is_some()), "block starts exactly at 7:00");
        for p in &progs {
            let Some(ix) = p.item else { continue };
            let toon = items[ix].genres[0] == "Animation";
            let inside = p.start_ms >= seven && p.start_ms < noon;
            assert_eq!(toon, inside, "program at {} in wrong block", p.start_ms);
        }
    }

    #[test]
    fn empty_pool_yields_filler() {
        let tz: Tz = "UTC".parse().unwrap();
        let mut c = cfg(0, vec![]);
        c.rule = Rule { genres: vec!["Western".into()], ..Default::default() };
        let (_, progs) = gen(&c, tz, 0, 2);
        assert!(progs.iter().all(|p| p.item.is_none()));
        assert_eq!(progs.len(), 1, "adjacent filler is merged");
    }
}
