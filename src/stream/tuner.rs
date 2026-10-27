//! Continuous MPEG-TS channel streams for Live TV tuners (Plex, Jellyfin) and
//! IPTV apps. Each program is produced by its own ffmpeg; RewindTV rewrites
//! the timestamps so programs join seamlessly, and starts the next program a
//! few seconds early so changes don't stall while the server spins up.

use std::process::Stdio;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::db::now_ms;
use crate::error::{ApiError, ApiResult};
use crate::guide::{self, ProgramInfo};
use crate::media::Source;
use crate::schedule::cuts;
use crate::schedule::store;
use crate::state::AppState;

/// Start the next program's transcode this long before the current one ends.
const PREFETCH_MS: i64 = 6_000;
/// How long to wait for output before sending keepalive packets.
const KEEPALIVE: Duration = Duration::from_millis(500);

pub async fn program_at(st: &AppState, channel_id: i64, t_ms: i64) -> anyhow::Result<Option<ProgramInfo>> {
    if let Some(p) = guide::at(&st.db, channel_id, t_ms).await? {
        return Ok(Some(p));
    }
    let _guard = st.guide_lock.lock().await;
    let items = st.items().await?;
    store::extend_channel(&st.db, &items, channel_id, st.cfg.tz, now_ms()).await?;
    Ok(guide::at(&st.db, channel_id, t_ms).await?)
}

// --- Timestamp stitching ---------------------------------------------------------

const TS: usize = 188;
const WRAP: u64 = 1 << 33;
/// Gap between the end of one program and the start of the next (90 kHz ticks ≈ 33 ms).
const JOIN_GAP: u64 = 3_000;
/// Hold at most this many packets while looking for a program's first timestamps.
const MAX_PENDING: usize = 400;

/// Seven MPEG-TS null packets (PID 0x1FFF), which every demuxer discards.
pub static NULL_PACKETS: [u8; TS * 7] = {
    let mut b = [0xFFu8; TS * 7];
    let mut i = 0;
    while i < 7 {
        b[i * TS] = 0x47;
        b[i * TS + 1] = 0x1F;
        b[i * TS + 2] = 0xFF;
        b[i * TS + 3] = 0x10;
        i += 1;
    }
    b
};

fn read_ts(t: &[u8]) -> u64 {
    ((t[0] as u64 >> 1) & 7) << 30 | (t[1] as u64) << 22 | ((t[2] as u64) >> 1) << 15 | (t[3] as u64) << 7 | (t[4] as u64) >> 1
}

fn write_ts(t: &mut [u8], v: u64) {
    t[0] = (t[0] & 0xF1) | ((v >> 29) & 0x0E) as u8;
    t[1] = (v >> 22) as u8;
    t[2] = (t[2] & 0x01) | ((v >> 14) & 0xFE) as u8;
    t[3] = (v >> 7) as u8;
    t[4] = (t[4] & 0x01) | ((v << 1) & 0xFE) as u8;
}

/// Byte offsets of the timestamps in one TS packet.
#[derive(Default)]
struct Stamps {
    pcr: Option<usize>,
    pts: Option<usize>,
    dts: Option<usize>,
}

fn locate(pkt: &[u8]) -> Stamps {
    let mut s = Stamps::default();
    let afc = (pkt[3] >> 4) & 3;
    let mut payload = 4;
    if afc & 2 != 0 {
        let len = pkt[4] as usize;
        if len > 0 && pkt[5] & 0x10 != 0 && len >= 7 {
            s.pcr = Some(6);
        }
        payload = 5 + len;
    }
    if afc & 1 == 0 || pkt[1] & 0x40 == 0 || payload + 19 > TS {
        return s;
    }
    let p = &pkt[payload..];
    if p[..3] != [0, 0, 1] || !(0xC0..=0xEF).contains(&p[3]) {
        return s;
    }
    if p[7] & 0x80 != 0 {
        s.pts = Some(payload + 9);
    }
    if p[7] & 0xC0 == 0xC0 {
        s.dts = Some(payload + 14);
    }
    s
}

fn read_pcr(pkt: &[u8], at: usize) -> u64 {
    (pkt[at] as u64) << 25 | (pkt[at + 1] as u64) << 17 | (pkt[at + 2] as u64) << 9 | (pkt[at + 3] as u64) << 1 | (pkt[at + 4] as u64) >> 7
}

