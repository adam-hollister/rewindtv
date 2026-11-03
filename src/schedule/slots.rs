//! Time-of-day programming blocks ("Saturday 7–12: cartoons").

use chrono::{Datelike, Duration, NaiveTime, TimeZone, Timelike};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use super::ordering::Ordering;
use super::rule::Rule;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    pub id: i64,
    pub title: String,
    /// Bit 0 = Monday … bit 6 = Sunday.
    pub days: u8,
    /// Minutes after local midnight.
    pub start_min: u32,
    /// `end_min <= start_min` means the slot runs past midnight.
    pub end_min: u32,
    pub rule: Rule,
    pub ordering: Ordering,
    /// Optional yearly date range, (month, day) inclusive; may wrap the new year.
    #[serde(default)]
    pub dates: Option<((u32, u32), (u32, u32))>,
}

/// "10-31" -> (10, 31)
pub fn parse_md(s: &str) -> Option<(u32, u32)> {
    let (m, d) = s.trim().split_once('-')?;
    let (m, d): (u32, u32) = (m.parse().ok()?, d.parse().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((m, d))
}

fn in_range(md: (u32, u32), (from, to): ((u32, u32), (u32, u32))) -> bool {
    if from <= to {
        md >= from && md <= to
    } else {
        md >= from || md <= to // wraps the new year
    }
}

impl Slot {
    fn on_date(&self, date: chrono::NaiveDate) -> bool {
        self.dates.is_none_or(|r| in_range((date.month(), date.day()), r))
    }

    fn on(&self, weekday0: u32) -> bool {
        self.days & (1 << weekday0) != 0
    }

    /// The block started on the given day (by weekday and date) — checked for
    /// today and, for blocks running past midnight, yesterday.
    fn active(&self, date: chrono::NaiveDate, minute: u32) -> bool {
        let wd = date.weekday().num_days_from_monday();
        if self.end_min > self.start_min {
            self.on(wd) && self.on_date(date) && minute >= self.start_min && minute < self.end_min
        } else {
            let yesterday = date.pred_opt().unwrap_or(date);
            (self.on(wd) && self.on_date(date) && minute >= self.start_min)
                || (self.on(yesterday.weekday().num_days_from_monday()) && self.on_date(yesterday) && minute < self.end_min)
        }
    }
}

/// Which slot (if any) is on air at `t_ms`. Blocks with a date range (holiday
/// specials) win over regular ones; otherwise earlier-starting blocks win.
pub fn slot_at(slots: &[Slot], tz: Tz, t_ms: i64) -> Option<&Slot> {
    let local = tz.timestamp_millis_opt(t_ms).single()?;
    let date = local.date_naive();
    let minute = local.hour() * 60 + local.minute();
    let mut active = slots.iter().filter(|s| s.active(date, minute));
    let first = active.next()?;
    if first.dates.is_some() {
        return Some(first);
    }
    Some(active.find(|s| s.dates.is_some()).unwrap_or(first))
}

/// The next instant after `t_ms` where the active slot changes, within ~8 days.
pub fn next_change(slots: &[Slot], tz: Tz, t_ms: i64) -> Option<i64> {
    if slots.is_empty() {
        return None;
    }
    let current = slot_at(slots, tz, t_ms).map(|s| s.id);
    let today = tz.timestamp_millis_opt(t_ms).single()?.date_naive();
    let mut candidates: Vec<i64> = Vec::new();
    for d in 0..9 {
        let date = today + Duration::days(d);
        // Midnight too: date ranges begin and end there.
        for m in slots.iter().flat_map(|s| [s.start_min, s.end_min]).chain([0]) {
            let time = NaiveTime::from_hms_opt(m / 60 % 24, m % 60, 0)?;
            if let Some(dt) = tz.from_local_datetime(&date.and_time(time)).earliest() {
                let ms = dt.timestamp_millis();
                if ms > t_ms {
                    candidates.push(ms);
                }
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates.into_iter().find(|&c| slot_at(slots, tz, c).map(|s| s.id) != current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(id: i64, days: u8, start: u32, end: u32) -> Slot {
        Slot { id, title: String::new(), days, start_min: start, end_min: end, rule: Rule::default(), ordering: Ordering::Shuffle, dates: None }
    }

    fn ms(tz: Tz, y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        tz.with_ymd_and_hms(y, mo, d, h, mi, 0).single().unwrap().timestamp_millis()
    }

    #[test]
    fn saturday_morning_block() {
        let tz: Tz = "America/New_York".parse().unwrap();
        let slots = vec![slot(1, 1 << 5, 7 * 60, 12 * 60)];
        // 2026-10-24 is a Saturday.
        assert_eq!(slot_at(&slots, tz, ms(tz, 2026, 10, 24, 8, 0)).map(|s| s.id), Some(1));
        assert!(slot_at(&slots, tz, ms(tz, 2026, 10, 24, 12, 0)).is_none());
        assert!(slot_at(&slots, tz, ms(tz, 2026, 10, 25, 8, 0)).is_none());
        assert_eq!(next_change(&slots, tz, ms(tz, 2026, 10, 23, 20, 0)), Some(ms(tz, 2026, 10, 24, 7, 0)));
        assert_eq!(next_change(&slots, tz, ms(tz, 2026, 10, 24, 9, 0)), Some(ms(tz, 2026, 10, 24, 12, 0)));
    }

    #[test]
    fn overnight_slot_spans_midnight() {
        let tz: Tz = "UTC".parse().unwrap();
        let slots = vec![slot(2, 1 << 4, 22 * 60, 2 * 60)]; // Friday 22:00 → Sat 02:00
        assert_eq!(slot_at(&slots, tz, ms(tz, 2026, 10, 24, 1, 0)).map(|s| s.id), Some(2));
        assert!(slot_at(&slots, tz, ms(tz, 2026, 10, 25, 1, 0)).is_none());
    }

    #[test]
    fn holiday_block_runs_only_in_its_dates_and_wins() {
        let tz: Tz = "America/Chicago".parse().unwrap();
        let primetime = slot(1, 0x7f, 18 * 60, 23 * 60);
        let mut halloween = slot(2, 0x7f, 0, 0); // all day, every day…
        halloween.dates = Some(((10, 1), (10, 31))); // …in October
        let slots = vec![primetime, halloween];
        let at = |m, d, h| slot_at(&slots, tz, ms(tz, 2026, m, d, h, 0)).map(|s| s.id);
        assert_eq!(at(10, 15, 20), Some(2), "Halloween beats prime time in October");
        assert_eq!(at(10, 31, 23), Some(2));
        assert_eq!(at(11, 1, 20), Some(1), "back to normal in November");
        assert_eq!(at(9, 30, 9), None);
        // The switch is found at midnight on October 1st.
        let next = next_change(&slots, tz, ms(tz, 2026, 9, 30, 23, 30)).unwrap();
        assert_eq!(next, ms(tz, 2026, 10, 1, 0, 0));
    }

    #[test]
    fn date_ranges_can_wrap_the_new_year() {
        let tz: Tz = "UTC".parse().unwrap();
        let mut xmas = slot(3, 0x7f, 0, 0);
        xmas.dates = Some(((12, 20), (1, 5)));
        let slots = vec![xmas];
        assert!(slot_at(&slots, tz, ms(tz, 2026, 12, 25, 10, 0)).is_some());
        assert!(slot_at(&slots, tz, ms(tz, 2027, 1, 3, 10, 0)).is_some());
        assert!(slot_at(&slots, tz, ms(tz, 2027, 1, 6, 10, 0)).is_none());
        assert_eq!(parse_md("10-31"), Some((10, 31)));
        assert_eq!(parse_md("13-01"), None);
    }

    #[test]
    fn dst_transition_day() {
        // US DST ends 2026-11-01 at 02:00 local; the slot boundary still lands on local 06:00.
        let tz: Tz = "America/Chicago".parse().unwrap();
        let slots = vec![slot(3, 0x7f, 6 * 60, 9 * 60)];
        let next = next_change(&slots, tz, ms(tz, 2026, 11, 1, 0, 30)).unwrap();
        assert_eq!(next, ms(tz, 2026, 11, 1, 6, 0));
    }
}
