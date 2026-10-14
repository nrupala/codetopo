// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! The Paddle wire: signature verification and event payloads.
//!
//! Signature scheme (Paddle Billing, and the estate's Conduit receiver
//! before this): the `Paddle-Signature` header carries
//! `ts=<unix>;h1=<hex HMAC-SHA256>` — several `h1` values may appear
//! during secret rotation — where the signed input is
//! `"<ts>:<raw body>"` keyed with the notification endpoint's secret.
//! Verification runs over the **raw** body, before any JSON is parsed:
//! re-serialising parsed JSON would not reproduce the signed bytes.
//! Comparison is constant-time, and the secret is never logged.
//!
//! No timestamp freshness window is enforced: the HMAC is the
//! authentication, and Paddle retries re-deliver the original signed
//! payload — a tight window would reject legitimate retries while
//! stopping nothing the event-id idempotency in [`crate::service`]
//! does not already stop.

use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// True when `header` is a valid `Paddle-Signature` for `raw_body`
/// under `secret`. Any well-formed `h1` candidate may match (secret
/// rotation); malformed candidates are skipped, and a header with no
/// usable `ts` or `h1` never verifies.
pub fn verify_signature(secret: &str, header: &str, raw_body: &[u8]) -> bool {
    let mut timestamp: Option<&str> = None;
    let mut candidates: Vec<&str> = Vec::new();
    for part in header.split(';') {
        let part = part.trim();
        if let Some(ts) = part.strip_prefix("ts=") {
            timestamp = Some(ts);
        } else if let Some(h1) = part.strip_prefix("h1=") {
            candidates.push(h1);
        }
    }
    let Some(ts) = timestamp else {
        return false;
    };
    if ts.is_empty() || !ts.bytes().all(|b| b.is_ascii_digit()) || candidates.is_empty() {
        return false;
    }
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(ts.as_bytes());
    mac.update(b":");
    mac.update(raw_body);
    let expected = mac.finalize().into_bytes();
    candidates.iter().any(|h1| {
        hex_decode(h1).is_some_and(|decoded| {
            decoded.len() == expected.len()
                && bool::from(decoded.as_slice().ct_eq(expected.as_slice()))
        })
    })
}

/// HMAC-SHA256 as raw bytes; exposed for the RFC 4231 test vectors.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Decodes hex (either case) to bytes; `None` on odd length or a
/// non-hex character.
pub fn hex_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.as_chunks::<2>().0 {
        out.push((hex_value(pair[0])? << 4) | hex_value(pair[1])?);
    }
    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Encodes bytes as lowercase hex; used by tests to build headers.
#[cfg(test)]
pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One webhook delivery, as Paddle posts it. `data` stays untyped
/// here: each event family reads the fields it needs, and an event we
/// do not act on never has to fit a schema.
#[derive(Debug, Clone, Deserialize)]
pub struct WebhookEvent {
    /// Paddle's unique id for this event — the idempotency key.
    pub event_id: String,
    /// e.g. `subscription.activated`.
    pub event_type: String,
    /// RFC 3339 delivery time; the ordering guard's clock.
    pub occurred_at: String,
    /// The event payload (a subscription or a transaction).
    pub data: serde_json::Value,
}

/// The subscription fields the bridge acts on.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SubscriptionData {
    /// `sub_…` — the entitlement's identity and the first key's id.
    pub id: String,
    /// `active`, `trialing`, `past_due`, `canceled`, `paused`.
    pub status: Option<String>,
    /// Line items; the codetopo price is matched against these.
    #[serde(default)]
    pub items: Vec<LineItem>,
    /// A pending cancellation (or pause/resume), if one is scheduled.
    pub scheduled_change: Option<ScheduledChange>,
    /// The current billing period, whose end is the fallback
    /// cancellation-effective time.
    pub current_billing_period: Option<BillingPeriod>,
    /// When the subscription was canceled, if it has been.
    pub canceled_at: Option<String>,
}

/// The transaction fields the bridge acts on.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TransactionData {
    /// `txn_…` — the checkout's claim token.
    pub id: String,
    /// `completed` on the event this type is read from.
    pub status: Option<String>,
    /// The subscription this transaction billed, if any.
    pub subscription_id: Option<String>,
    /// Line items; the codetopo price is matched against these.
    #[serde(default)]
    pub items: Vec<LineItem>,
}

/// One priced line item on a subscription or transaction.
#[derive(Debug, Clone, Deserialize)]
pub struct LineItem {
    /// The price reference; some item shapes omit it.
    pub price: Option<PriceRef>,
}

/// A price reference: just its id.
#[derive(Debug, Clone, Deserialize)]
pub struct PriceRef {
    /// `pri_…`.
    pub id: String,
}

/// A scheduled subscription change (Paddle's cancellation vehicle).
#[derive(Debug, Clone, Deserialize)]
pub struct ScheduledChange {
    /// `cancel`, `pause`, or `resume`.
    pub action: String,
    /// RFC 3339 time the change takes effect.
    pub effective_from: String,
}

/// A billing period's bounds.
#[derive(Debug, Clone, Deserialize)]
pub struct BillingPeriod {
    /// RFC 3339 period end.
    pub ends_at: String,
}

