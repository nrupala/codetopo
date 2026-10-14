// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! The entitlement state machine: Paddle events in, key actions out.
//!
//! Every handler is written to converge under Paddle's delivery
//! contract — at least once, unordered. Three guards do the work:
//! processed event ids make a replay a no-op; a per-subscription
//! `last_event_at` makes an out-of-order delivery stale instead of
//! destructive; and key issuance is keyed by deterministic ids (the
//! subscription id, then `<sub>~<seq>` for replacements), so a handler
//! retried after a partial failure finds its own earlier work instead
//! of duplicating it.
//!
//! The grace and scheduling rules, stated once, here and in the code:
//!
//! - **Cancel** revokes at the cancellation's *effective end* — the
//!   scheduled change's `effective_from`, else the current billing
//!   period's end, else the event time. Until then the key is live:
//!   the buyer paid for that period.
//! - **Past due** starts a 7-day grace from the *first* past-due event.
//!   Returning to active inside the grace clears it; the clock never
//!   restarts on a repeated past-due event. Past the deadline the sweep
//!   revokes. When both a cancellation and a past-due deadline pend,
//!   the earlier one wins.
//! - **Restore** (activated / resumed / updated-to-active) clears any
//!   pending revocation. If the key was already revoked, restore issues
//!   a *replacement* key with a fresh claim delivery — the old secret
//!   is gone and is never resurrected.

use codetopo_server::keys::KeyStore;

use crate::crypto;
use crate::paddle::{SubscriptionData, TransactionData, WebhookEvent};
use crate::store::{
    BridgeStore, Entitlement, STATUS_ACTIVE, STATUS_CANCEL_SCHEDULED, STATUS_PAST_DUE,
    STATUS_REVOKED,
};
use crate::BridgeError;

/// How long a buyer has to claim a key after issuance: 7 days.
pub const CLAIM_TTL_SECS: i64 = 7 * 24 * 3600;
/// Grace between a past-due event and revocation: 7 days.
pub const PAST_DUE_GRACE_SECS: i64 = 7 * 24 * 3600;

/// What an applied event decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOutcome {
    /// State changed (or was affirmatively re-established).
    Applied,
    /// This event id was already applied; nothing was done.
    Duplicate,
    /// The event predates the last event applied to its subscription.
    Stale,
    /// Not ours to act on (another product's price, an unknown
    /// subscription, an event family the bridge does not handle).
    Ignored,
}

/// The policy the state machine applies: which price is codetopo's,
/// and which plan its keys carry. Configuration, never literals in the
/// handlers (the Conduit pattern).
pub struct Policy {
    /// The Paddle price id for the hosted tier.
    pub price_id: String,
    /// The plan name recorded on issued keys.
    pub plan: String,
    /// The claim-sealing key (environment-only; see [`crate::crypto`]).
    pub claim_key: [u8; 32],
}

/// Applies one signature-verified event.
///
/// `now` is the caller's clock; `occurred_at` (inside the event) is the
/// ordering clock. The event is recorded only after its handler runs,
/// so a failure leaves Paddle's retry free to converge.
pub fn apply_event(
    store: &BridgeStore,
    keys: &KeyStore,
    policy: &Policy,
    event: &WebhookEvent,
    now: i64,
) -> Result<EventOutcome, BridgeError> {
    if store.event_processed(&event.event_id)? {
        return Ok(EventOutcome::Duplicate);
    }
    let occurred = event.occurred_unix().unwrap_or(now);
    // Central ordering guard: an event older than the last one applied
    // to its subscription is stale, whichever handler it names — this
    // is what stops a late transaction or activation from resurrecting
    // a subscription that has since been canceled.
    if let Some(subscription_id) = event_subscription_id(event) {
        if let Some(ent) = store.entitlement(&subscription_id)? {
            if occurred < ent.last_event_at {
                store.record_event(&event.event_id, &event.event_type, occurred, now)?;
                return Ok(EventOutcome::Stale);
            }
        }
    }
    let outcome = match event.event_type.as_str() {
        "subscription.created" | "subscription.activated" | "subscription.resumed" => {
            let data = parse_data::<SubscriptionData>(event)?;
            if !carries_price(&data.items, &policy.price_id) {
                EventOutcome::Ignored
            } else {
                ensure_key(store, keys, policy, &data.id, occurred, now)?;
                if data.status.as_deref() == Some("past_due") {
                    mark_past_due(store, &data.id, occurred, now)?;
                }
                EventOutcome::Applied
            }
        }
        "subscription.canceled" => {
            let data = parse_data::<SubscriptionData>(event)?;
            apply_cancellation(store, keys, &data, occurred, now)?
        }
        "subscription.past_due" => {
            let data = parse_data::<SubscriptionData>(event)?;
            with_entitlement(store, &data.id, occurred, now, |ent| {
                mark_past_due_row(ent, now);
                Ok(EventOutcome::Applied)
            })?
        }
        "subscription.updated" => {
            let data = parse_data::<SubscriptionData>(event)?;
            apply_update(store, keys, policy, &data, occurred, now)?
        }
        "transaction.completed" => {
            let data = parse_data::<TransactionData>(event)?;
            apply_transaction(store, keys, policy, &data, occurred, now)?
        }
        _ => EventOutcome::Ignored,
    };
    store.record_event(&event.event_id, &event.event_type, occurred, now)?;
    Ok(outcome)
}

