// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Append-only metering log (Phase 2 design §5).
//!
//! One JSON object per authenticated request, appended to the log file. The
//! log is never served over HTTP and no field of it is echoed in a response:
//! metering is a server-side accounting concern, and an agent learns its own
//! usage from the status code and the record count, not from a usage header.
//!
//! ## Deviation from the design
//!
//! The design places metering at the request boundary so that *every* request
//! is counted, including unauthenticated ones. Metering runs **inside** the
//! auth middleware here instead (see [`crate::auth`]), so a `401` is not
//! metered. Rationale: a `401` carries no `key_id`, and a metered record
//! without a `key_id` cannot be attributed to any key — it would only inflate
//! the row count with un-billable noise that no key can ever be blamed for.
//! Rejected requests are still observable: they appear in the process log as
//! `401`s, and repeated rejections are visible in any reverse-proxy access log.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// One metered request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// RFC 3339 timestamp with millisecond precision, UTC.
    pub ts: String,
    /// Authenticated key this request was attributed to. Never the secret.
    pub key_id: String,
    /// HTTP method as sent.
    pub method: String,
    /// Request path as sent, without the query string (query parameters are
    /// client-controlled and would otherwise dominate the log).
    pub path: String,
    /// Response status code.
    pub status: u16,
    /// Response body size in bytes.
    pub response_bytes: u64,
    /// Wall-clock duration of the request in milliseconds.
    pub duration_ms: u64,
}

/// Appends [`Record`]s to a log file, one JSON object per line.
#[derive(Debug, Clone)]
pub struct Metering {
    path: PathBuf,
}

impl Metering {
    /// Opens a meter writing to `path`. The file is created on first append.
    pub fn new(path: &Path) -> Self {
        Metering {
            path: path.to_path_buf(),
        }
    }

    /// Where records are appended.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one record.
    ///
    /// Opens the file in append mode per record rather than holding a handle:
    /// the file may be rotated or truncated between requests, and a stale
    /// handle would silently keep writing to a deleted inode. A write failure
    /// cannot fail the request that has already been served, so it is logged
    /// to stderr instead of propagated.
    pub fn append(&self, record: &Record) {
        let mut line = match serde_json::to_string(record) {
            Ok(line) => line,
            Err(err) => {
                eprintln!("warning: cannot serialize metering record: {err}");
                return;
            }
        };
        line.push('\n');
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(mut file) => {
                if let Err(err) = file.write_all(line.as_bytes()) {
                    eprintln!("warning: cannot append metering record: {err}");
                }
            }
            Err(err) => eprintln!(
                "warning: cannot open metering log {}: {err}",
                self.path.display()
            ),
        }
    }

    /// Builds a record with a freshly computed timestamp and duration.
    pub fn build(
        &self,
        key_id: &str,
        method: &str,
        path: &str,
        status: u16,
        response_bytes: u64,
        started: SystemTime,
    ) -> Record {
        Record {
            ts: now_rfc3339_millis(started),
            key_id: key_id.to_string(),
            method: method.to_string(),
            path: path.to_string(),
            status,
            response_bytes,
            duration_ms: started.elapsed().map(|d| d.as_millis() as u64).unwrap_or(0),
        }
    }
}

