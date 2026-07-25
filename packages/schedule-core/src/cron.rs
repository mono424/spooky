//! Next-fire computation for the two schedule syntaxes.
//!
//! - `cron`: standard 5-field expression (seconds field optional), evaluated
//!   in the schedule's IANA timezone (default UTC) so "every day at 02:00"
//!   means local 02:00 across DST transitions — `croner` is DST-aware.
//! - `every_ms`: fixed delay measured from the FIRE time, not from job
//!   completion (completion-anchored recurrence died with the old frontend
//!   recurring API).
//!
//! The CLI validates expressions with this same code path at lint/deploy time,
//! so an expression that deploys is an expression the engine can plan.

use chrono::{DateTime, Duration, TimeZone, Utc};
use croner::Cron;

#[derive(Debug, thiserror::Error)]
pub enum FireSpecError {
    #[error("invalid cron expression `{expr}`: {source}")]
    Cron {
        expr: String,
        source: croner::errors::CronError,
    },
    #[error("unknown timezone `{0}` (IANA name expected, e.g. `Europe/Berlin`)")]
    Timezone(String),
    #[error("`every` interval must be positive")]
    NonPositiveInterval,
    #[error("schedule must set exactly one of `cron` / `every`")]
    AmbiguousSyntax,
    #[error("no future occurrence found for cron expression `{0}`")]
    NoOccurrence(String),
}

/// Parsed firing rule. Construct via [`FireSpec::parse`].
#[derive(Debug, Clone)]
pub enum FireSpec {
    Cron {
        cron: Cron,
        expr: String,
        tz: Timezone,
    },
    Every(Duration),
}

/// Timezone a cron expression is evaluated in. With the `tz` feature (default)
/// any IANA name is accepted; without it only "UTC".
#[derive(Debug, Clone, Copy)]
pub enum Timezone {
    Utc,
    #[cfg(feature = "tz")]
    Named(chrono_tz::Tz),
}

impl FireSpec {
    /// Parse from the `_00_schedule` row's spec fields. Exactly one of
    /// `cron` / `every_ms` must be set.
    pub fn parse(
        cron_expr: Option<&str>,
        every_ms: Option<i64>,
        timezone: Option<&str>,
    ) -> Result<Self, FireSpecError> {
        match (cron_expr, every_ms) {
            (Some(expr), None) => {
                let cron = Cron::new(expr)
                    .with_seconds_optional()
                    .parse()
                    .map_err(|source| FireSpecError::Cron { expr: expr.to_string(), source })?;
                Ok(FireSpec::Cron { cron, expr: expr.to_string(), tz: parse_tz(timezone)? })
            }
            (None, Some(ms)) if ms > 0 => Ok(FireSpec::Every(Duration::milliseconds(ms))),
            (None, Some(_)) => Err(FireSpecError::NonPositiveInterval),
            _ => Err(FireSpecError::AmbiguousSyntax),
        }
    }

    /// First fire time strictly after `after`. For `every` this is simply
    /// `after + interval`; for cron it is the next matching wall-clock instant
    /// in the schedule's timezone, returned as UTC.
    pub fn next_fire_after(&self, after: DateTime<Utc>) -> Result<DateTime<Utc>, FireSpecError> {
        match self {
            FireSpec::Every(interval) => Ok(after + *interval),
            FireSpec::Cron { cron, expr, tz } => match tz {
                Timezone::Utc => cron
                    .find_next_occurrence(&after, false)
                    .map_err(|_| FireSpecError::NoOccurrence(expr.clone())),
                #[cfg(feature = "tz")]
                Timezone::Named(tz) => {
                    let local = after.with_timezone(tz);
                    cron.find_next_occurrence(&local, false)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(|_| FireSpecError::NoOccurrence(expr.clone()))
                }
            },
        }
    }
}

fn parse_tz(timezone: Option<&str>) -> Result<Timezone, FireSpecError> {
    match timezone {
        None => Ok(Timezone::Utc),
        Some(name) if name.eq_ignore_ascii_case("utc") => Ok(Timezone::Utc),
        #[cfg(feature = "tz")]
        Some(name) => name
            .parse::<chrono_tz::Tz>()
            .map(Timezone::Named)
            .map_err(|_| FireSpecError::Timezone(name.to_string())),
        #[cfg(not(feature = "tz"))]
        Some(name) => Err(FireSpecError::Timezone(name.to_string())),
    }
}