/// Exchanges a claim token for the credential it addresses, once.
///
/// Every refusal is the same [`BridgeError::ClaimUnavailable`]: an
/// unknown token, an already-claimed delivery, and an expired one are
/// indistinguishable from outside.
pub fn claim_key(
    store: &BridgeStore,
    policy: &Policy,
    token: &str,
    now: i64,
) -> Result<(String, String), BridgeError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(BridgeError::ClaimUnavailable);
    }
    let Some(key_id) = store.key_for_token(token)? else {
        return Err(BridgeError::ClaimUnavailable);
    };
    let Some(delivery) = store.delivery(&key_id)? else {
        return Err(BridgeError::ClaimUnavailable);
    };
    if delivery.claimed_at.is_some() {
        return Err(BridgeError::ClaimUnavailable);
    }
    let Some(sealed) = &delivery.sealed else {
        return Err(BridgeError::ClaimUnavailable);
    };
    if now >= delivery.expires_at {
        // Purge inline, so an expired token dies even between sweeps.
        store.purge_expired_deliveries(now)?;
        return Err(BridgeError::ClaimUnavailable);
    }
    let secret = crypto::open(&policy.claim_key, sealed)?;
    store.complete_claim(&key_id, now)?;
    Ok((key_id, secret))
}

/// What one sweep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepReport {
    /// Keys revoked because a grace or cancellation deadline fell due.
    pub revoked: usize,
    /// Unclaimed deliveries whose plaintext was purged at expiry.
    pub purged_deliveries: usize,
}

/// Applies everything that has fallen due: pending revocations whose
/// time has come, and expired unclaimed deliveries. Runs at bridge
/// startup and on an interval; safe to run any number of times.
pub fn sweep(store: &BridgeStore, keys: &KeyStore, now: i64) -> Result<SweepReport, BridgeError> {
    let mut report = SweepReport::default();
    for mut ent in store.due_revocations(now)? {
        keys.revoke(&ent.key_id)?;
        ent.status = STATUS_REVOKED.to_string();
        ent.revoke_at = None;
        store.save_entitlement(&ent, now)?;
        report.revoked += 1;
    }
    report.purged_deliveries = store.purge_expired_deliveries(now)?;
    Ok(report)
}

// -- handlers ----------------------------------------------------------

/// Issues (or re-affirms, or replaces) the key for a subscription and,
/// on a fresh issuance, seals its delivery and registers the
/// subscription id as a claim token.
fn ensure_key(
    store: &BridgeStore,
    keys: &KeyStore,
    policy: &Policy,
    subscription_id: &str,
    occurred: i64,
    now: i64,
) -> Result<(), BridgeError> {
    match store.entitlement(subscription_id)? {
        None => {
            let key_id = key_id_for(subscription_id, 1);
            let secret = issue_or_adopt(keys, policy, &key_id)?;
            store.save_entitlement(
                &Entitlement {
                    subscription_id: subscription_id.to_string(),
                    key_id: key_id.clone(),
                    key_seq: 1,
                    status: STATUS_ACTIVE.to_string(),
                    revoke_at: None,
                    last_event_at: occurred,
                },
                now,
            )?;
            if let Some(secret) = secret {
                create_delivery(store, policy, subscription_id, &key_id, &secret, now)?;
            }
            Ok(())
        }
        Some(mut ent) => {
            ent.last_event_at = ent.last_event_at.max(occurred);
            if key_is_active(keys, &ent.key_id)? {
                // Restore / re-affirm: clear any pending revocation.
                ent.status = STATUS_ACTIVE.to_string();
                ent.revoke_at = None;
                store.save_entitlement(&ent, now)?;
                return Ok(());
            }
            // The held key is revoked: replace it. The old secret stays
            // dead; the buyer claims the new one like the first.
            let seq = ent.key_seq + 1;
            let key_id = key_id_for(subscription_id, seq);
            let secret = issue_or_adopt(keys, policy, &key_id)?;
            ent.key_id = key_id.clone();
            ent.key_seq = seq;
            ent.status = STATUS_ACTIVE.to_string();
            ent.revoke_at = None;
            store.save_entitlement(&ent, now)?;
            if let Some(secret) = secret {
                create_delivery(store, policy, subscription_id, &key_id, &secret, now)?;
            }
            Ok(())
        }
    }
}

