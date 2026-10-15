// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! The Stripe wire: signature verification and event payloads.
//!
//! Signature scheme (Stripe webhooks): the `Stripe-Signature` header
//! carries `t=<unix>,v1=<hex HMAC-SHA256>` — several `v1` values may
//! appear during secret rotation — where the signed input is
//! `"<t>.<raw body>"` keyed with the endpoint's signing secret.
//! Verification runs over the **raw** body, before any JSON is parsed:
//! re-serialising parsed JSON would not reproduce the signed bytes.
//! Comparison is constant-time, and the secret is never logged.
//!
//! The timestamp is held to Stripe's own tolerance — five minutes
//! either side of the receiver's clock. A captured payload therefore
//! cannot be replayed hours later, while Stripe's retries (re-signed
//! on every attempt) are unaffected; the event-id idempotency in
//! [`crate::service`] remains the second wall.

use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// How far a signature timestamp may sit from the receiver's clock,
/// in seconds: Stripe's documented tolerance.
pub const SIGNATURE_TOLERANCE_SECS: i64 = 300;

/// True when `header` is a valid `Stripe-Signature` for `raw_body`
/// under `secret` at receiver time `now`. Any well-formed `v1`
/// candidate may match (secret rotation); malformed candidates are
/// skipped, and a header with no usable `t` or `v1`, or a timestamp
/// outside the tolerance, never verifies.
pub fn verify_signature(secret: &str, header: &str, raw_body: &[u8], now: i64) -> bool {
    let mut timestamp: Option<&str> = None;
    let mut candidates: Vec<&str> = Vec::new();
    for part in header.split(',') {
        let part = part.trim();
        if let Some(t) = part.strip_prefix("t=") {
            timestamp = Some(t);
        } else if let Some(v1) = part.strip_prefix("v1=") {
            candidates.push(v1);
        }
    }
    let Some(ts_text) = timestamp else {
        return false;
    };
    if ts_text.is_empty() || !ts_text.bytes().all(|b| b.is_ascii_digit()) || candidates.is_empty()
    {
        return false;
    }
    let Ok(ts) = ts_text.parse::<i64>() else {
        return false;
    };
    if (now - ts).abs() > SIGNATURE_TOLERANCE_SECS {
        return false;
    }
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    // The timestamp is signed exactly as it appears in the header,
    // never re-formatted from the parsed integer.
    mac.update(ts_text.as_bytes());
    mac.update(b".");
    mac.update(raw_body);
    let expected = mac.finalize().into_bytes();
    candidates.iter().any(|v1| {
        hex_decode(v1).is_some_and(|decoded| {
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

/// One webhook delivery, as Stripe posts it: an envelope whose
/// `data.object` carries the payload. The envelope is unwrapped at
/// parse time, so `data` here is the object itself (a subscription,
/// a checkout session, an invoice). The object stays untyped: each
/// event family reads the fields it needs, and an event we do not
/// act on never has to fit a schema.
#[derive(Debug, Clone)]
pub struct WebhookEvent {
    /// Stripe's unique id for this event (`evt_…`) — the idempotency
    /// key.
    pub event_id: String,
    /// e.g. `customer.subscription.updated`.
    pub event_type: String,
    /// Unix seconds the event was created at Stripe; the ordering
    /// guard's clock.
    pub created: i64,
    /// The event's object, unwrapped from `data.object`.
    pub data: serde_json::Value,
}

impl<'de> Deserialize<'de> for WebhookEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Envelope {
            id: String,
            #[serde(rename = "type")]
            event_type: String,
            created: i64,
            data: EnvelopeData,
        }
        #[derive(Deserialize)]
        struct EnvelopeData {
            object: serde_json::Value,
        }
        let envelope = Envelope::deserialize(deserializer)?;
        Ok(WebhookEvent {
            event_id: envelope.id,
            event_type: envelope.event_type,
            created: envelope.created,
            data: envelope.data.object,
        })
    }
}

impl WebhookEvent {
    /// The event's creation time as Unix seconds; always present in
    /// a Stripe envelope.
    pub fn occurred_unix(&self) -> Option<i64> {
        Some(self.created)
    }
}

/// The subscription fields the bridge acts on.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SubscriptionData {
    /// `sub_…` — the entitlement's identity and the first key's id.
    pub id: String,
    /// `active`, `trialing`, `past_due`, `unpaid`, `canceled`,
    /// `incomplete`, `paused`.
    pub status: Option<String>,
    /// Line items; the codetopo price is matched against these.
    #[serde(default)]
    pub items: ItemList,
    /// Unix seconds the current billing period ends. Newer Stripe
    /// API versions carry this on the items instead;
    /// [`SubscriptionData::period_end`] reads either.
    pub current_period_end: Option<i64>,
    /// True when a cancellation is scheduled for the period end.
    #[serde(default)]
    pub cancel_at_period_end: bool,
    /// Unix seconds a scheduled cancellation takes effect, if set.
    pub cancel_at: Option<i64>,
    /// Unix seconds the subscription was canceled, if it has been.
    pub canceled_at: Option<i64>,
}

impl SubscriptionData {
    /// When the current period ends: the subscription-level field,
    /// or the first item's where newer API versions put it.
    pub fn period_end(&self) -> Option<i64> {
        self.current_period_end.or_else(|| {
            self.items
                .data
                .iter()
                .filter_map(|item| item.current_period_end)
                .next()
        })
    }
}

/// The checkout-session fields the bridge acts on.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CheckoutSessionData {
    /// `cs_…` — the claim flow's transaction handle.
    pub id: String,
    /// `subscription` on the sessions that sell entitlements.
    pub mode: Option<String>,
    /// `paid` once the money has landed. A session can complete
    /// while an asynchronous payment is still processing; only
    /// `paid` creates anything.
    pub payment_status: Option<String>,
    /// The subscription the checkout created, if any.
    pub subscription: Option<String>,
}

/// The invoice fields the bridge acts on.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct InvoiceData {
    /// `in_…`.
    pub id: String,
    /// The billed subscription, on API versions that carry it at
    /// the top level.
    pub subscription: Option<String>,
    /// Where newer API versions nest the subscription link.
    pub parent: Option<InvoiceParent>,
}

