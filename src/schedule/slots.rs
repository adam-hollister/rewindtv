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
}

impl Slot {
    fn on(&self, weekday0: u32) -> bool {
        self.days & (1 << weekday0) != 0
    }

    fn active(&self, weekday0: u32, minute: u32) -> bool {
        if self.end_min > self.start_min {
            self.on(weekday0) && minute >= self.start_min && minute < self.end_min
        } else {
            let yesterday = (weekday0 + 6) % 7;
            (self.on(weekday0) && minute >= self.start_min) || (self.on(yesterday) && minute < self.end_min)
        }
    }
}

/// Which slot (if any) is on air at `t_ms`. Earlier-starting slots win overlaps.
pub fn slot_at<'a>(slots: &'a [Slot], tz: Tz, t_ms: i64) -> Option<&'a Slot> {
    let local = tz.timestamp_millis_opt(t_ms).single()?;
    let wd = local.weekday().num_days_from_monday();
    let minute = local.hour() * 60 + local.minute();
    slots.iter().find(|s| s.active(wd, minute))
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
        for s in slots {
            for m in [s.start_min, s.end_min] {
                let time = NaiveTime::from_hms_opt(m / 60 % 24, m % 60, 0)?;
                if let Some(dt) = tz.from_local_datetime(&date.and_time(time)).earliest() {
                    let ms = dt.timestamp_millis();
                    if ms > t_ms {
                        candidates.push(ms);
                    }
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
        Slot { id, title: String::new(), days, start_min: start, end_min: end, rule: Rule::default(), ordering: Ordering::Shuffle }
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
    fn dst_transition_day() {
        // US DST ends 2026-11-01 at 02:00 local; the slot boundary still lands on local 06:00.
        let tz: Tz = "America/Chicago".parse().unwrap();
        let slots = vec![slot(3, 0x7f, 6 * 60, 9 * 60)];
        let next = next_change(&slots, tz, ms(tz, 2026, 11, 1, 0, 30)).unwrap();
        assert_eq!(next, ms(tz, 2026, 11, 1, 6, 0));
    }
}
