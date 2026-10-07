//! Time-window scheduling ("start at 23:00, stop at 07:00").

use chrono::{Datelike, NaiveDateTime, Timelike};

use crate::settings::{parse_hhmm, ScheduleSettings};

/// Whether the queue may run at local time `now`.
///
/// Windows may cross midnight (start > stop). The day-of-week filter applies
/// to the day on which the window *started*: a Monday 23:00–07:00 window still
/// allows Tuesday 02:00.
pub fn window_allows(s: &ScheduleSettings, now: NaiveDateTime) -> bool {
    if !s.enabled {
        return true;
    }
    let (Some((sh, sm)), Some((eh, em))) = (parse_hhmm(&s.start_time), parse_hhmm(&s.stop_time)) else {
        return true;
    };
    let start = sh * 60 + sm;
    let stop = eh * 60 + em;
    let t = now.hour() * 60 + now.minute();
    let today = now.weekday().num_days_from_monday() as u8;
    let yesterday = (today + 6) % 7;
    let day_ok = |d: u8| s.days.is_empty() || s.days.contains(&d);
    if start == stop {
        // A zero-length window means "all day" on the selected days.
        return day_ok(today);
    }
    if start < stop {
        t >= start && t < stop && day_ok(today)
    } else if t >= start {
        day_ok(today)
    } else if t < stop {
        day_ok(yesterday)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AfterQueueAction;

    fn sched(start: &str, stop: &str, days: &[u8]) -> ScheduleSettings {
        ScheduleSettings { enabled: true, start_time: start.into(), stop_time: stop.into(), days: days.to_vec(), after_queue: AfterQueueAction::Nothing }
    }

    fn at(s: &str) -> NaiveDateTime {
        // 2024-01-01 is a Monday.
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn disabled_always_allows() {
        let mut s = sched("23:00", "07:00", &[]);
        s.enabled = false;
        assert!(window_allows(&s, at("2024-01-01 12:00")));
    }

    #[test]
    fn same_day_window() {
        let s = sched("09:00", "17:00", &[0, 1, 2, 3, 4, 5, 6]);
        assert!(!window_allows(&s, at("2024-01-01 08:59")));
        assert!(window_allows(&s, at("2024-01-01 09:00")));
        assert!(window_allows(&s, at("2024-01-01 16:59")));
        assert!(!window_allows(&s, at("2024-01-01 17:00")));
    }

    #[test]
    fn overnight_window() {
        let s = sched("23:00", "07:00", &[0, 1, 2, 3, 4, 5, 6]);
        assert!(window_allows(&s, at("2024-01-01 23:30")));
        assert!(window_allows(&s, at("2024-01-02 06:59")));
        assert!(!window_allows(&s, at("2024-01-02 07:00")));
        assert!(!window_allows(&s, at("2024-01-02 12:00")));
    }

    #[test]
    fn overnight_respects_start_day() {
        // Only Monday's window.
        let s = sched("23:00", "07:00", &[0]);
        assert!(window_allows(&s, at("2024-01-01 23:30"))); // Mon night
        assert!(window_allows(&s, at("2024-01-02 03:00"))); // Tue early = Monday's window
        assert!(!window_allows(&s, at("2024-01-02 23:30"))); // Tue night
        assert!(!window_allows(&s, at("2024-01-01 03:00"))); // Mon early = Sunday's window
    }
}