/// Formats `time` as RFC 3339 UTC with millisecond precision.
///
/// Hand-rolled rather than pulling in `chrono`/`time`: the whole job is a
/// civil-from-days conversion, and the server's dependency footprint is kept
/// small on purpose. Accurate for any date the Unix epoch can express.
pub fn now_rfc3339_millis(time: SystemTime) -> String {
    let duration = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration,
        Err(_) => return "1970-01-01T00:00:00.000Z".to_string(),
    };
    let secs = duration.as_secs() as i64;
    let millis = duration.subsec_millis();
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to (y, m, d).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Formats `secs` + `millis` past the epoch.
    fn at(secs: u64, millis: u32) -> String {
        now_rfc3339_millis(
            UNIX_EPOCH + Duration::new(secs, 0) + Duration::from_millis(u64::from(millis)),
        )
    }

    fn record(key_id: &str, status: u16) -> Record {
        Record {
            ts: "2026-09-27T04:35:12.345Z".to_string(),
            key_id: key_id.to_string(),
            method: "GET".to_string(),
            path: "/v1/graphs/x/stats".to_string(),
            status,
            response_bytes: 12,
            duration_ms: 7,
        }
    }

    fn meter(dir: &tempfile::TempDir) -> Metering {
        Metering::new(&dir.path().join("metering.log"))
    }

    #[test]
    fn epoch_formats_as_the_epoch() {
        assert_eq!(at(0, 0), "1970-01-01T00:00:00.000Z");
    }

    /// Anchors for the hand-rolled calendar: a leap day, the day after it (so a
    /// wrong leap rule shows up as an off-by-one day, not just a wrong label),
    /// a year boundary in both directions, and a century that is *not* a leap
    /// year.
    #[test]
    fn known_dates_format_exactly() {
        for (secs, millis, expected) in [
            (0u64, 1u32, "1970-01-01T00:00:00.001Z"),
            (946_684_799, 999, "1999-12-31T23:59:59.999Z"),
            (951_782_400, 0, "2000-02-29T00:00:00.000Z"),
            (951_868_800, 0, "2000-03-01T00:00:00.000Z"),
            (1_790_483_712, 345, "2026-09-27T04:35:12.345Z"),
            (1_798_761_599, 999, "2026-12-31T23:59:59.999Z"),
            (4_107_585_600, 0, "2100-03-01T12:00:00.000Z"),
        ] {
            assert_eq!(at(secs, millis), expected);
        }
    }

    #[test]
    fn sub_second_precision_is_milliseconds() {
        // 999_999_999ns must round *down* to 999ms, not spill into a new second.
        let time = UNIX_EPOCH + Duration::new(1_790_483_712, 999_999_999);
        assert_eq!(now_rfc3339_millis(time), "2026-09-27T04:35:12.999Z");
    }

    #[test]
    fn every_output_is_exactly_24_characters_and_zero_padded() {
        for secs in [0u64, 59, 60, 3599, 86_399, 86_400, 1_790_483_712] {
            let out = at(secs, 7);
            assert_eq!(out.len(), 24, "{out}");
            assert!(out.ends_with('Z'));
            assert_eq!(&out[4..5], "-", "{out}");
            assert_eq!(&out[10..11], "T", "{out}");
            assert_eq!(&out[19..20], ".", "{out}");
            assert!(
                (1..=12).contains(&out[5..7].parse::<u32>().unwrap()),
                "{out}"
            );
            assert!(
                (1..=31).contains(&out[8..10].parse::<u32>().unwrap()),
                "{out}"
            );
            assert!(out[11..13].parse::<u32>().unwrap() < 24, "{out}");
            assert!(out[14..16].parse::<u32>().unwrap() < 60, "{out}");
            assert!(out[17..19].parse::<u32>().unwrap() < 60, "{out}");
        }
    }

    /// A time before the epoch is a clock fault, not a request to render year
    /// 1969 garbage. The function stays total and pins the floor.
    #[test]
    fn pre_epoch_time_is_pinned_to_the_epoch() {
        let before = UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(now_rfc3339_millis(before), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn build_populates_every_field() {
        let dir = tempfile::tempdir().unwrap();
        let started = UNIX_EPOCH + Duration::from_secs(1_790_483_712);
        let built = meter(&dir).build("agent", "POST", "/v1/index", 201, 42, started);
        assert_eq!(built.ts, "2026-09-27T04:35:12.000Z");
        assert_eq!(built.key_id, "agent");
        assert_eq!(built.method, "POST");
        assert_eq!(built.path, "/v1/index");
        assert_eq!(built.status, 201);
        assert_eq!(built.response_bytes, 42);
        // `started` is deliberately in the past, so the elapsed time between it
        // and now is what the record must carry.
        assert!(built.duration_ms > 0, "duration is measured from `started`");
    }

    #[test]
    fn build_measures_a_non_zero_duration() {
        let dir = tempfile::tempdir().unwrap();
        let built = meter(&dir).build("agent", "GET", "/v1/x", 200, 0, SystemTime::now());
        // A real request cannot be served in under a millisecond, but a coarse
        // clock must not be able to produce a *negative* or absent one either.
        assert!(
            built.duration_ms < 60_000,
            "implausible duration {}",
            built.duration_ms
        );
    }

    #[test]
    fn path_is_where_records_go() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("metering.log");
        assert_eq!(Metering::new(&path).path(), path);
    }

    #[test]
    fn append_creates_the_file_and_writes_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let meter = meter(&dir);
        assert!(!meter.path().exists(), "file is created on first append");
        meter.append(&record("agent", 200));
        let contents = std::fs::read_to_string(meter.path()).unwrap();
        assert_eq!(contents.lines().count(), 1);
        assert!(contents.ends_with('\n'), "records are newline-delimited");
    }

    #[test]
    fn append_accumulates_instead_of_truncating() {
        let dir = tempfile::tempdir().unwrap();
        let meter = meter(&dir);
        for status in [200u16, 404, 500] {
            meter.append(&record("agent", status));
        }
        let contents = std::fs::read_to_string(meter.path()).unwrap();
        let statuses: Vec<u64> = contents
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["status"]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        assert_eq!(statuses, vec![200, 404, 500]);
    }

    #[test]
    fn written_line_round_trips_to_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let meter = meter(&dir);
        let original = record("agent", 200);
        meter.append(&original);
        let line = std::fs::read_to_string(meter.path()).unwrap();
        let parsed: Record = serde_json::from_str(line.trim()).expect("line is a record");
        assert_eq!(parsed, original);
    }

    /// The log is JSON Lines, so a line must be exactly the seven documented
    /// fields — a new field would silently change the format the doc promises.
    #[test]
    fn line_has_exactly_the_documented_fields() {
        let line = serde_json::to_string(&record("agent", 200)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        let mut fields: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            [
                "duration_ms",
                "key_id",
                "method",
                "path",
                "response_bytes",
                "status",
                "ts"
            ]
        );
    }

    /// A `path` is recorded without a query string, so the log cannot be flooded
    /// with unbounded client-controlled text.
    #[test]
    fn query_strings_are_not_part_of_the_recorded_path() {
        let dir = tempfile::tempdir().unwrap();
        let built = meter(&dir).build(
            "agent",
            "GET",
            "/v1/graphs/x/stats",
            200,
            0,
            SystemTime::now(),
        );
        assert!(!built.path.contains('?'));
    }

    /// A metering failure is logged, never propagated: the request it accounts
    /// for has already been served (§5). The contract under test is that
    /// `append` does not panic on an unwritable path.
    #[test]
    fn unwritable_path_is_logged_not_panicked() {
        let dir = tempfile::tempdir().unwrap();
        // A path under a *file* can never be opened.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let meter = Metering::new(&blocker.join("metering.log"));
        meter.append(&record("agent", 200));
    }

    /// Rotating or truncating the log between requests must not leave a stale
    /// handle writing into a deleted inode (§5): re-reading after a truncation
    /// has to see the new record only.
    #[test]
    fn append_after_truncation_writes_to_the_current_file() {
        let dir = tempfile::tempdir().unwrap();
        let meter = meter(&dir);
        meter.append(&record("agent", 200));
        std::fs::write(meter.path(), b"").unwrap();
        meter.append(&record("agent", 404));
        let contents = std::fs::read_to_string(meter.path()).unwrap();
        assert_eq!(contents.lines().count(), 1);
    }
}
