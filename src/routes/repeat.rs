//! Cards that come back: the rules `:repeat` takes, and the date a finished
//! card comes back on.
//!
//! Steps are counted on the calendar in the configured timezone, not in
//! seconds, so a card due at noon stays due at noon across a change to
//! daylight saving time, and "monthly" means the same day of the next month —
//! the last day of it when the month is shorter.

use chrono::{DateTime, Days, Months, NaiveDateTime, TimeZone};
use chrono_tz::Tz;

/// The longest step a rule takes, so a slip of the keyboard (`+2000w`) is
/// refused rather than parked decades ahead.
const MAX_STEP: u32 = 999;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Days(u32),
    Months(u32),
}

impl Repeat {
    /// `daily`, `weekly`, `monthly`, `yearly`, `+3d`, `+2w`, `+6m`, `+1y`, in
    /// any case. Anything else is not a rule.
    pub fn parse(text: &str) -> Option<Repeat> {
        let text = text.trim().to_ascii_lowercase();
        let rule = match text.as_str() {
            "daily" => Repeat::Days(1),
            "weekly" => Repeat::Days(7),
            "monthly" => Repeat::Months(1),
            "yearly" => Repeat::Months(12),
            _ => {
                let step = text.strip_prefix('+')?;
                let (split, unit) = step.char_indices().last()?;
                let n: u32 = step[..split].parse().ok()?;
                if n == 0 || n > MAX_STEP {
                    return None;
                }
                match unit {
                    'd' => Repeat::Days(n),
                    'w' => Repeat::Days(n * 7),
                    'm' => Repeat::Months(n),
                    'y' => Repeat::Months(n * 12),
                    _ => return None,
                }
            }
        };
        Some(rule)
    }

    /// The rule as it is stored and shown: the word where there is one, the
    /// largest unit that fits otherwise, so `+14d` and `+2w` are one rule.
    pub fn name(self) -> String {
        match self {
            Repeat::Days(1) => "daily".into(),
            Repeat::Days(7) => "weekly".into(),
            Repeat::Days(n) if n % 7 == 0 => format!("+{}w", n / 7),
            Repeat::Days(n) => format!("+{n}d"),
            Repeat::Months(1) => "monthly".into(),
            Repeat::Months(12) => "yearly".into(),
            Repeat::Months(n) if n % 12 == 0 => format!("+{}y", n / 12),
            Repeat::Months(n) => format!("+{n}m"),
        }
    }

    /// `k` steps after `from`, on the local calendar.
    fn after(self, from: NaiveDateTime, k: u32) -> Option<NaiveDateTime> {
        match self {
            Repeat::Days(n) => from.checked_add_days(Days::new(u64::from(n) * u64::from(k))),
            // Always counted from the first date rather than step by step, so
            // a card due on the 31st that once landed on the 28th still comes
            // back on the 31st of the months that have one.
            Repeat::Months(n) => from.checked_add_months(Months::new(n.checked_mul(k)?)),
        }
    }