impl WebhookEvent {
    /// `occurred_at` as Unix seconds; `None` when unparseable, in
    /// which case the caller falls back to its own clock.
    pub fn occurred_unix(&self) -> Option<i64> {
        parse_rfc3339(&self.occurred_at)
    }
}

/// Parses an RFC 3339 timestamp (`Z` or a numeric offset, optional
/// fractional seconds) to Unix seconds. Hand-rolled — the bridge needs
/// parsing only (the server crate formats), and one small civil-date
/// conversion is cheaper than a time dependency.
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let year = digits(bytes, 0, 4)?;
    let month = digits(bytes, 5, 2)?;
    let day = digits(bytes, 8, 2)?;
    if bytes[4] != b'-' || bytes[7] != b'-' || (bytes[10] != b'T' && bytes[10] != b't') {
        return None;
    }
    let hour = digits(bytes, 11, 2)?;
    let minute = digits(bytes, 14, 2)?;
    let second = digits(bytes, 17, 2)?;
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // Fractional seconds, if present, are skipped; the remainder must
    // be `Z` or a numeric offset.
    let mut idx = 19;
    if bytes.get(idx) == Some(&b'.') {
        idx += 1;
        while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
            idx += 1;
        }
    }
    let offset_secs: i64 = match bytes.get(idx) {
        Some(b'Z') | Some(b'z') => 0,
        Some(sign @ (b'+' | b'-')) => {
            let oh = digits(bytes, idx + 1, 2)?;
            if bytes.get(idx + 3) != Some(&b':') {
                return None;
            }
            let om = digits(bytes, idx + 4, 2)?;
            let magnitude = oh * 3600 + om * 60;
            if *sign == b'+' {
                magnitude
            } else {
                -magnitude
            }
        }
        _ => return None,
    };
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset_secs)
}

/// Reads `len` ASCII digits at `start` as a non-negative integer.
fn digits(bytes: &[u8], start: usize, len: usize) -> Option<i64> {
    let mut value: i64 = 0;
    for i in start..start + len {
        let b = *bytes.get(i)?;
        if !b.is_ascii_digit() {
            return None;
        }
        value = value * 10 + i64::from(b - b'0');
    }
    Some(value)
}

/// Howard Hinnant's `days_from_civil`: days since 1970-01-01.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-secret-not-a-real-one";
    const BODY: &[u8] = br#"{"event_id":"evt_1","event_type":"subscription.activated"}"#;

    fn signed_header(secret: &str, ts: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac");
        mac.update(ts.as_bytes());
        mac.update(b":");
        mac.update(body);
        format!("ts={ts};h1={}", hex_encode(&mac.finalize().into_bytes()))
    }

    #[test]
    fn a_correct_signature_verifies() {
        let header = signed_header(SECRET, "1729000000", BODY);
        assert!(verify_signature(SECRET, &header, BODY));
    }

    #[test]
    fn wrong_secret_tampered_body_and_garbage_headers_fail() {
        let header = signed_header(SECRET, "1729000000", BODY);
        assert!(!verify_signature("other-secret", &header, BODY));
        assert!(!verify_signature(SECRET, &header, b"{}"));
        assert!(!verify_signature(SECRET, "", BODY));
        assert!(!verify_signature(SECRET, "h1=abcd", BODY));
        assert!(!verify_signature(SECRET, "ts=1729000000", BODY));
        assert!(!verify_signature(SECRET, "ts=now;h1=abcd", BODY));
        // A truncated (16-byte) hex h1 must not verify against a prefix.
        let full = signed_header(SECRET, "1729000000", BODY);
        let short = format!("ts=1729000000;h1={}", &full[full.len() - 32..]);
        assert!(!verify_signature(SECRET, &short, BODY));
    }

    #[test]
    fn any_h1_of_several_may_match_secret_rotation() {
        let good = signed_header(SECRET, "1729000000", BODY);
        let h1 = good.split("h1=").nth(1).expect("h1");
        let header = format!("ts=1729000000;h1={};h1={h1}", "00".repeat(32));
        assert!(verify_signature(SECRET, &header, BODY));
    }

    #[test]
    fn hmac_matches_rfc4231_test_case_2() {
        // Key "Jefe", data "what do ya want for nothing?".
        let out = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex_encode(&out),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn rfc3339_parsing_handles_z_offsets_and_fractions() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-10-14T07:00:00Z"), Some(1_791_961_200));
        // Offset form of the same instant.
        assert_eq!(
            parse_rfc3339("2026-10-14T09:00:00+02:00"),
            parse_rfc3339("2026-10-14T07:00:00Z")
        );
        assert_eq!(
            parse_rfc3339("2026-10-14T07:00:00.123Z"),
            parse_rfc3339("2026-10-14T07:00:00Z")
        );
        assert_eq!(parse_rfc3339("not a date"), None);
        assert_eq!(parse_rfc3339("2026-13-14T07:00:00Z"), None);
        assert_eq!(parse_rfc3339("2026-10-14 07:00:00Z"), None);
    }
}
