//! Intro/credits skipping: which parts of an episode to cut, and mapping
//! between "time into the program" (guide time) and "time into the file".

use serde::{Deserialize, Serialize};

/// A marker from any source, in file time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub kind: Kind,
    pub start_ms: i64,
    pub end_ms: i64,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Intro,
    Credits,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "intro" => Some(Kind::Intro),
            "credits" => Some(Kind::Credits),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Intro => "intro",
            Kind::Credits => "credits",
        }
    }
}

/// Where a marker came from, highest priority first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Set by hand for this episode.
    Manual,
    /// From a manual season/show rule.
    Rule,
    /// From Plex/Jellyfin.
    Server,
    /// Found by RewindTV's audio fingerprinting.
    Detected,
}

impl Source {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "manual" => Some(Source::Manual),
            "rule" => Some(Source::Rule),
            "server" => Some(Source::Server),
            "detected" => Some(Source::Detected),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Source::Manual => "manual",
            Source::Rule => "rule",
            Source::Server => "server",
            Source::Detected => "detected",
        }
    }
}

/// Which marker sources the user has switched on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sources {
    pub server: bool,
    pub manual: bool,
    pub detect: bool,
}

impl Default for Sources {
    fn default() -> Self {
        Self { server: true, manual: true, detect: false }
    }
}

impl Sources {
    fn allows(&self, s: Source) -> bool {
        match s {
            Source::Manual | Source::Rule => self.manual,
            Source::Server => self.server,
            Source::Detected => self.detect,
        }
    }
}

/// What a channel skips.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skip {
    pub intros: bool,
    pub credits: bool,
}

impl Skip {
    pub fn any(&self) -> bool {
        self.intros || self.credits
    }
}

/// Cuts shorter than this aren't worth a jump.
const MIN_CUT_MS: i64 = 5_000;
/// Cuts closer together than this are merged.
const MERGE_GAP_MS: i64 = 2_000;
/// Credits ending this close to the end run to the end of the file.
const END_SNAP_MS: i64 = 5_000;
/// A marker longer than this share of the runtime is treated as bogus.
const MAX_SHARE: f64 = 0.4;

/// Is this marker plausible for a file of `duration_ms`?
pub fn valid(m: &Marker, duration_ms: i64) -> bool {
    let len = m.end_ms - m.start_ms;
    m.start_ms >= 0
        && len > 0
        && m.end_ms <= duration_ms + 1_000
        && (len as f64) <= duration_ms as f64 * MAX_SHARE
        // Intros live in the first half; credits in the second.
        && match m.kind {
            Kind::Intro => m.start_ms < duration_ms / 2,
            Kind::Credits => m.end_ms > duration_ms / 2,
        }
}

/// The best valid marker of each kind among the enabled sources.
pub fn resolve(markers: &[Marker], duration_ms: i64, sources: Sources) -> Vec<Marker> {
    let mut out: Vec<Marker> = Vec::new();
    for kind in [Kind::Intro, Kind::Credits] {
        let best = markers
            .iter()
            .filter(|m| m.kind == kind && sources.allows(m.source) && valid(m, duration_ms))
            .min_by_key(|m| m.source);
        if let Some(m) = best {
            out.push(*m);
        }
    }
    out
}

