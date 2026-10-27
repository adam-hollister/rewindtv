//! RewindTV's own intro/credits detection, for episodes the media server has no
//! markers for. Episodes of a season share their opening and closing themes,
//! so we fingerprint the start and end of each episode's audio (chromaprint,
//! via ffmpeg) and look for the longest stretch two episodes have in common.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::process::Command;

use crate::db::now_ms;
use crate::schedule::cuts::{Kind, Source as MarkerSource};
use crate::state::AppState;

/// Chromaprint's default algorithm emits one point per 4096/3 samples at 11025 Hz.
pub const POINT_MS: f64 = 4096.0 / 3.0 / 11.025;
/// Points that differ in at most this many of 32 bits still match.
const MAX_BIT_ERRORS: u32 = 6;
/// Match density is judged over windows this long.
const WINDOW_MS: f64 = 3_000.0;
/// Minimum share of matching points inside a shared stretch.
const MIN_DENSITY: f64 = 0.6;
/// Where to look: the first quarter (≤ 10 min) for intros, the last 4 minutes for credits.
const HEAD_MAX_MS: i64 = 10 * 60_000;
const TAIL_MS: i64 = 4 * 60_000;
const INTRO_MIN_MS: i64 = 15_000;
const INTRO_MAX_MS: i64 = 150_000;
const CREDITS_MIN_MS: i64 = 15_000;

// --- Matching (pure) -------------------------------------------------------------

/// A shared stretch: start/end point indices in `a`, and the shift to `b`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Run {
    pub a_start: usize,
    pub a_end: usize,
    pub shift: isize,
    /// Share of points in the run that matched.
    pub quality: f64,
}

impl Run {
    pub fn len(&self) -> usize {
        self.a_end - self.a_start
    }
}

/// Longest stretch of audio `a` and `b` have in common, at any offset.
pub fn longest_shared(a: &[u32], b: &[u32]) -> Option<Run> {
    if a.len() < 20 || b.len() < 20 {
        return None;
    }
    // Values that repeat a lot (silence, tones) say nothing about alignment.
    let mut freq: HashMap<u32, usize> = HashMap::new();
    for v in a.iter().chain(b) {
        *freq.entry(*v).or_default() += 1;
    }
    let common = (a.len() + b.len()) / 50 + 2;
    // Score every alignment on a sample of points. Re-encoded copies rarely
    // share exact values, so this counts near-matches rather than equal ones.
    let mut shifts: Vec<(isize, usize)> = Vec::new();
    for shift in -(a.len() as isize - 1)..(b.len() as isize) {
        let lo = 0.max(-shift) as usize;
        let hi = (a.len() as isize).min(b.len() as isize - shift).max(0) as usize;
        let score = (lo..hi)
            .step_by(3)
            .filter(|&i| freq[&a[i]] <= common && (a[i] ^ b[(i as isize + shift) as usize]).count_ones() <= MAX_BIT_ERRORS)
            .count();
        if score >= 5 {
            shifts.push((shift, score));
        }
    }
    shifts.sort_by(|x, y| y.1.cmp(&x.1));
    shifts.truncate(16);

    let window = (WINDOW_MS / POINT_MS) as usize;
    let mut best: Option<Run> = None;
    for (shift, _) in shifts {
        let lo = 0.max(-shift) as usize;
        let hi = (a.len() as isize).min(b.len() as isize - shift).max(0) as usize;
        if hi <= lo + window {
            continue;
        }
        let good: Vec<bool> = (lo..hi).map(|i| (a[i] ^ b[(i as isize + shift) as usize]).count_ones() <= MAX_BIT_ERRORS).collect();
        // How often unrelated audio matches at this alignment; a shared theme
        // has to stand well clear of that.
        let background = good.iter().filter(|g| **g).count() as f64 / good.len() as f64;
        let need = (background + 0.3).clamp(MIN_DENSITY, 0.95);
        // Points whose surrounding window is dense enough in matches.
        let mut prefix = vec![0usize; good.len() + 1];
        for (k, g) in good.iter().enumerate() {
            prefix[k + 1] = prefix[k] + *g as usize;
        }
        let half = window / 2;
        let dense: Vec<bool> = (0..good.len())
            .map(|k| {
                let (s, e) = (k.saturating_sub(half), (k + half + 1).min(good.len()));
                (prefix[e] - prefix[s]) as f64 / (e - s) as f64 >= need
            })
            .collect();
        let mut k = 0;
        while k < dense.len() {
            if !dense[k] {
                k += 1;
                continue;
            }
            let start = k;
            while k < dense.len() && dense[k] {
                k += 1;
            }
            // Trim to the first and last actual matches.
            let (Some(mut s), Some(mut e)) = ((start..k).find(|&x| good[x]), (start..k).rev().find(|&x| good[x])) else { continue };
            // The window blurs the edges; grow over directly adjacent matches.
            while s > 0 && good[s - 1] {
                s -= 1;
            }
            while e + 1 < good.len() && good[e + 1] {
                e += 1;
            }
            let len = e + 1 - s;
            let hits = prefix[e + 1] - prefix[s];
            let distinct: HashSet<u32> = (s..=e).map(|x| a[lo + x]).collect();
            // Require variety, so long silences don't count as a theme song.
            if len < 8 || distinct.len() * 3 < len {
                continue;
            }
            let run = Run { a_start: lo + s, a_end: lo + e + 1, shift, quality: hits as f64 / len as f64 };
            if best.is_none_or(|b| run.len() > b.len()) {
                best = Some(run);
            }
        }
    }
    best
}