/// Applies a cancellation: schedule the revocation at its effective
/// end, or revoke now when that end has already passed.
fn apply_cancellation(
    store: &BridgeStore,
    keys: &KeyStore,
    data: &SubscriptionData,
    occurred: i64,
    now: i64,
) -> Result<EventOutcome, BridgeError> {
    let effective = cancellation_effective_at(data, occurred);
    with_entitlement(store, &data.id, occurred, now, |ent| {
        if effective <= now {
            keys.revoke(&ent.key_id)?;
            ent.status = STATUS_REVOKED.to_string();
            ent.revoke_at = None;
        } else {
            schedule_revocation(ent, STATUS_CANCEL_SCHEDULED, effective);
        }
        Ok(EventOutcome::Applied)
    })
}

/// Marks a subscription past due (the ensure-then-mark path used when a
/// created/activated event already carries a past-due status).
fn mark_past_due(
    store: &BridgeStore,
    subscription_id: &str,
    occurred: i64,
    now: i64,
) -> Result<(), BridgeError> {
    let _ = with_entitlement(store, subscription_id, occurred, now, |ent| {
        mark_past_due_row(ent, now);
        Ok(EventOutcome::Applied)
    })?;
    Ok(())
}

/// The past-due row transition: first past-due starts the grace clock;
/// a repeat never extends it; an earlier cancellation deadline is kept.
fn mark_past_due_row(ent: &mut Entitlement, now: i64) {
    if ent.status == STATUS_REVOKED {
        return;
    }
    let grace_end = now + PAST_DUE_GRACE_SECS;
    ent.revoke_at = Some(match ent.revoke_at {
        Some(existing) if ent.status == STATUS_PAST_DUE => existing,
        Some(existing) => existing.min(grace_end),
        None => grace_end,
    });
    ent.status = STATUS_PAST_DUE.to_string();
}

/// Schedules a revocation, keeping the earlier deadline when one is
/// already pending (a cancellation inside a past-due grace, or the
/// reverse, revokes at whichever comes first).
fn schedule_revocation(ent: &mut Entitlement, status: &str, effective: i64) {
    if ent.status == STATUS_REVOKED {
        return;
    }
    ent.revoke_at = Some(match ent.revoke_at {
        Some(existing) => existing.min(effective),
        None => effective,
    });
    if ent.status != STATUS_PAST_DUE {
        ent.status = status.to_string();
    }
}

/// Applies `subscription.updated`: a scheduled cancellation appears or
/// is withdrawn, or the status moves to past_due / back to active.
fn apply_update(
    store: &BridgeStore,
    keys: &KeyStore,
    policy: &Policy,
    data: &SubscriptionData,
    occurred: i64,
    now: i64,
) -> Result<EventOutcome, BridgeError> {
    // An update for a subscription the bridge never granted carries no
    // action — updates also fire for trials and imports.
    if store.entitlement(&data.id)?.is_none() {
        return Ok(EventOutcome::Ignored);
    }
    let cancel = data
        .scheduled_change
        .as_ref()
        .filter(|change| change.action == "cancel");
    match cancel {
        Some(change) => {
            let effective = crate::paddle::parse_rfc3339(&change.effective_from)
                .unwrap_or(occurred);
            with_entitlement(store, &data.id, occurred, now, |ent| {
                if effective <= now {
                    keys.revoke(&ent.key_id)?;
                    ent.status = STATUS_REVOKED.to_string();
                    ent.revoke_at = None;
                } else {
                    schedule_revocation(ent, STATUS_CANCEL_SCHEDULED, effective);
                }
                Ok(EventOutcome::Applied)
            })
        }
        None => match data.status.as_deref() {
            Some("past_due") => with_entitlement(store, &data.id, occurred, now, |ent| {
                mark_past_due_row(ent, now);
                Ok(EventOutcome::Applied)
            }),
            Some("active") | Some("trialing") => {
                // Back in good standing — the same restore as an
                // activation, including replacing a revoked key.
                ensure_key(store, keys, policy, &data.id, occurred, now)?;
                Ok(EventOutcome::Applied)
            }
            _ => with_entitlement(store, &data.id, occurred, now, |ent| {
                // Any other update still lifts a withdrawn
                // cancellation's schedule.
                if ent.status == STATUS_CANCEL_SCHEDULED
                    && data.scheduled_change.is_none()
                {
                    ent.status = STATUS_ACTIVE.to_string();
                    ent.revoke_at = None;
                }
                Ok(EventOutcome::Applied)
            }),
        },
    }
}