/// File-time ranges to cut, sorted, merged and clamped. Post-credit scenes
/// survive because only the credits range itself is cut.
pub fn cuts_for(resolved: &[Marker], duration_ms: i64, skip: Skip) -> Vec<(i64, i64)> {
    let mut cuts: Vec<(i64, i64)> = resolved
        .iter()
        .filter(|m| match m.kind {
            Kind::Intro => skip.intros,
            Kind::Credits => skip.credits,
        })
        .map(|m| {
            let end = if m.kind == Kind::Credits && duration_ms - m.end_ms <= END_SNAP_MS { duration_ms } else { m.end_ms.min(duration_ms) };
            (m.start_ms.max(0), end)
        })
        .collect();
    cuts.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (s, e) in cuts {
        match merged.last_mut() {
            Some(last) if s <= last.1 + MERGE_GAP_MS => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged.retain(|(s, e)| e - s >= MIN_CUT_MS);
    // Never cut the whole thing.
    if merged.iter().map(|(s, e)| e - s).sum::<i64>() >= duration_ms {
        return Vec::new();
    }
    merged
}

/// The parts of the file that play, in order.
pub fn kept_ranges(duration_ms: i64, cuts: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut pos = 0;
    for &(s, e) in cuts {
        if s > pos {
            out.push((pos, s));
        }
        pos = pos.max(e);
    }
    if pos < duration_ms {
        out.push((pos, duration_ms));
    }
    out
}

/// How long the program runs on air.
pub fn effective_len(duration_ms: i64, cuts: &[(i64, i64)]) -> i64 {
    kept_ranges(duration_ms, cuts).iter().map(|(s, e)| e - s).sum()
}

/// Where in the file we are `program_offset_ms` into the program, and which
/// kept range that falls in.
pub fn media_pos(duration_ms: i64, cuts: &[(i64, i64)], program_offset_ms: i64) -> (i64, usize) {
    let kept = kept_ranges(duration_ms, cuts);
    let mut left = program_offset_ms.max(0);
    for (i, (s, e)) in kept.iter().enumerate() {
        if left < e - s {
            return (s + left, i);
        }
        left -= e - s;
    }
    match kept.last() {
        Some(&(_, e)) => (e, kept.len() - 1),
        None => (duration_ms, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn m(kind: Kind, s: i64, e: i64, source: Source) -> Marker {
        Marker { kind, start_ms: s, end_ms: e, source }
    }

    #[test]
    fn priority_and_validation() {
        let dur = 24 * MIN;
        let markers = [
            m(Kind::Intro, 90_000, 180_000, Source::Detected),
            m(Kind::Intro, 92_000, 182_000, Source::Server),
            m(Kind::Intro, 0, 15 * MIN, Source::Manual), // 62% of the runtime: bogus
            m(Kind::Credits, 22 * MIN, 23 * MIN + 30_000, Source::Rule),
        ];
        let r = resolve(&markers, dur, Sources::default());
        assert_eq!(r[0], markers[1], "server beats detected; the bogus manual one is ignored");
        assert_eq!(r[1].source, Source::Rule);
        let r = resolve(&markers, dur, Sources { server: false, manual: false, detect: true });
        assert_eq!(r, vec![markers[0]]);
    }

    #[test]
    fn keeps_post_credit_scene_and_maps_time() {
        let dur = 24 * MIN;
        let markers = [m(Kind::Intro, 60_000, 150_000, Source::Server), m(Kind::Credits, 21 * MIN, 23 * MIN, Source::Server)];
        let cuts = cuts_for(&markers, dur, Skip { intros: true, credits: true });
        assert_eq!(cuts, vec![(60_000, 150_000), (21 * MIN, 23 * MIN)]);
        assert_eq!(kept_ranges(dur, &cuts), vec![(0, 60_000), (150_000, 21 * MIN), (23 * MIN, dur)]);
        assert_eq!(effective_len(dur, &cuts), dur - 90_000 - 2 * MIN);
        assert_eq!(media_pos(dur, &cuts, 30_000), (30_000, 0));
        assert_eq!(media_pos(dur, &cuts, 60_000), (150_000, 1), "right after the cold open comes the episode");
        let into_scene = 60_000 + (21 * MIN - 150_000) + 10_000;
        assert_eq!(media_pos(dur, &cuts, into_scene), (23 * MIN + 10_000, 2));
    }

    #[test]
    fn credits_near_end_run_to_end_and_tiny_cuts_vanish() {
        let dur = 24 * MIN;
        let markers = [m(Kind::Intro, 0, 3_000, Source::Server), m(Kind::Credits, 22 * MIN, dur - 2_000, Source::Server)];
        let cuts = cuts_for(&markers, dur, Skip { intros: true, credits: true });
        assert_eq!(cuts, vec![(22 * MIN, dur)]);
        assert_eq!(kept_ranges(dur, &cuts), vec![(0, 22 * MIN)]);
    }

    #[test]
    fn respects_channel_toggles_and_merges() {
        let dur = 24 * MIN;
        let markers = [m(Kind::Intro, 60_000, 150_000, Source::Server), m(Kind::Credits, 21 * MIN, 23 * MIN, Source::Server)];
        assert_eq!(cuts_for(&markers, dur, Skip { intros: false, credits: true }), vec![(21 * MIN, 23 * MIN)]);
        assert!(cuts_for(&markers, dur, Skip::default()).is_empty());
        let close = [m(Kind::Intro, 0, 60_000, Source::Server), m(Kind::Credits, 61_000, 61_500, Source::Server)];
        // The credits marker here is invalid (first half), so only the intro remains.
        assert_eq!(cuts_for(&resolve(&close, dur, Sources::default()), dur, Skip { intros: true, credits: true }), vec![(0, 60_000)]);
    }
}