fn write_pcr(pkt: &mut [u8], at: usize, v: u64) {
    pkt[at] = (v >> 25) as u8;
    pkt[at + 1] = (v >> 17) as u8;
    pkt[at + 2] = (v >> 9) as u8;
    pkt[at + 3] = (v >> 1) as u8;
    pkt[at + 4] = (pkt[at + 4] & 0x7F) | (((v & 1) as u8) << 7);
}

/// Shifts each program's PTS/DTS/PCR so it starts right after the previous
/// program ended, giving the viewer one continuous timeline.
pub struct Stitcher {
    carry: Vec<u8>,
    /// Packets held until the program's earliest timestamp is known.
    pending: Vec<u8>,
    pending_min: Option<u64>,
    pending_pids: Vec<u16>,
    /// Added (mod 2^33) to every timestamp of the current program.
    delta: Option<u64>,
    /// Where the current program should start, in unwrapped 90 kHz ticks.
    start_at: u64,
    /// Latest presentation time written so far (unwrapped).
    max_out: Option<u64>,
}

impl Stitcher {
    pub fn new(start_at: u64) -> Self {
        Self { carry: Vec::new(), pending: Vec::new(), pending_min: None, pending_pids: Vec::new(), delta: None, start_at, max_out: None }
    }

    /// Feed bytes of the current program; returns bytes ready to send.
    pub fn process(&mut self, data: &[u8]) -> Vec<u8> {
        self.carry.extend_from_slice(data);
        let mut out = Vec::with_capacity(self.carry.len());
        let mut i = 0;
        while i + TS <= self.carry.len() {
            if self.carry[i] != 0x47 {
                i += 1; // resync
                continue;
            }
            let mut pkt = [0u8; TS];
            pkt.copy_from_slice(&self.carry[i..i + TS]);
            i += TS;
            if self.delta.is_none() {
                self.hold(pkt);
                if self.ready() {
                    self.release(&mut out);
                }
            } else {
                self.rewrite(&mut pkt);
                out.extend_from_slice(&pkt);
            }
        }
        self.carry.drain(..i);
        out
    }

    /// The current program has ended: flush anything held back and line up
    /// the next program to start right after it.
    pub fn finish_program(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.delta.is_none() && !self.pending.is_empty() {
            self.release(&mut out);
        }
        self.carry.clear();
        self.delta = None;
        if let Some(max) = self.max_out {
            self.start_at = self.start_at.max(max + JOIN_GAP);
        }
        out
    }

    fn hold(&mut self, pkt: [u8; TS]) {
        let st = locate(&pkt);
        let pid = ((pkt[1] as u16 & 0x1F) << 8) | pkt[2] as u16;
        for at in [st.pts, st.dts].into_iter().flatten() {
            let v = read_ts(&pkt[at..at + 5]);
            self.pending_min = Some(self.pending_min.map_or(v, |m| m.min(v)));
            if !self.pending_pids.contains(&pid) {
                self.pending_pids.push(pid);
            }
        }
        self.pending.extend_from_slice(&pkt);
    }

    fn ready(&self) -> bool {
        self.pending_pids.len() >= 2 || self.pending.len() / TS >= MAX_PENDING
    }

    fn release(&mut self, out: &mut Vec<u8>) {
        let min = self.pending_min.unwrap_or(0);
        self.delta = Some((self.start_at + WRAP - min % WRAP) % WRAP);
        let held = std::mem::take(&mut self.pending);
        for chunk in held.chunks_exact(TS) {
            let mut pkt = [0u8; TS];
            pkt.copy_from_slice(chunk);
            self.rewrite(&mut pkt);
            out.extend_from_slice(&pkt);
        }
        self.pending_min = None;
        self.pending_pids.clear();
    }