/// Median of a list (by value).
fn median(mut v: Vec<i64>) -> Option<i64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

/// One episode's fingerprint window.
#[derive(Debug, Clone)]
pub struct Print {
    pub item_id: i64,
    pub offset_ms: i64,
    pub points: Vec<u32>,
}

/// For each episode, the stretch it shares with its neighbours in the season
/// (episodes N±1, N±2), in file time. `min_ms..=max_ms` bounds the length.
pub fn shared_ranges(prints: &[Print], min_ms: i64, max_ms: i64) -> HashMap<i64, (i64, i64, f64)> {
    let mut found: HashMap<i64, Vec<(i64, i64, f64)>> = HashMap::new();
    for i in 0..prints.len() {
        for d in [1, 2] {
            let Some(other) = prints.get(i + d) else { continue };
            let me = &prints[i];
            let Some(run) = longest_shared(&me.points, &other.points) else { continue };
            let len_ms = (run.len() as f64 * POINT_MS) as i64;
            if len_ms < min_ms || len_ms > max_ms {
                continue;
            }
            let to_ms = |p: &Print, idx: isize| p.offset_ms + (idx as f64 * POINT_MS) as i64;
            let a = (to_ms(me, run.a_start as isize), to_ms(me, run.a_end as isize));
            let b = (to_ms(other, run.a_start as isize + run.shift), to_ms(other, run.a_end as isize + run.shift));
            found.entry(me.item_id).or_default().push((a.0, a.1, run.quality));
            found.entry(other.item_id).or_default().push((b.0, b.1, run.quality));
        }
    }
    found
        .into_iter()
        .filter_map(|(id, v)| {
            let q = v.iter().map(|x| x.2).sum::<f64>() / v.len() as f64;
            let conf = (q * (v.len().min(3) as f64 / 3.0)).min(1.0);
            Some((id, (median(v.iter().map(|x| x.0).collect())?, median(v.iter().map(|x| x.1).collect())?, conf)))
        })
        .collect()
}

// --- Fingerprinting -------------------------------------------------------------------