/// Applies `transaction.completed`: the checkout's money event. Ensures
/// the subscription's key (whichever of the two events arrives first
/// issues it) and registers the transaction id as a claim token.
fn apply_transaction(
    store: &BridgeStore,
    keys: &KeyStore,
    policy: &Policy,
    data: &TransactionData,
    occurred: i64,
    now: i64,
) -> Result<EventOutcome, BridgeError> {
    let Some(subscription_id) = data.subscription_id.as_deref() else {
        return Ok(EventOutcome::Ignored); // a one-off payment buys no key
    };
    if !carries_price(&data.items, &policy.price_id) {
        return Ok(EventOutcome::Ignored);
    }
    ensure_key(store, keys, policy, subscription_id, occurred, now)?;
    // Attach this transaction as a claim token when the key's delivery
    // is still waiting to be claimed.
    if let Some(ent) = store.entitlement(subscription_id)? {
        if let Some(delivery) = store.delivery(&ent.key_id)? {
            if delivery.claimed_at.is_none() && delivery.sealed.is_some() {
                store.add_claim_token(&data.id, &ent.key_id)?;
            }
        }
    }
    Ok(EventOutcome::Applied)
}

// -- helpers -----------------------------------------------------------

/// Runs `f` against a subscription's entitlement under the ordering
/// guard: unknown subscription → [`EventOutcome::Ignored`]; an event
/// older than the last applied one → [`EventOutcome::Stale`] (the
/// central guard in [`apply_event`] normally catches it first; this is
/// the same check at the row, as defence in depth).
fn with_entitlement(
    store: &BridgeStore,
    subscription_id: &str,
    occurred: i64,
    now: i64,
    f: impl FnOnce(&mut Entitlement) -> Result<EventOutcome, BridgeError>,
) -> Result<EventOutcome, BridgeError> {
    let Some(mut ent) = store.entitlement(subscription_id)? else {
        return Ok(EventOutcome::Ignored);
    };
    if occurred < ent.last_event_at {
        return Ok(EventOutcome::Stale);
    }
    ent.last_event_at = occurred;
    let outcome = f(&mut ent)?;
    store.save_entitlement(&ent, now)?;
    Ok(outcome)
}