    fn rewrite(&mut self, pkt: &mut [u8; TS]) {
        let delta = self.delta.unwrap_or(0);
        let st = locate(pkt);
        if let Some(at) = st.pcr {
            let v = (read_pcr(pkt, at) + delta) % WRAP;
            write_pcr(pkt, at, v);
        }
        if let Some(at) = st.dts {
            let v = (read_ts(&pkt[at..at + 5]) + delta) % WRAP;
            write_ts(&mut pkt[at..at + 5], v);
        }
        if let Some(at) = st.pts {
            let shifted = read_ts(&pkt[at..at + 5]) + delta;
            write_ts(&mut pkt[at..at + 5], shifted % WRAP);
            // Unwrap relative to where this program started.
            let base = self.start_at - self.start_at % WRAP;
            let mut unwrapped = base + shifted % WRAP;
            if unwrapped + WRAP / 2 < self.start_at {
                unwrapped += WRAP;
            }
            self.max_out = Some(self.max_out.map_or(unwrapped, |m| m.max(unwrapped)));
        }
    }
}

// --- Producing programs ------------------------------------------------------------

/// Starts a uniform transcode on the media server for one program and returns
/// the URL ffmpeg should read (our own HLS proxy, which adds credentials), the
/// session id, and how far ffmpeg itself must seek.
async fn open_server_session(st: &AppState, src: Source, prog: &ProgramInfo, offset_ms: i64) -> (String, String, f64) {
    let sid = uuid::Uuid::new_v4().simple().to_string();
    let master = src.tuner_url(prog.item_key.as_deref().unwrap_or_default(), prog.part_key.as_deref(), offset_ms, &sid);
    src.prepare(&master).await;
    st.sessions.insert(sid.clone(), src, master).await;
    let offset_s = offset_ms as f64 / 1000.0;
    let full_s = prog.duration_ms.unwrap_or(prog.end_ms - prog.start_ms) as f64 / 1000.0;
    // Did the server start at the offset (Plex), or hand back the whole program (Jellyfin)?
    let seek = match crate::stream::hls::playlist_seconds(st, &sid).await {
        Ok(total) if offset_s > 0.0 && (total - full_s).abs() < (total - (full_s - offset_s)).abs() => offset_s,
        _ => 0.0,
    };
    let url = format!("http://127.0.0.1:{}/stream/{sid}/master.m3u8", st.cfg.port);
    (url, sid, seek)
}

/// ffmpeg arguments for one program, or for color bars when `input` is None.
fn ffmpeg_args(st: &AppState, input: Option<&str>, seek_s: f64, length_s: f64) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin", "-re"].map(String::from).to_vec();
    match input {
        Some(url) => {
            if seek_s > 0.0 {
                // Non-accurate seek starts cleanly on the keyframe before the offset.
                // Only when needed: even `-ss 0` on HLS input drops leading video.
                a.extend(["-noaccurate_seek".into(), "-ss".into(), format!("{seek_s:.3}")]);
            }
            a.extend(["-i".into(), url.into()]);
            a.extend(["-t", &format!("{length_s:.3}"), "-map", "0:v:0", "-map", "0:a:0?"].map(String::from));
            if st.cfg.iptv_video == "h264" {
                a.extend(["-vf", "scale=-2:720,fps=30", "-c:v", "libx264", "-preset", "veryfast", "-b:v", "4M", "-g", "60"].map(String::from));
            } else {
                a.extend(["-c:v", "copy"].map(String::from));
            }
        }
        None => {
            // Retro color bars with silence, matching the 1080p tuner feed.
            a.extend(["-f", "lavfi", "-i", "smptehdbars=size=1920x1080:rate=30", "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo"].map(String::from));
            a.extend(["-t".into(), format!("{length_s:.3}")]);
            a.extend(["-c:v", "libx264", "-preset", "ultrafast", "-tune", "stillimage", "-profile:v", "high", "-g", "60", "-pix_fmt", "yuv420p"].map(String::from));
        }
    }
    a.extend(["-c:a", "aac", "-ac", "2", "-ar", "48000", "-b:a", "192k"].map(String::from));
    a.extend(["-muxdelay", "0", "-muxpreload", "0", "-mpegts_flags", "+resend_headers", "-f", "mpegts", "pipe:1"].map(String::from));
    a
}

/// One stretch of a program to play: the file range [media_start, media_end),
/// which airs until `air_end`. A program with skipped intro/credits is several
/// units back to back.
#[derive(Clone)]
struct Unit {
    prog: ProgramInfo,
    range: usize,
    media_start: i64,
    media_end: i64,
    air_end: i64,
}