impl InvoiceData {
    /// The subscription this invoice bills, read from either of the
    /// two shapes Stripe's API versions have used.
    pub fn subscription_id(&self) -> Option<&str> {
        if let Some(subscription) = self.subscription.as_deref() {
            return Some(subscription);
        }
        self.parent
            .as_ref()
            .and_then(|parent| parent.subscription_details.as_ref())
            .and_then(|details| details.subscription.as_deref())
    }
}

/// An invoice's parentage block (newer API versions).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct InvoiceParent {
    /// The subscription details inside the parentage block.
    pub subscription_details: Option<SubscriptionDetails>,
}

/// The subscription link inside an invoice's parentage block.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SubscriptionDetails {
    /// The billed subscription id.
    pub subscription: Option<String>,
}

/// A subscription's line items, as Stripe nests them.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ItemList {
    /// The items themselves.
    #[serde(default)]
    pub data: Vec<LineItem>,
}

/// One priced line item on a subscription.
#[derive(Debug, Clone, Deserialize)]
pub struct LineItem {
    /// The price reference; some item shapes omit it.
    pub price: Option<PriceRef>,
    /// Per-item period end, on newer API versions.
    pub current_period_end: Option<i64>,
}

/// A price reference: just its id.
#[derive(Debug, Clone, Deserialize)]
pub struct PriceRef {
    /// `price_…`.
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "whsec_test_secret_not_a_real_one";
    const BODY: &[u8] = br#"{"id":"evt_1","type":"customer.subscription.updated"}"#;
    const NOW: i64 = 1_800_000_000;

    fn signed_header(secret: &str, t: i64, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac");
        mac.update(t.to_string().as_bytes());
        mac.update(b".");
        mac.update(body);
        format!("t={t},v1={}", hex_encode(&mac.finalize().into_bytes()))
    }

    #[test]
    fn a_correct_signature_verifies() {
        let header = signed_header(SECRET, NOW, BODY);
        assert!(verify_signature(SECRET, &header, BODY, NOW));
        // A delivery signed two minutes ago verifies on arrival.
        let header = signed_header(SECRET, NOW - 120, BODY);
        assert!(verify_signature(SECRET, &header, BODY, NOW));
    }