/// Fingerprint `len_ms` of audio starting at `start_ms`.
async fn fingerprint(ffmpeg: &str, input: &str, start_ms: i64, len_ms: i64) -> anyhow::Result<Vec<u32>> {
    let out = tokio::time::timeout(
        Duration::from_secs(300),
        Command::new(ffmpeg)
            .args(["-nostdin", "-v", "error", "-threads", "1"])
            .args(["-ss", &format!("{:.3}", start_ms as f64 / 1000.0), "-t", &format!("{:.3}", len_ms as f64 / 1000.0)])
            .args(["-i", input, "-vn", "-sn", "-map", "0:a:0", "-ac", "1", "-f", "chromaprint", "-fp_format", "raw", "pipe:1"])
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim().chars().take(200).collect::<String>());
    }
    Ok(out.stdout.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

fn encode(points: &[u32]) -> Vec<u8> {
    points.iter().flat_map(|p| p.to_le_bytes()).collect()
}

fn decode(blob: &[u8]) -> Vec<u32> {
    blob.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// Window to fingerprint for a part of an episode.
fn window(part: &str, duration_ms: i64) -> (i64, i64) {
    match part {
        // A quarter of the episode (6 min of a half-hour show), but at least
        // 3 minutes on short episodes, never more than 10.
        "head" => (0, (duration_ms / 4).max((duration_ms * 6 / 10).min(180_000)).min(HEAD_MAX_MS)),
        _ => ((duration_ms - TAIL_MS).max(duration_ms / 2), duration_ms),
    }
}

/// A local copy of the file, if the media folder is mounted and mapped.
fn local_path(st: &AppState, server_path: Option<&str>) -> Option<String> {
    let p = server_path?;
    st.cfg.path_maps.iter().find_map(|(from, to)| {
        let rest = p.strip_prefix(from.as_str())?;
        let local = PathBuf::from(format!("{}{}", to, rest));
        local.exists().then(|| local.to_string_lossy().into_owned())
    })
}

#[derive(sqlx::FromRow, Clone)]
struct Ep {
    id: i64,
    source_id: i64,
    item_key: String,
    part_key: Option<String>,
    file_path: Option<String>,
    media_key: String,
    show_title: String,
    season: i64,
    episode: i64,
    duration_ms: i64,
}

async fn print_for(st: &AppState, ep: &Ep, part: &str) -> anyhow::Result<Print> {
    if let Some((offset_ms, blob)) = sqlx::query_as::<_, (i64, Vec<u8>)>("SELECT offset_ms, points FROM fingerprints WHERE item_id = ? AND part = ?")
        .bind(ep.id)
        .bind(part)
        .fetch_optional(&st.db)
        .await?
    {
        return Ok(Print { item_id: ep.id, offset_ms, points: decode(&blob) });
    }
    let input = match local_path(st, ep.file_path.as_deref()) {
        Some(p) => p,
        None => {
            let src = st.source(ep.source_id).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
            src.file_url(&ep.item_key, ep.part_key.as_deref()).ok_or_else(|| anyhow::anyhow!("no file URL"))?.to_string()
        }
    };
    let (start, end) = window(part, ep.duration_ms);
    let points = fingerprint(&st.cfg.ffmpeg, &input, start, end - start).await?;
    sqlx::query("INSERT OR REPLACE INTO fingerprints (item_id, part, offset_ms, points) VALUES (?, ?, ?, ?)")
        .bind(ep.id)
        .bind(part)
        .bind(start)
        .bind(encode(&points))
        .execute(&st.db)
        .await?;
    Ok(Print { item_id: ep.id, offset_ms: start, points })
}

// --- The background job ---------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub running: bool,
    pub current: Option<String>,
    pub done: usize,
    pub total: usize,
    pub found: usize,
    pub message: Option<String>,
    pub finished_at: Option<i64>,
}

/// Seasons worth scanning: shows on channels that skip something, with
/// episodes that still lack a manual or server marker. `only_channel` limits
/// the scan to one channel.
async fn seasons_to_scan(st: &AppState, only_channel: Option<i64>) -> anyhow::Result<Vec<Vec<Ep>>> {
    let items = crate::schedule::store::load_items(&st.db).await?;
    let channels: Vec<i64> = sqlx::query_scalar("SELECT id FROM channels WHERE (skip_intros = 1 OR skip_credits = 1) AND (?1 IS NULL OR id = ?1)")
        .bind(only_channel)
        .fetch_all(&st.db)
        .await?;
    let mut shows: HashSet<String> = HashSet::new();
    for id in channels {
        let Some(cfg) = crate::schedule::store::load_config(&st.db, id).await? else { continue };
        for it in items.iter().filter(|it| !it.is_movie) {
            if cfg.rule.matches(it) || cfg.slots.iter().any(|s| s.rule.matches(it)) {
                if let Some(k) = &it.show_key {
                    shows.insert(k.clone());
                }
            }
        }
    }
    let mut out = Vec::new();
    for show in shows {
        let eps: Vec<Ep> = sqlx::query_as(
            "SELECT id, source_id, item_key, part_key, file_path, media_key, show_title, season, episode, duration_ms
             FROM media_items WHERE show_key = ? AND missing = 0 AND media_key IS NOT NULL
               AND season IS NOT NULL AND episode IS NOT NULL
             ORDER BY season, episode",
        )
        .bind(&show)
        .fetch_all(&st.db)
        .await?;
        let mut by_season: HashMap<i64, Vec<Ep>> = HashMap::new();
        for e in eps {
            by_season.entry(e.season).or_default().push(e);
        }
        for (_, season) in by_season {
            // Only worth it if some episode still needs markers.
            let mut needs = false;
            for e in &season {
                let has: i64 = sqlx::query_scalar(
                    "SELECT COUNT(DISTINCT kind) FROM markers WHERE media_key = ? AND source IN ('manual', 'server')",
                )
                .bind(&e.media_key)
                .fetch_one(&st.db)
                .await?;
                let checked: Option<i64> = sqlx::query_scalar("SELECT detect_checked_at FROM media_items WHERE id = ?").bind(e.id).fetch_one(&st.db).await?;
                if has < 2 && checked.is_none() {
                    needs = true;
                    break;
                }
            }
            if needs && season.len() >= 2 {
                out.push(season);
            }
        }
    }
    out.sort_by(|a, b| (&a[0].show_title, a[0].season).cmp(&(&b[0].show_title, b[0].season)));
    Ok(out)
}

/// Scan for missing intro/credits markers. Runs one ffmpeg at a time.
pub async fn run(st: AppState, only_channel: Option<i64>) {
    {
        let mut s = st.detect_status.write().await;
        if s.running {
            return;
        }
        *s = Status { running: true, message: Some("Looking for shows to scan…".into()), ..Default::default() };
    }
    let seasons = seasons_to_scan(&st, only_channel).await.unwrap_or_else(|e| {
        tracing::warn!("marker detection: {e:#}");
        Vec::new()
    });
    let total: usize = seasons.iter().map(Vec::len).sum();
    st.detect_status.write().await.total = total;
    let mut found = 0;
    let mut touched: HashSet<String> = HashSet::new();
    for season in seasons {
        let label = format!("{} S{}", season[0].show_title, season[0].season);
        let mut prints: HashMap<&str, Vec<Print>> = HashMap::new();
        for ep in &season {
            {
                let mut s = st.detect_status.write().await;
                s.current = Some(format!("{label} · E{}", ep.episode));
            }
            for part in ["head", "tail"] {
                match print_for(&st, ep, part).await {
                    Ok(p) => prints.entry(part).or_default().push(p),
                    Err(e) => tracing::debug!("fingerprint {} {part}: {e:#}", ep.media_key),
                }
            }
            st.detect_status.write().await.done += 1;
        }
        let intros = shared_ranges(prints.get("head").map(Vec::as_slice).unwrap_or_default(), INTRO_MIN_MS, INTRO_MAX_MS);
        let credits = shared_ranges(prints.get("tail").map(Vec::as_slice).unwrap_or_default(), CREDITS_MIN_MS, TAIL_MS);
        for ep in &season {
            let mut rows = Vec::new();
            if let Some(&(s, e, c)) = intros.get(&ep.id) {
                rows.push((Kind::Intro, s, e, Some(c)));
            }
            if let Some(&(s, e, c)) = credits.get(&ep.id) {
                rows.push((Kind::Credits, s, e, Some(c)));
            }
            if !rows.is_empty() {
                if super::save(&st.db, &ep.media_key, MarkerSource::Detected, ep.duration_ms, &rows).await.is_ok() {
                    found += 1;
                    touched.insert(ep.media_key.clone());
                }
            }
            let _ = sqlx::query("UPDATE media_items SET detect_checked_at = ? WHERE id = ?").bind(now_ms()).bind(ep.id).execute(&st.db).await;
        }
        st.detect_status.write().await.found = found;
    }
    crate::sync::export_touched(&st, touched).await;
    if found > 0 {
        let _guard = st.guide_lock.lock().await;
        if let Err(e) = crate::schedule::store::refresh_skipping(&st.db, st.cfg.tz, now_ms() + 6 * 3_600_000).await {
            tracing::warn!("refreshing skipping channels: {e:#}");
        }
    }
    let mut s = st.detect_status.write().await;
    s.running = false;
    s.current = None;
    s.message = Some(if total == 0 { "Nothing new to scan on channels that skip intros/credits".into() } else { format!("Found markers for {found} of {total} episodes") });
    s.finished_at = Some(now_ms());
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};

    fn noise(rng: &mut rand::rngs::StdRng, n: usize) -> Vec<u32> {
        (0..n).map(|_| rng.gen()).collect()
    }

    /// Flip a couple of bits per point, like real re-encodes do.
    fn degrade(rng: &mut rand::rngs::StdRng, v: &[u32]) -> Vec<u32> {
        v.iter().map(|p| p ^ (1 << rng.gen_range(0..32)) ^ (1 << rng.gen_range(0..32))).collect()
    }

    #[test]
    fn finds_shared_theme_at_different_offsets() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(9);
        let theme = noise(&mut rng, 700); // ~87 s
        // Episode A: 30 s cold open, theme, show. Episode B: 75 s cold open.
        let a = [noise(&mut rng, 240), theme.clone(), noise(&mut rng, 1500)].concat();
        let b = [noise(&mut rng, 600), degrade(&mut rng, &theme), noise(&mut rng, 1200)].concat();
        let run = longest_shared(&a, &b).unwrap();
        assert!((run.a_start as i64 - 240).abs() <= 2, "{run:?}");
        assert!((run.len() as i64 - 700).abs() <= 4, "{run:?}");
        assert_eq!(run.shift, 360);
        assert!(run.quality > 0.9);
    }

    #[test]
    fn unrelated_audio_and_silence_dont_match() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(4);
        let a = noise(&mut rng, 2000);
        let b = noise(&mut rng, 2000);
        assert!(longest_shared(&a, &b).is_none_or(|r| r.len() < 20));
        let silence_a = [vec![0u32; 800], noise(&mut rng, 500)].concat();
        let silence_b = [vec![0u32; 800], noise(&mut rng, 500)].concat();
        assert!(longest_shared(&silence_a, &silence_b).is_none_or(|r| r.len() < 20), "silence is not a theme song");
    }

    #[test]
    fn season_consensus_in_file_time() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let theme = noise(&mut rng, 500);
        let colds = [100usize, 300, 0, 200];
        let prints: Vec<Print> = colds
            .iter()
            .enumerate()
            .map(|(i, &c)| Print { item_id: i as i64, offset_ms: 0, points: [noise(&mut rng, c), degrade(&mut rng, &theme), noise(&mut rng, 900)].concat() })
            .collect();
        let found = shared_ranges(&prints, INTRO_MIN_MS, INTRO_MAX_MS);
        for (i, &c) in colds.iter().enumerate() {
            let (s, e, conf) = found[&(i as i64)];
            let want = (c as f64 * POINT_MS) as i64;
            assert!((s - want).abs() < 400, "ep {i}: start {s} vs {want}");
            assert!(((e - s) - (500.0 * POINT_MS) as i64).abs() < 800, "ep {i}: len {}", e - s);
            assert!(conf > 0.5);
        }
    }
}