/// The unit on air `offset_ms` into a program.
fn unit_at(prog: ProgramInfo, offset_ms: i64) -> Unit {
    let len = prog.media_len();
    let cuts = prog.cut_ranges();
    let kept = cuts::kept_ranges(len, &cuts);
    let (pos, range) = cuts::media_pos(len, &cuts, offset_ms);
    let aired_before: i64 = kept[..range.min(kept.len())].iter().map(|(s, e)| e - s).sum();
    let (_, end) = kept.get(range).copied().unwrap_or((pos, len));
    let air_end = prog.start_ms + aired_before + (end - kept.get(range).map_or(pos, |r| r.0));
    Unit { prog, range, media_start: pos, media_end: end, air_end }
}

/// The next kept range of the same program, if any.
fn next_in_program(u: &Unit) -> Option<Unit> {
    let kept = cuts::kept_ranges(u.prog.media_len(), &u.prog.cut_ranges());
    let &(s, e) = kept.get(u.range + 1)?;
    Some(Unit { prog: u.prog.clone(), range: u.range + 1, media_start: s, media_end: e, air_end: u.air_end + (e - s) })
}

/// One unit being produced in the background.
struct Segment {
    unit: Unit,
    rx: mpsc::Receiver<Bytes>,
    got_data: bool,
}

fn start_segment(st: &AppState, unit: Unit, bars: bool) -> Segment {
    let (tx, rx) = mpsc::channel(512);
    tokio::spawn(run_segment(st.clone(), unit.clone(), bars, tx));
    Segment { unit, rx, got_data: false }
}

async fn run_segment(st: AppState, unit: Unit, bars: bool, tx: mpsc::Sender<Bytes>) {
    let prog = &unit.prog;
    let offset_ms = unit.media_start;
    let length_ms = unit.media_end - unit.media_start;
    if length_ms < 300 {
        return;
    }
    let started = Instant::now();
    let mut seek_s = offset_ms as f64 / 1000.0;
    let mut session: Option<String> = None;
    let input = if bars || prog.is_filler() {
        None
    } else {
        match st.source(prog.source_id.unwrap_or_default()).await {
            Ok(src) if st.cfg.iptv_video == "server" => {
                let (url, sid, seek) = open_server_session(&st, src, prog, offset_ms).await;
                session = Some(sid);
                seek_s = seek;
                Some(url)
            }
            Ok(src) => src.file_url(prog.item_key.as_deref().unwrap_or_default(), prog.part_key.as_deref()).map(|u| u.to_string()),
            Err(_) => None,
        }
    };
    let args = ffmpeg_args(&st, input.as_deref(), seek_s, length_ms as f64 / 1000.0);
    let child = Command::new(&st.cfg.ffmpeg).args(&args).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("could not start {}: {e}", st.cfg.ffmpeg);
            stop_session(&st, session.as_deref()).await;
            return;
        }
    };
    let mut stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::warn!("ffmpeg: {}", crate::media::redact(&line));
        }
    });
    let mut buf = vec![0u8; TS * 256];
    let mut first = true;
    loop {
        match stdout.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if first {
                    tracing::debug!(
                        "program {} file {:.1}s–{:.1}s: first bytes after {:?}",
                        prog.id,
                        unit.media_start as f64 / 1000.0,
                        unit.media_end as f64 / 1000.0,
                        started.elapsed()
                    );
                    first = false;
                }
                if tx.send(Bytes::copy_from_slice(&buf[..n])).await.is_err() {
                    // Nobody is watching any more.
                    let _ = child.kill().await;
                    break;
                }
            }
        }
    }
    let _ = child.wait().await;
    stop_session(&st, session.as_deref()).await;
}

async fn stop_session(st: &AppState, sid: Option<&str>) {
    if let Some(sid) = sid {
        st.sessions.stop(sid).await;
    }
}