/// The subscription an event belongs to, read from the raw payload:
/// `data.id` for subscription events, `data.subscription_id` for
/// transactions. `None` for anything else (nothing to order against).
fn event_subscription_id(event: &WebhookEvent) -> Option<String> {
    if event.event_type.starts_with("subscription.") {
        event.data.get("id").and_then(|v| v.as_str()).map(str::to_string)
    } else if event.event_type == "transaction.completed" {
        event
            .data
            .get("subscription_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    } else {
        None
    }
}

/// When a cancellation bites: the scheduled change's effective time if
/// one is present, else the current period's end, else the event time.
fn cancellation_effective_at(data: &SubscriptionData, occurred: i64) -> i64 {
    if let Some(change) = &data.scheduled_change {
        if change.action == "cancel" {
            if let Some(at) = crate::paddle::parse_rfc3339(&change.effective_from) {
                return at;
            }
        }
    }
    if let Some(period) = &data.current_billing_period {
        if let Some(at) = crate::paddle::parse_rfc3339(&period.ends_at) {
            return at;
        }
    }
    if let Some(canceled_at) = &data.canceled_at {
        if let Some(at) = crate::paddle::parse_rfc3339(canceled_at) {
            return at;
        }
    }
    occurred
}

/// Issues a key, or adopts the row a previous partial attempt created.
///
/// Returns the plaintext only when this call created the key — the one
/// moment it exists. Adoption (the key row is already there and active)
/// returns `None`: its secret was sealed by the attempt that made it,
/// or is unrecoverable and an operator concern.
fn issue_or_adopt(
    keys: &KeyStore,
    policy: &Policy,
    key_id: &str,
) -> Result<Option<String>, BridgeError> {
    match keys.issue(key_id, &policy.plan) {
        Ok(secret) => Ok(Some(secret)),
        Err(err) if err.code() == "key_exists" => {
            if key_is_active(keys, key_id)? {
                Ok(None)
            } else {
                Err(err.into())
            }
        }
        Err(err) => Err(err.into()),
    }
}

/// Seals a fresh secret into a delivery and registers the subscription
/// id as its first claim token.
fn create_delivery(
    store: &BridgeStore,
    policy: &Policy,
    subscription_id: &str,
    key_id: &str,
    secret: &str,
    now: i64,
) -> Result<(), BridgeError> {
    let sealed = crypto::seal(&policy.claim_key, secret)?;
    store.create_delivery(key_id, &sealed, now, now + CLAIM_TTL_SECS)?;
    store.add_claim_token(subscription_id, key_id)?;
    Ok(())
}

/// True when `key_id` exists in the key store and is `active`.
fn key_is_active(keys: &KeyStore, key_id: &str) -> Result<bool, BridgeError> {
    let summaries = keys.list()?;
    Ok(summaries
        .iter()
        .any(|s| s.key_id == key_id && s.status == "active"))
}

/// The deterministic key id for a subscription's `seq`-th key: the
/// subscription id itself first, then `<sub>~<seq>` for replacements.
/// Deterministic ids are what make a retried issuance converge instead
/// of duplicating.
fn key_id_for(subscription_id: &str, seq: i64) -> String {
    if seq <= 1 {
        subscription_id.to_string()
    } else {
        format!("{subscription_id}~{seq}")
    }
}

/// True when any line item names the configured price.
fn carries_price(items: &[crate::paddle::LineItem], price_id: &str) -> bool {
    items
        .iter()
        .any(|item| item.price.as_ref().is_some_and(|price| price.id == price_id))
}

/// Deserialises an event's `data` into the family shape it needs.
fn parse_data<T: serde::de::DeserializeOwned>(event: &WebhookEvent) -> Result<T, BridgeError> {
    serde_json::from_value(event.data.clone()).map_err(|err| {
        BridgeError::Store(format!("event {} payload does not parse: {err}", event.event_id))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::BridgeStore;

    const PRICE: &str = "pri_codetopo";
    const NOW: i64 = 1_800_000_000;

    fn policy() -> Policy {
        Policy {
            price_id: PRICE.to_string(),
            plan: "hosted".to_string(),
            claim_key: [3u8; 32],
        }
    }

    fn stores() -> (BridgeStore, KeyStore) {
        (
            BridgeStore::in_memory().expect("bridge store"),
            KeyStore::parse(""),
        )
    }

    fn rfc(ts: i64) -> String {
        // Test timestamps ride on the parser's inverse: build from a
        // known-good formatting via civil math in the paddle tests —
        // here a fixed epoch anchor plus whole days keeps strings exact.
        let base = 1_800_000_000; // 2027-01-15T08:00:00Z
        let delta = ts - base;
        assert_eq!(delta % 60, 0, "test times are whole minutes");
        // Format by round-tripping through the parser's own inverse:
        // use a tiny formatter over days_from_civil's sibling.
        format_rfc3339(ts)
    }

    fn format_rfc3339(ts: i64) -> String {
        let days = ts.div_euclid(86_400);
        let secs = ts.rem_euclid(86_400);
        let (y, m, d) = civil_from_days(days);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            secs / 3600,
            (secs % 3600) / 60,
            secs % 60
        )
    }

    fn civil_from_days(z: i64) -> (i64, i64, i64) {
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (if m <= 2 { y + 1 } else { y }, m, d)
    }

    fn subscription_event(
        event_id: &str,
        event_type: &str,
        sub: &str,
        occurred: i64,
        extra: serde_json::Value,
    ) -> WebhookEvent {
        let mut data = serde_json::json!({
            "id": sub,
            "status": "active",
            "items": [{ "price": { "id": PRICE } }],
        });
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            data[k] = v;
        }
        WebhookEvent {
            event_id: event_id.to_string(),
            event_type: event_type.to_string(),
            occurred_at: rfc(occurred),
            data,
        }
    }

    fn transaction_event(event_id: &str, txn: &str, sub: &str, occurred: i64) -> WebhookEvent {
        WebhookEvent {
            event_id: event_id.to_string(),
            event_type: "transaction.completed".to_string(),
            occurred_at: rfc(occurred),
            data: serde_json::json!({
                "id": txn,
                "status": "completed",
                "subscription_id": sub,
                "items": [{ "price": { "id": PRICE } }],
            }),
        }
    }

    fn apply(
        store: &BridgeStore,
        keys: &KeyStore,
        event: &WebhookEvent,
        now: i64,
    ) -> EventOutcome {
        apply_event(store, keys, &policy(), event, now).expect("apply event")
    }

    #[test]
    fn activation_issues_a_hosted_key_and_a_claimable_delivery() {
        let (store, keys) = stores();
        let ev = subscription_event("evt_1", "subscription.activated", "sub_A", NOW, serde_json::json!({}));
        assert_eq!(apply(&store, &keys, &ev, NOW), EventOutcome::Applied);
        let ent = store.entitlement("sub_A").expect("read").expect("entitlement");
        assert_eq!(ent.key_id, "sub_A");
        assert_eq!(ent.status, STATUS_ACTIVE);
        // The buyer claims with the subscription id and gets a working key.
        let (key_id, secret) = claim_key(&store, &policy(), "sub_A", NOW).expect("claim");
        assert_eq!(key_id, "sub_A");
        let verified = keys.verify("sub_A", &secret).expect("claimed key verifies");
        assert_eq!(verified.plan, "hosted");
        // Second claim is refused, by either token.
        assert!(matches!(
            claim_key(&store, &policy(), "sub_A", NOW),
            Err(BridgeError::ClaimUnavailable)
        ));
    }

    #[test]
    fn transaction_completed_attaches_the_checkout_token() {
        let (store, keys) = stores();
        let act = subscription_event("evt_1", "subscription.activated", "sub_B", NOW, serde_json::json!({}));
        apply(&store, &keys, &act, NOW);
        let txn = transaction_event("evt_2", "txn_B", "sub_B", NOW + 60);
        assert_eq!(apply(&store, &keys, &txn, NOW + 60), EventOutcome::Applied);
        let (_, secret) = claim_key(&store, &policy(), "txn_B", NOW + 120).expect("claim by txn");
        assert!(keys.verify("sub_B", &secret).is_ok());
    }

    #[test]
    fn transaction_first_also_issues_the_key() {
        let (store, keys) = stores();
        let txn = transaction_event("evt_1", "txn_C", "sub_C", NOW);
        assert_eq!(apply(&store, &keys, &txn, NOW), EventOutcome::Applied);
        let (_, secret) = claim_key(&store, &policy(), "txn_C", NOW).expect("claim by txn");
        assert!(keys.verify("sub_C", &secret).is_ok());
    }

    #[test]
    fn replayed_events_are_no_ops() {
        let (store, keys) = stores();
        let ev = subscription_event("evt_1", "subscription.activated", "sub_D", NOW, serde_json::json!({}));
        apply(&store, &keys, &ev, NOW);
        assert_eq!(apply(&store, &keys, &ev, NOW), EventOutcome::Duplicate);
        assert_eq!(keys.len(), 1, "one subscription, one key");
        let ent = store.entitlement("sub_D").expect("read").expect("entitlement");
        assert_eq!(ent.key_seq, 1);
    }

    #[test]
    fn another_products_price_is_ignored() {
        let (store, keys) = stores();
        let mut ev = subscription_event("evt_1", "subscription.activated", "sub_E", NOW, serde_json::json!({}));
        ev.data["items"] = serde_json::json!([{ "price": { "id": "pri_someone_else" } }]);
        assert_eq!(apply(&store, &keys, &ev, NOW), EventOutcome::Ignored);
        assert_eq!(keys.len(), 0);
        assert!(store.entitlement("sub_E").expect("read").is_none());
    }

    #[test]
    fn cancellation_schedules_revocation_at_the_period_end() {
        let (store, keys) = stores();
        let act = subscription_event("evt_1", "subscription.activated", "sub_F", NOW, serde_json::json!({}));
        apply(&store, &keys, &act, NOW);
        let (_, secret) = claim_key(&store, &policy(), "sub_F", NOW).expect("claim");

        let ends = NOW + 20 * 86_400;
        let cancel = subscription_event(
            "evt_2",
            "subscription.canceled",
            "sub_F",
            NOW + 3600,
            serde_json::json!({
                "status": "canceled",
                "scheduled_change": { "action": "cancel", "effective_from": rfc(ends) },
                "current_billing_period": { "ends_at": rfc(ends) },
            }),
        );
        assert_eq!(apply(&store, &keys, &cancel, NOW + 3600), EventOutcome::Applied);
        let ent = store.entitlement("sub_F").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_CANCEL_SCHEDULED);
        assert_eq!(ent.revoke_at, Some(ends));
        // Paid period still running: the key works, and the sweep agrees.
        assert!(keys.verify("sub_F", &secret).is_ok());
        assert_eq!(sweep(&store, &keys, ends - 1).expect("sweep").revoked, 0);
        assert!(keys.verify("sub_F", &secret).is_ok());
        // At the effective end the sweep revokes.
        assert_eq!(sweep(&store, &keys, ends).expect("sweep").revoked, 1);
        assert!(keys.verify("sub_F", &secret).is_err());
        let ent = store.entitlement("sub_F").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_REVOKED);
    }

    #[test]
    fn immediate_cancellation_revokes_now() {
        let (store, keys) = stores();
        let act = subscription_event("evt_1", "subscription.activated", "sub_G", NOW, serde_json::json!({}));
        apply(&store, &keys, &act, NOW);
        let (_, secret) = claim_key(&store, &policy(), "sub_G", NOW).expect("claim");
        let cancel = subscription_event(
            "evt_2",
            "subscription.canceled",
            "sub_G",
            NOW + 60,
            serde_json::json!({ "status": "canceled" }),
        );
        apply(&store, &keys, &cancel, NOW + 60);
        assert!(keys.verify("sub_G", &secret).is_err());
    }

    #[test]
    fn past_due_grace_restore_and_expiry() {
        let (store, keys) = stores();
        let act = subscription_event("evt_1", "subscription.activated", "sub_H", NOW, serde_json::json!({}));
        apply(&store, &keys, &act, NOW);
        let (_, secret) = claim_key(&store, &policy(), "sub_H", NOW).expect("claim");

        let due = subscription_event(
            "evt_2",
            "subscription.past_due",
            "sub_H",
            NOW + 60,
            serde_json::json!({ "status": "past_due" }),
        );
        apply(&store, &keys, &due, NOW + 60);
        let ent = store.entitlement("sub_H").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_PAST_DUE);
        assert_eq!(ent.revoke_at, Some(NOW + 60 + PAST_DUE_GRACE_SECS));
        assert!(keys.verify("sub_H", &secret).is_ok(), "grace keeps the key live");

        // A repeat past-due does not restart the clock.
        let due2 = subscription_event(
            "evt_3",
            "subscription.past_due",
            "sub_H",
            NOW + 120,
            serde_json::json!({ "status": "past_due" }),
        );
        apply(&store, &keys, &due2, NOW + 120);
        let ent = store.entitlement("sub_H").expect("read").expect("entitlement");
        assert_eq!(ent.revoke_at, Some(NOW + 60 + PAST_DUE_GRACE_SECS));

        // Returning to active clears the pending revocation.
        let back = subscription_event("evt_4", "subscription.activated", "sub_H", NOW + 180, serde_json::json!({}));
        apply(&store, &keys, &back, NOW + 180);
        let ent = store.entitlement("sub_H").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_ACTIVE);
        assert_eq!(ent.revoke_at, None);
        assert_eq!(sweep(&store, &keys, NOW + 60 + PAST_DUE_GRACE_SECS + 1).expect("sweep").revoked, 0);
        assert!(keys.verify("sub_H", &secret).is_ok());
    }

    #[test]
    fn past_due_past_grace_revokes_in_the_sweep() {
        let (store, keys) = stores();
        let act = subscription_event("evt_1", "subscription.activated", "sub_I", NOW, serde_json::json!({}));
        apply(&store, &keys, &act, NOW);
        let (_, secret) = claim_key(&store, &policy(), "sub_I", NOW).expect("claim");
        let due = subscription_event(
            "evt_2",
            "subscription.past_due",
            "sub_I",
            NOW + 60,
            serde_json::json!({ "status": "past_due" }),
        );
        apply(&store, &keys, &due, NOW + 60);
        let deadline = NOW + 60 + PAST_DUE_GRACE_SECS;
        assert_eq!(sweep(&store, &keys, deadline - 1).expect("sweep").revoked, 0);
        assert_eq!(sweep(&store, &keys, deadline).expect("sweep").revoked, 1);
        assert!(keys.verify("sub_I", &secret).is_err());
    }

    #[test]
    fn a_revoked_subscription_restored_gets_a_replacement_key() {
        let (store, keys) = stores();
        let act = subscription_event("evt_1", "subscription.activated", "sub_J", NOW, serde_json::json!({}));
        apply(&store, &keys, &act, NOW);
        let (_, old_secret) = claim_key(&store, &policy(), "sub_J", NOW).expect("claim");
        let cancel = subscription_event(
            "evt_2",
            "subscription.canceled",
            "sub_J",
            NOW + 60,
            serde_json::json!({ "status": "canceled" }),
        );
        apply(&store, &keys, &cancel, NOW + 60);
        assert!(keys.verify("sub_J", &old_secret).is_err());

        let back = subscription_event("evt_3", "subscription.activated", "sub_J", NOW + 120, serde_json::json!({}));
        apply(&store, &keys, &back, NOW + 120);
        let ent = store.entitlement("sub_J").expect("read").expect("entitlement");
        assert_eq!(ent.key_id, "sub_J~2");
        assert_eq!(ent.status, STATUS_ACTIVE);
        let (key_id, new_secret) = claim_key(&store, &policy(), "sub_J", NOW + 120).expect("claim replacement");
        assert_eq!(key_id, "sub_J~2");
        assert_ne!(new_secret, old_secret);
        assert!(keys.verify("sub_J~2", &new_secret).is_ok());
        assert!(keys.verify("sub_J", &old_secret).is_err(), "the old key stays dead");
    }

    #[test]
    fn a_second_subscription_is_a_second_key() {
        let (store, keys) = stores();
        for sub in ["sub_K1", "sub_K2"] {
            let ev = subscription_event("evt_x", "subscription.activated", sub, NOW, serde_json::json!({}));
            // Distinct event ids per delivery in real life; emulate.
            let mut ev = ev;
            ev.event_id = format!("evt_{sub}");
            apply(&store, &keys, &ev, NOW);
        }
        assert_eq!(keys.active_len(), 2);
    }

    #[test]
    fn stale_events_do_not_kill_a_restored_key() {
        let (store, keys) = stores();
        let act = subscription_event("evt_1", "subscription.activated", "sub_L", NOW + 1200, serde_json::json!({}));
        apply(&store, &keys, &act, NOW + 1200);
        let (_, secret) = claim_key(&store, &policy(), "sub_L", NOW + 1200).expect("claim");
        // A cancellation that *occurred* before the activation arrives late.
        let stale_cancel = subscription_event(
            "evt_2",
            "subscription.canceled",
            "sub_L",
            NOW,
            serde_json::json!({ "status": "canceled" }),
        );
        assert_eq!(apply(&store, &keys, &stale_cancel, NOW + 2400), EventOutcome::Stale);
        assert!(keys.verify("sub_L", &secret).is_ok());
    }

    #[test]
    fn unclaimed_deliveries_expire_and_purge_their_plaintext() {
        let (store, keys) = stores();
        let act = subscription_event("evt_1", "subscription.activated", "sub_M", NOW, serde_json::json!({}));
        apply(&store, &keys, &act, NOW);
        // Within the window the claim works (fresh pair for the purge half).
        let act2 = subscription_event("evt_2", "subscription.activated", "sub_N", NOW, serde_json::json!({}));
        apply(&store, &keys, &act2, NOW);
        assert!(claim_key(&store, &policy(), "sub_M", NOW + CLAIM_TTL_SECS - 1).is_ok());
        // Past it, the sweep purges and the claim is refused.
        let report = sweep(&store, &keys, NOW + CLAIM_TTL_SECS).expect("sweep");
        assert_eq!(report.purged_deliveries, 1);
        assert!(matches!(
            claim_key(&store, &policy(), "sub_N", NOW + CLAIM_TTL_SECS),
            Err(BridgeError::ClaimUnavailable)
        ));
    }

    #[test]
    fn updated_withdraws_a_scheduled_cancellation() {
        let (store, keys) = stores();
        let act = subscription_event("evt_1", "subscription.activated", "sub_O", NOW, serde_json::json!({}));
        apply(&store, &keys, &act, NOW);
        let ends = NOW + 20 * 86_400;
        let cancel = subscription_event(
            "evt_2",
            "subscription.canceled",
            "sub_O",
            NOW + 60,
            serde_json::json!({
                "status": "canceled",
                "scheduled_change": { "action": "cancel", "effective_from": rfc(ends) },
            }),
        );
        apply(&store, &keys, &cancel, NOW + 60);
        // The buyer undoes the cancellation: an update with no
        // scheduled change and an active status restores the key.
        let undo = subscription_event("evt_3", "subscription.updated", "sub_O", NOW + 120, serde_json::json!({}));
        assert_eq!(apply(&store, &keys, &undo, NOW + 120), EventOutcome::Applied);
        let ent = store.entitlement("sub_O").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_ACTIVE);
        assert_eq!(ent.revoke_at, None);
        assert_eq!(sweep(&store, &keys, ends + 1).expect("sweep").revoked, 0);
    }
}