    /// When a card due at `due_at` comes back: the first step after it that
    /// falls on `today_start` or later, so finishing a card that is weeks
    /// late does not leave a copy that is late already.
    pub fn next(self, tz: Tz, due_at: i64, today_start: i64) -> Option<i64> {
        let from = DateTime::from_timestamp(due_at, 0)?
            .with_timezone(&tz)
            .naive_local();
        let mut k = 1;
        loop {
            let local = self.after(from, k)?;
            // The hour a change to summer time skips does not exist; the clock
            // has passed it, so the card is due the hour after.
            let at = tz
                .from_local_datetime(&local)
                .earliest()
                .or_else(|| {
                    tz.from_local_datetime(&(local + chrono::Duration::hours(1)))
                        .earliest()
                })?
                .timestamp();
            if at >= today_start {
                return Some(at);
            }
            k = k.checked_add(1)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Repeat;
    use chrono::{NaiveDate, TimeZone};
    use chrono_tz::Tz;

    const BERLIN: Tz = chrono_tz::Europe::Berlin;

    fn at(y: i32, m: u32, d: u32, h: u32) -> i64 {
        BERLIN
            .with_ymd_and_hms(y, m, d, h, 0, 0)
            .earliest()
            .unwrap()
            .timestamp()
    }

    fn day(y: i32, m: u32, d: u32) -> i64 {
        crate::notify::day_bounds(BERLIN, NaiveDate::from_ymd_opt(y, m, d).unwrap()).0
    }

    #[test]
    fn rules_read_in_words_and_steps_and_are_stored_one_way() {
        assert_eq!(Repeat::parse("Weekly"), Some(Repeat::Days(7)));
        assert_eq!(Repeat::parse(" +3d "), Some(Repeat::Days(3)));
        assert_eq!(
            Repeat::parse("+14d").map(Repeat::name).as_deref(),
            Some("+2w")
        );
        assert_eq!(
            Repeat::parse("+1w").map(Repeat::name).as_deref(),
            Some("weekly")
        );
        assert_eq!(
            Repeat::parse("+12m").map(Repeat::name).as_deref(),
            Some("yearly")
        );
        assert_eq!(
            Repeat::parse("+3m").map(Repeat::name).as_deref(),
            Some("+3m")
        );
        for wrong in [
            "",
            "+",
            "+0d",
            "+d",
            "+3x",
            "3d",
            "fortnightly",
            "+1000d",
            "+-1d",
            "+3é",
        ] {
            assert_eq!(Repeat::parse(wrong), None, "{wrong}");
        }
    }

    #[test]
    fn each_rule_steps_on_the_calendar() {
        let due = at(2026, 10, 20, 12);
        let today = day(2026, 10, 20);
        let next = |rule: &str| Repeat::parse(rule).unwrap().next(BERLIN, due, today);
        assert_eq!(next("daily"), Some(at(2026, 10, 21, 12)));
        assert_eq!(next("weekly"), Some(at(2026, 10, 27, 12)));
        assert_eq!(next("+3d"), Some(at(2026, 10, 23, 12)));
        assert_eq!(next("+2w"), Some(at(2026, 11, 3, 12)));
        assert_eq!(next("monthly"), Some(at(2026, 11, 20, 12)));
        assert_eq!(next("+6m"), Some(at(2027, 4, 20, 12)));
        assert_eq!(next("yearly"), Some(at(2027, 10, 20, 12)));
    }

    /// Noon stays noon when the clocks go back on 25 October.
    #[test]
    fn a_step_across_daylight_saving_keeps_the_time_of_day() {
        let due = at(2026, 10, 24, 12);
        assert_eq!(
            Repeat::Days(1).next(BERLIN, due, day(2026, 10, 24)),
            Some(at(2026, 10, 25, 12))
        );
    }

    #[test]
    fn the_end_of_a_month_clamps_and_does_not_drift() {
        let due = at(2026, 1, 31, 12);
        let monthly = Repeat::Months(1);
        assert_eq!(
            monthly.next(BERLIN, due, day(2026, 1, 31)),
            Some(at(2026, 2, 28, 12))
        );
        // Rolled forward from the 31st, March has one again.
        assert_eq!(
            monthly.next(BERLIN, due, day(2026, 3, 1)),
            Some(at(2026, 3, 31, 12))
        );
        assert_eq!(
            Repeat::Months(12).next(BERLIN, at(2028, 2, 29, 12), day(2028, 2, 29)),
            Some(at(2029, 2, 28, 12))
        );
    }

    /// A card finished weeks late comes back on its next date from today on,
    /// not on one that has passed already.
    #[test]
    fn a_late_card_comes_back_today_or_later() {
        let due = at(2026, 9, 1, 12);
        assert_eq!(
            Repeat::Days(7).next(BERLIN, due, day(2026, 10, 20)),
            Some(at(2026, 10, 20, 12))
        );
        assert_eq!(
            Repeat::Days(7).next(BERLIN, due, day(2026, 10, 21)),
            Some(at(2026, 10, 27, 12))
        );
    }
}