    #[test]
    fn wrong_secret_tampered_body_and_garbage_headers_fail() {
        let header = signed_header(SECRET, NOW, BODY);
        assert!(!verify_signature("other-secret", &header, BODY, NOW));
        assert!(!verify_signature(SECRET, &header, b"{}", NOW));
        assert!(!verify_signature(SECRET, "", BODY, NOW));
        assert!(!verify_signature(SECRET, "v1=abcd", BODY, NOW));
        assert!(!verify_signature(SECRET, &format!("t={NOW}"), BODY, NOW));
        assert!(!verify_signature(SECRET, "t=now,v1=abcd", BODY, NOW));
        // A truncated (16-byte) hex v1 must not verify against a prefix.
        let full = signed_header(SECRET, NOW, BODY);
        let v1 = full.split("v1=").nth(1).expect("v1");
        let short = format!("t={NOW},v1={}", &v1[..32]);
        assert!(!verify_signature(SECRET, &short, BODY, NOW));
    }

    #[test]
    fn stale_and_far_future_timestamps_fail() {
        // Older than the five-minute tolerance: a replayed capture.
        let stale = signed_header(SECRET, NOW - SIGNATURE_TOLERANCE_SECS - 1, BODY);
        assert!(!verify_signature(SECRET, &stale, BODY, NOW));
        // Implausibly far in the future fails the same way.
        let future = signed_header(SECRET, NOW + SIGNATURE_TOLERANCE_SECS + 1, BODY);
        assert!(!verify_signature(SECRET, &future, BODY, NOW));
        // The edge of the tolerance still verifies.
        let edge = signed_header(SECRET, NOW - SIGNATURE_TOLERANCE_SECS, BODY);
        assert!(verify_signature(SECRET, &edge, BODY, NOW));
    }

    #[test]
    fn any_v1_of_several_may_match_secret_rotation() {
        let good = signed_header(SECRET, NOW, BODY);
        let v1 = good.split("v1=").nth(1).expect("v1");
        let header = format!("t={NOW},v1={},v0={},v1={v1}", "00".repeat(32), "ff".repeat(32));
        assert!(verify_signature(SECRET, &header, BODY, NOW));
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
    fn the_envelope_unwraps_data_object() {
        let event: WebhookEvent = serde_json::from_slice(
            br#"{
                "id": "evt_9",
                "type": "invoice.payment_failed",
                "created": 1800000123,
                "data": { "object": { "id": "in_9", "subscription": "sub_9" } }
            }"#,
        )
        .expect("envelope parses");
        assert_eq!(event.event_id, "evt_9");
        assert_eq!(event.event_type, "invoice.payment_failed");
        assert_eq!(event.occurred_unix(), Some(1_800_000_123));
        assert_eq!(event.data["id"], "in_9");
    }

    #[test]
    fn period_end_reads_the_item_level_field_when_the_top_level_is_absent() {
        let data: SubscriptionData = serde_json::from_value(serde_json::json!({
            "id": "sub_1",
            "status": "active",
            "items": { "data": [{ "price": { "id": "price_1" }, "current_period_end": 1800100000 }] },
        }))
        .expect("subscription parses");
        assert_eq!(data.period_end(), Some(1_800_100_000));
        let data: SubscriptionData = serde_json::from_value(serde_json::json!({
            "id": "sub_1",
            "status": "active",
            "current_period_end": 1800200000,
            "items": { "data": [{ "price": { "id": "price_1" }, "current_period_end": 1800100000 }] },
        }))
        .expect("subscription parses");
        assert_eq!(data.period_end(), Some(1_800_200_000), "top level wins");
    }

    #[test]
    fn invoice_subscription_id_reads_both_api_shapes() {
        let legacy: InvoiceData = serde_json::from_value(serde_json::json!({
            "id": "in_1",
            "subscription": "sub_legacy",
        }))
        .expect("invoice parses");
        assert_eq!(legacy.subscription_id(), Some("sub_legacy"));
        let nested: InvoiceData = serde_json::from_value(serde_json::json!({
            "id": "in_2",
            "parent": { "subscription_details": { "subscription": "sub_nested" } },
        }))
        .expect("invoice parses");
        assert_eq!(nested.subscription_id(), Some("sub_nested"));
        let none: InvoiceData =
            serde_json::from_value(serde_json::json!({ "id": "in_3" })).expect("invoice parses");
        assert_eq!(none.subscription_id(), None);
    }
}