/// Parse an RFC 3339 datetime as emitted by SurrealDB's flattened-JSON values.
pub fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.with_timezone(&Utc))
}

/// `Utc.timestamp_millis_opt` without the panic-y unwrap ergonomics.
pub fn from_millis(ms: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_millis_opt(ms).single()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    #[test]
    fn every_is_fixed_delay_from_fire() {
        let spec = FireSpec::parse(None, Some(300_000), None).unwrap();
        let after = utc(2026, 7, 25, 9, 0, 0);
        assert_eq!(spec.next_fire_after(after).unwrap(), utc(2026, 7, 25, 9, 5, 0));
    }

    #[test]
    fn cron_hourly_utc() {
        let spec = FireSpec::parse(Some("0 * * * *"), None, None).unwrap();
        assert_eq!(
            spec.next_fire_after(utc(2026, 7, 25, 9, 30, 0)).unwrap(),
            utc(2026, 7, 25, 10, 0, 0)
        );
        // strictly after: sitting exactly on a boundary plans the NEXT slot
        assert_eq!(
            spec.next_fire_after(utc(2026, 7, 25, 10, 0, 0)).unwrap(),
            utc(2026, 7, 25, 11, 0, 0)
        );
    }

    #[test]
    fn cron_daily_in_berlin_is_utc_shifted() {
        let spec = FireSpec::parse(Some("0 3 * * *"), None, Some("Europe/Berlin")).unwrap();
        // July: CEST = UTC+2, so 03:00 Berlin == 01:00 UTC.
        assert_eq!(
            spec.next_fire_after(utc(2026, 7, 25, 9, 0, 0)).unwrap(),
            utc(2026, 7, 26, 1, 0, 0)
        );
    }

    #[test]
    fn cron_across_dst_fall_back_stays_at_local_time() {
        // Europe/Berlin leaves DST on 2026-10-25: clocks 03:00 → 02:00.
        // Daily 03:00 local should be 01:00 UTC before and 02:00 UTC after.
        let spec = FireSpec::parse(Some("0 3 * * *"), None, Some("Europe/Berlin")).unwrap();
        let before = spec.next_fire_after(utc(2026, 10, 24, 6, 0, 0)).unwrap();
        assert_eq!(before, utc(2026, 10, 25, 2, 0, 0)); // fires 03:00 CET (UTC+1) that same morning
        let after = spec.next_fire_after(before).unwrap();
        assert_eq!(after, utc(2026, 10, 26, 2, 0, 0));
    }

    #[test]
    fn cron_across_dst_spring_forward_skips_nonexistent_time() {
        // Europe/Berlin enters DST on 2026-03-29: 02:00 → 03:00, so 02:30
        // local does not exist that day. croner must not error or loop.
        let spec = FireSpec::parse(Some("30 2 * * *"), None, Some("Europe/Berlin")).unwrap();
        let next = spec.next_fire_after(utc(2026, 3, 28, 12, 0, 0)).unwrap();
        // Accept either DST convention (skip to next day, or shifted fire);
        // the invariant is: it advances past the gap and stays parseable.
        assert!(next > utc(2026, 3, 29, 0, 59, 0), "must be past the last pre-gap slot, got {next}");
        let following = spec.next_fire_after(next).unwrap();
        assert!(following > next);
    }

    #[test]
    fn parse_rejects_bad_inputs() {
        assert!(matches!(FireSpec::parse(None, None, None), Err(FireSpecError::AmbiguousSyntax)));
        assert!(matches!(
            FireSpec::parse(Some("0 * * * *"), Some(1000), None),
            Err(FireSpecError::AmbiguousSyntax)
        ));
        assert!(matches!(FireSpec::parse(None, Some(0), None), Err(FireSpecError::NonPositiveInterval)));
        assert!(matches!(
            FireSpec::parse(Some("not a cron"), None, None),
            Err(FireSpecError::Cron { .. })
        ));
        assert!(matches!(
            FireSpec::parse(Some("0 * * * *"), None, Some("Mars/Olympus")),
            Err(FireSpecError::Timezone(_))
        ));
    }

    #[test]
    fn six_field_seconds_cron_accepted() {
        let spec = FireSpec::parse(Some("*/30 * * * * *"), None, None).unwrap();
        let next = spec.next_fire_after(utc(2026, 7, 25, 9, 0, 10)).unwrap();
        assert_eq!(next, utc(2026, 7, 25, 9, 0, 30));
    }
}