/// GET /iptv/stream/:channel.ts
pub async fn stream(State(st): State<AppState>, Path(file): Path<String>) -> ApiResult<Response> {
    let channel_id: i64 = file.trim_end_matches(".ts").parse().map_err(|_| ApiError::NotFound)?;
    let Ok(slot) = st.tuner_slots.clone().try_acquire_owned() else {
        return Ok((axum::http::StatusCode::SERVICE_UNAVAILABLE, "All tuners are busy").into_response());
    };
    let now = now_ms();
    let first = program_at(&st, channel_id, now).await?.ok_or(ApiError::NotFound)?;
    let offset = now - first.start_ms;
    let mut cur = start_segment(&st, unit_at(first, offset), false);

    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(64);
    tokio::spawn(async move {
        let _slot = slot; // released when the viewer disconnects
        let mut stitch = Stitcher::new(90_000);
        let mut next: Option<Segment> = None;
        let mut failed: Option<i64> = None;
        loop {
            // Play the current unit to its end.
            loop {
                if next.is_none() && now_ms() >= cur.unit.air_end - PREFETCH_MS {
                    let upcoming = match next_in_program(&cur.unit) {
                        Some(u) => Some(u),
                        None => program_at(&st, channel_id, cur.unit.prog.end_ms).await.ok().flatten().map(|p| unit_at(p, 0)),
                    };
                    next = upcoming.map(|u| start_segment(&st, u, false));
                }
                let out = match tokio::time::timeout(KEEPALIVE, cur.rx.recv()).await {
                    Ok(Some(b)) => {
                        cur.got_data = true;
                        Bytes::from(stitch.process(&b))
                    }
                    Ok(None) => break,
                    // Still waiting for the server's transcode: keep the tuner
                    // connection alive with null packets.
                    Err(_) if !cur.got_data => Bytes::from_static(&NULL_PACKETS),
                    Err(_) => continue,
                };
                if !out.is_empty() && tx.send(Ok(out)).await.is_err() {
                    return; // viewer left; dropping the segments stops them
                }
            }
            let tail = stitch.finish_program();
            if !tail.is_empty() && tx.send(Ok(Bytes::from(tail))).await.is_err() {
                return;
            }

            let now = now_ms();
            if !cur.got_data && failed != Some(cur.unit.prog.id) && cur.unit.prog.end_ms - now > 1_000 {
                // Source failed (file gone, server down): color bars for the rest of it.
                tracing::warn!("channel {channel_id}: no output for program {}, showing bars", cur.unit.prog.id);
                failed = Some(cur.unit.prog.id);
                let prog = cur.unit.prog.clone();
                let bars = Unit { media_start: 0, media_end: prog.end_ms - now, air_end: prog.end_ms, range: usize::MAX - 1, prog };
                drop(next.take());
                cur = start_segment(&st, bars, true);
                continue;
            }
            cur = match next.take() {
                // Only use the prefetched unit if we haven't fallen behind.
                Some(n) if n.unit.air_end > now + 1_000 => n,
                _ => {
                    let cursor = cur.unit.air_end.max(now - 2_000);
                    let Ok(Some(p)) = program_at(&st, channel_id, cursor).await else { break };
                    let offset = (cursor - p.start_ms).max(0);
                    start_segment(&st, unit_at(p, offset), false)
                }
            };
        }
    });

    Ok((
        [(header::CONTENT_TYPE, "video/mp2t"), (header::CACHE_CONTROL, "no-cache")],
        Body::from_stream(ReceiverStream::new(rx)),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A video PES-start packet with PTS (and DTS), optionally carrying a PCR.
    fn pes(pid: u16, pts: u64, dts: Option<u64>, pcr: Option<u64>) -> Vec<u8> {
        let mut p = vec![0x47, 0x40 | (pid >> 8) as u8, pid as u8];
        if let Some(pcr) = pcr {
            p.extend([0x30, 7, 0x10, 0, 0, 0, 0, 0x7E, 0]);
            write_pcr(&mut p, 6, pcr);
        } else {
            p.push(0x10);
        }
        let stream_id = if pid == 0x100 { 0xE0 } else { 0xC0 };
        let flags = if dts.is_some() { 0xC0 } else { 0x80 };
        p.extend([0, 0, 1, stream_id, 0, 0, 0x80, flags, if dts.is_some() { 10 } else { 5 }]);
        let at = p.len();
        p.extend([0x21, 0, 1, 0, 1]);
        write_ts(&mut p[at..at + 5], pts);
        if let Some(d) = dts {
            let at = p.len();
            p.extend([0x11, 0, 1, 0, 1]);
            write_ts(&mut p[at..at + 5], d);
        }
        p.resize(TS, 0xFF);
        p
    }

    fn times(bytes: &[u8]) -> Vec<(u64, Option<u64>, Option<u64>)> {
        bytes
            .chunks_exact(TS)
            .map(|pkt| {
                let s = locate(pkt);
                (s.pts.map(|a| read_ts(&pkt[a..a + 5])).unwrap_or(0), s.dts.map(|a| read_ts(&pkt[a..a + 5])), s.pcr.map(|a| read_pcr(pkt, a)))
            })
            .collect()
    }

    #[test]
    fn timestamps_round_trip() {
        let pkt = pes(0x100, 123_456_789, Some(123_000_000), Some(122_999_000));
        assert_eq!(times(&pkt), vec![(123_456_789, Some(123_000_000), Some(122_999_000))]);
    }

    #[test]
    fn programs_are_joined_end_to_start() {
        let mut s = Stitcher::new(90_000);
        // Program A: starts at 5 s (in its own clock), lasts 10 s.
        let mut a = pes(0x100, 450_000, Some(447_000), Some(447_000));
        a.extend(pes(0x101, 450_000, None, None));
        a.extend(pes(0x100, 1_350_000, None, None));
        let out_a: Vec<u8> = a.chunks(100).flat_map(|c| s.process(c)).collect();
        let out_a = [out_a, s.finish_program()].concat();
        let ta = times(&out_a);
        assert_eq!(ta[0], (93_000, Some(90_000), Some(90_000)), "earliest timestamp lands on start_at");
        assert_eq!(ta[2].0, 993_000);

        // Program B: its own clock starts near zero; it must continue after A.
        let mut b = pes(0x101, 0, None, None);
        b.extend(pes(0x100, 3_000, Some(0), Some(0)));
        let out_b = [s.process(&b), s.finish_program()].concat();
        let tb = times(&out_b);
        assert_eq!(tb[0].0, 993_000 + JOIN_GAP);
        assert_eq!(tb[1].1, Some(993_000 + JOIN_GAP));
    }

    fn episode(cuts: &str) -> ProgramInfo {
        // A 24-minute episode with a cold open, intro, show, credits and a stinger.
        ProgramInfo {
            id: 1,
            start_ms: 1_000_000,
            end_ms: 1_000_000 + 1_440_000 - 90_000 - 90_000,
            item_id: Some(1),
            duration_ms: Some(1_440_000),
            cuts: Some(cuts.into()),
            ..Default::default()
        }
    }

    #[test]
    fn units_follow_the_kept_ranges() {
        let p = episode("[[60000,150000],[1290000,1380000]]");
        let u = unit_at(p.clone(), 0);
        assert_eq!((u.media_start, u.media_end, u.air_end), (0, 60_000, 1_060_000), "cold open first");
        let u2 = next_in_program(&u).unwrap();
        assert_eq!((u2.media_start, u2.media_end), (150_000, 1_290_000), "intro skipped");
        assert_eq!(u2.air_end, 1_060_000 + 1_140_000);
        let u3 = next_in_program(&u2).unwrap();
        assert_eq!((u3.media_start, u3.media_end), (1_380_000, 1_440_000), "credits skipped, stinger kept");
        assert_eq!(u3.air_end, p.end_ms, "the last unit ends with the program");
        assert!(next_in_program(&u3).is_none());

        // Tuning in 70 s into the program lands 10 s after the intro.
        let mid = unit_at(p.clone(), 70_000);
        assert_eq!((mid.media_start, mid.range), (160_000, 1));
        assert_eq!(mid.air_end, u2.air_end);
    }

    #[test]
    fn programs_without_cuts_are_one_unit() {
        let mut p = episode("[]");
        p.cuts = None;
        p.end_ms = p.start_ms + 1_440_000;
        let u = unit_at(p.clone(), 5_000);
        assert_eq!((u.media_start, u.media_end, u.air_end), (5_000, 1_440_000, p.end_ms));
        assert!(next_in_program(&u).is_none());
    }

    #[test]
    fn null_packets_pass_through() {
        let mut s = Stitcher::new(90_000);
        let mut data = pes(0x100, 0, None, None);
        data.extend(pes(0x101, 0, None, None));
        s.process(&data);
        assert_eq!(s.process(&NULL_PACKETS).len(), NULL_PACKETS.len());
    }
}
