// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The entitlement state machine: Stripe events in, key actions out.
//!
//! Every handler is written to converge under Stripe's delivery
//! contract — at least once, unordered. Three guards do the work:
//! processed event ids make a replay a no-op; a per-subscription
//! `last_event_at` makes an out-of-order delivery stale instead of
//! destructive; and key issuance is keyed by deterministic ids (the
//! subscription id, then `<sub>~<seq>` for replacements), so a handler
//! retried after a partial failure finds its own earlier work instead
//! of duplicating it.
//!
//! Creation is price-guarded: the only
//! handlers that can create an entitlement read the price from a
//! subscription object's line items and act only when it is one of
//! the configured ones — the hosted (Team) price, and the Guardrails
//! price when that tier is open. The price is also the tier stamp:
//! the plan resolved from it is recorded on the issued key and on
//! the entitlement, so a replacement key is re-issued at the tier
//! the subscription pays for. A checkout session's payload carries no line items
//! (Stripe leaves them unexpanded in events), so
//! `checkout.session.completed` never creates — it contributes the
//! claim token, and the subscription event does the creating,
//! whichever of the two arrives second attaching the session to the
//! delivery. One consequence worth stating plainly: the webhook
//! endpoint must be subscribed to `customer.subscription.created`;
//! without it, purchases would record claim tokens and mint nothing.
//!
//! The grace and scheduling rules, stated once, here and in the code:
//!
//! - **Cancel** revokes at the cancellation's *effective end* — the
//!   scheduled `cancel_at`, else the current period's end, else the
//!   event time. Until then the key is live: the buyer paid for that
//!   period. `customer.subscription.deleted` is the end itself and
//!   revokes immediately.
//! - **Past due** starts a 7-day grace from the *first* past-due
//!   signal (a subscription status move or a failed invoice payment).
//!   Returning to active inside the grace clears it; the clock never
//!   restarts on a repeated past-due signal. Past the deadline the
//!   sweep revokes. When both a cancellation and a past-due deadline
//!   pend, the earlier one wins.
//! - **Restore** (created / updated-to-active) clears any pending
//!   revocation. If the key was already revoked, restore issues a
//!   *replacement* key with a fresh claim delivery — the old secret
//!   is gone and is never resurrected.
//!
//! Three of those transitions also email the buyer (see
//! [`maybe_email_lifecycle`]): a cancellation scheduled for the
//! period end, a subscription ended (the `deleted` event, or an
//! update whose cancellation is already effective), and the first
//! entry into past-due. The trigger is derived from the event and
//! the entitlement it produced, once, after the event is recorded —
//! the same best-effort discipline as the key email: a failed send
//! is logged and never fails the webhook.
//!
//! A fourth email rides the same seam (see [`maybe_email_refund`]):
//! when a charge against a codetopo subscription is refunded, the
//! buyer hears the amount, the plan, and what the refund does to
//! the key. A refund changes no bridge state — it is not a
//! cancellation, and the entitlement keeps exactly the status the
//! subscription's own events gave it (the Director's ruling,
//! 2026-11-03: a refund and a cancellation are separate acts, as
//! his own proof purchase performed them) — so the refund handler
//! only resolves and reports. The charge payload names an invoice,
//! not a subscription, so resolution runs charge → invoice → the
//! subscription recorded for that invoice when its invoice event
//! applied → entitlement. A charge that cannot be resolved to a
//! codetopo entitlement is ignored with a log line; for those
//! buyers, Stripe's own refund email remains the notice.

use codetopo_server::keys::KeyStore;
use codetopo_server::limits::plan_limits;

use crate::crypto;
use crate::mail::{
    compose_cancellation_scheduled_email, compose_key_email, compose_payment_failed_email,
    compose_refund_email, compose_subscription_ended_email, KeyMailer, RefundKeyFate,
};
use crate::store::{
    BridgeStore, Entitlement, STATUS_ACTIVE, STATUS_CANCEL_SCHEDULED, STATUS_PAST_DUE,
    STATUS_REVOKED,
};
use crate::stripe::{
    ChargeData, CheckoutSessionData, InvoiceData, LineItem, SubscriptionData, WebhookEvent,
};
use crate::BridgeError;

/// How long a buyer has to claim a key after issuance: 7 days.
pub const CLAIM_TTL_SECS: i64 = 7 * 24 * 3600;
/// Grace between a past-due event and revocation: 7 days.
pub const PAST_DUE_GRACE_SECS: i64 = 7 * 24 * 3600;

/// The plan name recorded on keys the Guardrails price issues — the
/// tier stamp for the US$49/mo tier. The server's limits table
/// carries a Guardrails row (see `codetopo_server::limits::plan_limits`;
/// the Director's ruling, 2026-11-03: 100 active indexes, 50,000
/// queries per month, 250,000 files per index), so the stamp is both
/// the record of the tier paid and a different enforcement.
pub const GUARDRAILS_PLAN: &str = "guardrails";

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

/// The policy the state machine applies: which prices are codetopo's,
/// and which plan each one's keys carry. Configuration, never literals
/// in the handlers (the Conduit pattern).
pub struct Policy {
    /// The Stripe price id for the hosted tier (Team, US$19/mo).
    pub price_id: String,
    /// The plan name recorded on keys the hosted price issues.
    pub plan: String,
    /// The Stripe price id for the Guardrails tier (US$49/mo), when
    /// that tier is open. `None` is the single-price bridge: only
    /// `price_id` creates, exactly as before tiers existed.
    pub guardrails_price_id: Option<String>,
    /// The claim-sealing key (environment-only; see [`crate::crypto`]).
    pub claim_key: [u8; 32],
}

impl Policy {
    /// The plan a subscription's line items earn, or `None` when no
    /// item names a price this bridge sells. Items are read in
    /// order and the first known price decides: a subscription
    /// Stripe bills carries one price per item, and neither tier is
    /// sold as a mixed cart, so the order rule never arbitrates a
    /// real purchase — it only keeps the resolution deterministic.
    pub fn plan_for_items(&self, items: &[LineItem]) -> Option<String> {
        for item in items {
            let Some(price) = &item.price else {
                continue;
            };
            if price.id == self.price_id {
                return Some(self.plan.clone());
            }
            if self.guardrails_price_id.as_deref() == Some(price.id.as_str()) {
                return Some(GUARDRAILS_PLAN.to_string());
            }
        }
        None
    }
}

/// Applies one signature-verified event.
///
/// `now` is the caller's clock; `created` (inside the event) is the
/// ordering clock. The event is recorded only after its handler runs,
/// so a failure leaves Stripe's retry free to converge.
///
/// `mailer` is the key-delivery email transport, when configured.
/// After an event applies, [`maybe_email_key`] offers the send for
/// the event's subscription; email is a side channel with its own
/// failure rules (see there) and never changes the outcome here.
pub fn apply_event(
    store: &BridgeStore,
    keys: &KeyStore,
    policy: &Policy,
    mailer: Option<&dyn KeyMailer>,
    event: &WebhookEvent,
    now: i64,
) -> Result<EventOutcome, BridgeError> {
    if store.event_processed(&event.event_id)? {
        return Ok(EventOutcome::Duplicate);
    }
    let occurred = event.occurred_unix().unwrap_or(now);
    // Central ordering guard: an event older than the last one applied
    // to its subscription is stale, whichever handler it names — this
    // is what stops a late checkout or creation from resurrecting a
    // subscription that has since been deleted.
    if let Some(subscription_id) = event_subscription_id(event) {
        if let Some(ent) = store.entitlement(&subscription_id)? {
            if occurred < ent.last_event_at {
                store.record_event(&event.event_id, &event.event_type, occurred, now)?;
                return Ok(EventOutcome::Stale);
            }
        }
    }
    let outcome = match event.event_type.as_str() {
        "checkout.session.completed" => {
            let data = parse_data::<CheckoutSessionData>(event)?;
            apply_checkout(store, &data, now)?
        }
        "customer.subscription.created" => {
            let data = parse_data::<SubscriptionData>(event)?;
            apply_created(store, keys, policy, &data, occurred, now)?
        }
        "customer.subscription.updated" => {
            let data = parse_data::<SubscriptionData>(event)?;
            apply_update(store, keys, policy, &data, occurred, now)?
        }
        "customer.subscription.deleted" => {
            let data = parse_data::<SubscriptionData>(event)?;
            with_entitlement(store, &data.id, occurred, now, |ent| {
                keys.revoke(&ent.key_id)?;
                ent.status = STATUS_REVOKED.to_string();
                ent.revoke_at = None;
                Ok(EventOutcome::Applied)
            })?
        }
        // A refund moves no state (see [`apply_refund`]): the
        // handler resolves the charge to its entitlement, and the
        // refund email hook below carries the news. An unresolvable
        // charge — another product's, or an invoice this bridge
        // never saw — is ignored with a log line.
        "charge.refunded" => {
            let data = parse_data::<ChargeData>(event)?;
            apply_refund(store, &data, &event.event_id)?
        }
        "invoice.payment_failed" => {
            let data = parse_data::<InvoiceData>(event)?;
            record_invoice_link(store, &data, now)?;
            match data.subscription_id() {
                Some(subscription_id) => {
                    with_entitlement(store, subscription_id, occurred, now, |ent| {
                        mark_past_due_row(ent, now);
                        Ok(EventOutcome::Applied)
                    })?
                }
                None => EventOutcome::Ignored,
            }
        }
        "invoice.payment_succeeded" => {
            let data = parse_data::<InvoiceData>(event)?;
            record_invoice_link(store, &data, now)?;
            match data.subscription_id() {
                Some(subscription_id) => {
                    with_entitlement(store, subscription_id, occurred, now, |ent| {
                        if ent.status == STATUS_PAST_DUE {
                            ent.status = STATUS_ACTIVE.to_string();
                            ent.revoke_at = None;
                        }
                        Ok(EventOutcome::Applied)
                    })?
                }
                None => EventOutcome::Ignored,
            }
        }
        _ => EventOutcome::Ignored,
    };
    store.record_event(&event.event_id, &event.event_type, occurred, now)?;
    if outcome == EventOutcome::Applied {
        // The event is recorded before any send is attempted: if the
        // send is slow or Stripe redelivers meanwhile, the replay is
        // a duplicate and the hook below never runs twice for it.
        maybe_email_key(store, policy, mailer, event, now);
        maybe_email_lifecycle(store, mailer, event, now);
        maybe_email_refund(store, mailer, event);
    }
    Ok(outcome)
}

/// Offers the one key email for an applied event's subscription.
///
/// The send happens when everything it needs has converged — an
/// entitlement, a live (unclaimed, unexpired, not-yet-emailed)
/// delivery, and a recorded buyer address — whichever event
/// completes that set triggers it: the checkout that records the
/// address, or the creation that issues the key when the checkout
/// landed first. A replacement key is a fresh delivery with its own
/// unset marker, so it is emailed like the first.
///
/// Failure rules are strict: nothing in here may fail the webhook.
/// A missing address, an already-claimed delivery, or a transport
/// error is a log line on the bridge host (naming the event), and
/// the claim page remains the buyer's path. The send itself is
/// guarded by an atomic claim on the delivery's marker
/// ([`BridgeStore::claim_delivery_email`]): exactly one caller
/// sends, even when two events for one subscription are processed
/// concurrently. A failed send releases the claim, so a later event
/// for the same subscription may retry inside the claim window.
fn maybe_email_key(
    store: &BridgeStore,
    policy: &Policy,
    mailer: Option<&dyn KeyMailer>,
    event: &WebhookEvent,
    now: i64,
) {
    let Some(mailer) = mailer else {
        return;
    };
    let Some(subscription_id) = event_subscription_id(event) else {
        return;
    };
    let attempt = |store: &BridgeStore| -> Result<bool, String> {
        let fail = |err: BridgeError| err.to_string();
        let Some(ent) = store.entitlement(&subscription_id).map_err(fail)? else {
            return Ok(false);
        };
        let Some(delivery) = store.delivery(&ent.key_id).map_err(fail)? else {
            return Ok(false);
        };
        if delivery.claimed_at.is_some()
            || delivery.emailed_at.is_some()
            || now >= delivery.expires_at
        {
            return Ok(false);
        }
        let Some(sealed) = &delivery.sealed else {
            return Ok(false);
        };
        let Some(to) = store.buyer_email(&subscription_id).map_err(fail)? else {
            eprintln!(
                "codetopo-bridge: no buyer email recorded for subscription \
                 {subscription_id} (event {}); the key is delivered by the claim page only",
                event.event_id
            );
            return Ok(false);
        };
        let secret = crypto::open(&policy.claim_key, sealed).map_err(fail)?;
        // The email quotes the entitlement's own tier: a Guardrails
        // buyer's key email states their plan's limits, not the base
        // tier's.
        let email = compose_key_email(&to, &ent.key_id, &secret, &plan_limits(&ent.plan));
        // Claim the send before attempting it: the marker is the
        // lock, taken atomically, so a concurrent event for this
        // subscription backs off here instead of double-sending.
        if !store.claim_delivery_email(&ent.key_id, now).map_err(fail)? {
            return Ok(false);
        }
        if let Err(err) = mailer.send_key_email(&email) {
            // A failed send hands the claim back, so a later event
            // may retry inside the claim window; the retry property
            // matters more than the stamp's precision.
            if let Err(release_err) = store.release_delivery_email_claim(&ent.key_id, now) {
                eprintln!(
                    "codetopo-bridge: could not release the key-email claim for \
                     subscription {subscription_id} (event {}): {release_err}",
                    event.event_id
                );
            }
            return Err(err.to_string());
        }
        eprintln!(
            "codetopo-bridge: key email sent for subscription {subscription_id} (event {})",
            event.event_id
        );
        Ok(true)
    };
    if let Err(err) = attempt(store) {
        eprintln!(
            "codetopo-bridge: key email not sent for event {} (subscription \
             {subscription_id}): {err}",
            event.event_id
        );
    }
}

/// Offers the one lifecycle email an applied event carries, if any.
///
/// The news is derived from the event and the entitlement it
/// produced — no handler grows a mail parameter, and each rule
/// names the transition it reports:
///
/// - `customer.subscription.deleted` → the subscription has ended;
///   the key no longer works.
/// - A `customer.subscription.updated` whose payload carries a
///   cancellation (`cancel_at_period_end`, `cancel_at`, or the
///   `canceled` status): if the entitlement is now revoked, the
///   cancellation was already effective and the news is the ending;
///   if it is cancel-scheduled, the news is the scheduling, with
///   the entitlement's own `revoke_at` as the date the key stops.
/// - Any event that leaves the entitlement past-due with the grace
///   deadline this very event set (`revoke_at == now + grace`) →
///   the payment failed. That equality is the first-entry test:
///   `mark_past_due_row` sets the deadline exactly then and never
///   moves it on a repeat signal, so a second failure notice is
///   not re-emailed.
///
/// Failure rules match the key email's: nothing here may fail the
/// webhook. A lifecycle notice with no recorded buyer address is a
/// log line (the address only ever arrives on the checkout event,
/// so a subscription whose checkout predates address capture gets
/// no lifecycle mail), and a transport error is logged with the
/// event id. Unlike the key email there is no send marker and no
/// retry: the event-id guard is the dedup (a redelivery is a
/// duplicate and never reaches this hook).
fn maybe_email_lifecycle(
    store: &BridgeStore,
    mailer: Option<&dyn KeyMailer>,
    event: &WebhookEvent,
    now: i64,
) {
    let Some(mailer) = mailer else {
        return;
    };
    let Some(subscription_id) = event_subscription_id(event) else {
        return;
    };
    let attempt = |store: &BridgeStore| -> Result<bool, String> {
        let fail = |err: BridgeError| err.to_string();
        let Some(ent) = store.entitlement(&subscription_id).map_err(fail)? else {
            return Ok(false);
        };
        // Which notice, if any, this event carries. Cancellation
        // news is read before the past-due news: an update carries
        // one status, and a cancellation it schedules is the fact
        // the buyer acted on.
        enum News {
            Ended,
            Scheduled(i64),
            PaymentFailed(i64),
        }
        let news = match event.event_type.as_str() {
            "customer.subscription.deleted" => Some(News::Ended),
            "customer.subscription.updated" => {
                let data = parse_data::<SubscriptionData>(event).map_err(fail)?;
                let carries_cancellation = data.cancel_at_period_end
                    || data.cancel_at.is_some()
                    || data.status.as_deref() == Some("canceled");
                if !carries_cancellation {
                    None
                } else if ent.status == STATUS_REVOKED {
                    Some(News::Ended)
                } else if ent.status == STATUS_CANCEL_SCHEDULED {
                    ent.revoke_at.map(News::Scheduled)
                } else {
                    None
                }
            }
            _ => None,
        };
        let news = match news {
            Some(news) => news,
            None => {
                if ent.status == STATUS_PAST_DUE
                    && ent.revoke_at == Some(now + PAST_DUE_GRACE_SECS)
                {
                    News::PaymentFailed(now + PAST_DUE_GRACE_SECS)
                } else {
                    return Ok(false);
                }
            }
        };
        let Some(to) = store.buyer_email(&subscription_id).map_err(fail)? else {
            eprintln!(
                "codetopo-bridge: no buyer email recorded for subscription \
                 {subscription_id} (event {}); no lifecycle email sent",
                event.event_id
            );
            return Ok(false);
        };
        let email = match news {
            News::Ended => compose_subscription_ended_email(&to),
            News::Scheduled(ends_at) => compose_cancellation_scheduled_email(&to, ends_at),
            News::PaymentFailed(grace_ends_at) => {
                compose_payment_failed_email(&to, grace_ends_at)
            }
        };
        mailer
            .send_lifecycle_email(&email)
            .map_err(|err| err.to_string())?;
        eprintln!(
            "codetopo-bridge: lifecycle email sent for subscription \
             {subscription_id} (event {})",
            event.event_id
        );
        Ok(true)
    };
    if let Err(err) = attempt(store) {
        eprintln!(
            "codetopo-bridge: lifecycle email not sent for event {} \
             (subscription {subscription_id}): {err}",
            event.event_id
        );
    }
}

/// Records the invoice → subscription link an invoice event
/// carries, so a later charge against that invoice can resolve.
/// The link is a fact about Stripe's objects, not about the
/// entitlement: it is recorded whether or not the subscription is
/// one the bridge has entitled (a refund for an unentitled
/// subscription still has to be able to resolve — to the
/// entitlement check that then ignores it).
fn record_invoice_link(
    store: &BridgeStore,
    data: &InvoiceData,
    now: i64,
) -> Result<(), BridgeError> {
    if let Some(subscription_id) = data.subscription_id() {
        store.record_invoice_subscription(&data.id, subscription_id, now)?;
    }
    Ok(())
}

/// Resolves a charge to the codetopo subscription it concerns:
/// charge → its invoice → the subscription recorded for that
/// invoice → an entitlement must exist for it. Any break in the
/// chain resolves to `None` — the charge is not this bridge's
/// business (another product's charge on the shared account, an
/// invoice no invoice event ever named, or a subscription the
/// bridge never entitled).
fn resolve_refund_subscription(
    store: &BridgeStore,
    data: &ChargeData,
) -> Result<Option<String>, BridgeError> {
    let Some(invoice_id) = data.invoice_id() else {
        return Ok(None);
    };
    let Some(subscription_id) = store.subscription_for_invoice(invoice_id)? else {
        return Ok(None);
    };
    if store.entitlement(&subscription_id)?.is_none() {
        return Ok(None);
    }
    Ok(Some(subscription_id))
}

/// Applies a `charge.refunded` event. No state changes: a refund
/// is not a cancellation and moves no entitlement — the key's
/// position after a refund is whatever the subscription's own
/// events made it, and [`maybe_email_refund`] states exactly that.
/// The handler only decides whether the refund concerns a codetopo
/// entitlement; an unresolvable charge is a quiet
/// [`EventOutcome::Ignored`] with a log line (the account is
/// shared, and other products' refunds are not an error here).
fn apply_refund(
    store: &BridgeStore,
    data: &ChargeData,
    event_id: &str,
) -> Result<EventOutcome, BridgeError> {
    match resolve_refund_subscription(store, data)? {
        Some(_) => Ok(EventOutcome::Applied),
        None => {
            eprintln!(
                "codetopo-bridge: charge.refunded for charge {} (event {event_id}) \
                 does not resolve to a codetopo entitlement; no refund email",
                data.id
            );
            Ok(EventOutcome::Ignored)
        }
    }
}

/// Offers the refund email for an applied `charge.refunded` event.
///
/// The news is the refund itself; everything the email states is
/// re-read at send time — the entitlement's plan, and the key's
/// fate under the entitlement's *current* status (a refund that
/// lands after the subscription ended says the key has stopped;
/// one that lands on a live subscription says the refund changed
/// nothing). Dedup is the event-id guard, exactly as for the
/// lifecycle emails: the event is recorded before this hook runs,
/// so a redelivery is a duplicate and never reaches it, while a
/// second, distinct refund against the same charge is a second
/// event and earns its own email stating its own amount. Failure
/// rules match the other sends: a missing buyer address or a
/// transport error is a log line, never a failed webhook.
fn maybe_email_refund(store: &BridgeStore, mailer: Option<&dyn KeyMailer>, event: &WebhookEvent) {
    if event.event_type != "charge.refunded" {
        return;
    }
    let Some(mailer) = mailer else {
        return;
    };
    let attempt = |store: &BridgeStore| -> Result<bool, String> {
        let fail = |err: BridgeError| err.to_string();
        let data = parse_data::<ChargeData>(event).map_err(|err| err.to_string())?;
        let Some(subscription_id) = resolve_refund_subscription(store, &data).map_err(fail)?
        else {
            return Ok(false);
        };
        let Some(ent) = store.entitlement(&subscription_id).map_err(fail)? else {
            return Ok(false);
        };
        let Some(to) = store.buyer_email(&subscription_id).map_err(fail)? else {
            eprintln!(
                "codetopo-bridge: no buyer email recorded for subscription \
                 {subscription_id} (event {}); no refund email sent",
                event.event_id
            );
            return Ok(false);
        };
        let fate = if ent.status == STATUS_REVOKED {
            RefundKeyFate::AlreadyStopped
        } else if ent.status == STATUS_CANCEL_SCHEDULED || ent.status == STATUS_PAST_DUE {
            match ent.revoke_at {
                Some(ends_at) => RefundKeyFate::WorksUntil(ends_at),
                None => RefundKeyFate::Unaffected,
            }
        } else {
            RefundKeyFate::Unaffected
        };
        let email = compose_refund_email(
            &to,
            &ent.plan,
            data.refunded_amount(),
            data.amount,
            &data.currency,
            data.is_partial(),
            fate,
        );
        mailer
            .send_lifecycle_email(&email)
            .map_err(|err| err.to_string())?;
        eprintln!(
            "codetopo-bridge: refund email sent for subscription \
             {subscription_id} (event {})",
            event.event_id
        );
        Ok(true)
    };
    if let Err(err) = attempt(store) {
        eprintln!(
            "codetopo-bridge: refund email not sent for event {}: {err}",
            event.event_id
        );
    }
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
    // Pending checkout sessions whose subscription never materialised
    // are bookkeeping, not state: clear them on the claim horizon so
    // the table cannot grow on abandoned (or other products')
    // checkouts.
    store.purge_stale_pending_claims(now - CLAIM_TTL_SECS)?;
    Ok(report)
}

// -- handlers ----------------------------------------------------------

/// Applies `checkout.session.completed`: the checkout's money event.
/// It never creates an entitlement — the session payload carries no
/// price to guard creation with — so it contributes the session id as
/// a claim token: attached to the subscription's delivery when the
/// entitlement already exists, recorded as pending when the
/// subscription event has not landed yet (see [`ensure_key`], which
/// attaches pending sessions at issuance).
fn apply_checkout(
    store: &BridgeStore,
    data: &CheckoutSessionData,
    now: i64,
) -> Result<EventOutcome, BridgeError> {
    if data.mode.as_deref() != Some("subscription") || data.payment_status.as_deref() != Some("paid")
    {
        return Ok(EventOutcome::Ignored);
    }
    let Some(subscription_id) = data.subscription.as_deref() else {
        return Ok(EventOutcome::Ignored); // a paid session with no subscription sells no key
    };
    // The buyer address rides this event and no other: record it
    // against the subscription whether the entitlement already
    // exists or is still to come, so the key email can be offered
    // the moment both halves converge (see `maybe_email_key`). A
    // session that carried no usable address records nothing — the
    // claim page stays the only channel for that purchase.
    if let Some(email) = data.buyer_email() {
        store.record_buyer_email(subscription_id, email, now)?;
    }
    match store.entitlement(subscription_id)? {
        Some(ent) => {
            if let Some(delivery) = store.delivery(&ent.key_id)? {
                if delivery.claimed_at.is_none() && delivery.sealed.is_some() {
                    store.add_claim_token(&data.id, &ent.key_id)?;
                }
            }
        }
        None => {
            store.add_pending_claim(&data.id, subscription_id, now)?;
        }
    }
    Ok(EventOutcome::Applied)
}

/// Applies `customer.subscription.created`: the price-guarded creation
/// event. A subscription Stripe created but has not been paid for
/// (`incomplete`) mints nothing; if it later reaches an entitled
/// status, [`apply_update`]'s rescue creates the entitlement then.
fn apply_created(
    store: &BridgeStore,
    keys: &KeyStore,
    policy: &Policy,
    data: &SubscriptionData,
    occurred: i64,
    now: i64,
) -> Result<EventOutcome, BridgeError> {
    let Some(plan) = policy.plan_for_items(&data.items.data) else {
        return Ok(EventOutcome::Ignored);
    };
    match data.status.as_deref() {
        Some("active") | Some("trialing") => {
            ensure_key(store, keys, policy, &data.id, Some(&plan), occurred, now)?;
            Ok(EventOutcome::Applied)
        }
        Some("past_due") | Some("unpaid") => {
            ensure_key(store, keys, policy, &data.id, Some(&plan), occurred, now)?;
            mark_past_due(store, &data.id, occurred, now)?;
            Ok(EventOutcome::Applied)
        }
        _ => Ok(EventOutcome::Ignored),
    }
}

/// Applies `customer.subscription.updated`: a scheduled cancellation
/// appears or is withdrawn, or the status moves to past_due / back to
/// active. For a subscription the bridge never granted, an update is
/// also the creation rescue — Stripe can deliver `created` before the
/// first payment settles, so an entitled update may be the first
/// sighting that may create, under the same price guard.
fn apply_update(
    store: &BridgeStore,
    keys: &KeyStore,
    policy: &Policy,
    data: &SubscriptionData,
    occurred: i64,
    now: i64,
) -> Result<EventOutcome, BridgeError> {
    if store.entitlement(&data.id)?.is_none() {
        let entitled = matches!(data.status.as_deref(), Some("active") | Some("trialing"));
        if entitled {
            if let Some(plan) = policy.plan_for_items(&data.items.data) {
                ensure_key(store, keys, policy, &data.id, Some(&plan), occurred, now)?;
                return Ok(EventOutcome::Applied);
            }
        }
        return Ok(EventOutcome::Ignored);
    }
    if data.cancel_at_period_end || data.cancel_at.is_some() {
        let effective = data.cancel_at.or_else(|| data.period_end()).unwrap_or(occurred);
        return with_entitlement(store, &data.id, occurred, now, |ent| {
            if effective <= now {
                keys.revoke(&ent.key_id)?;
                ent.status = STATUS_REVOKED.to_string();
                ent.revoke_at = None;
            } else {
                schedule_revocation(ent, STATUS_CANCEL_SCHEDULED, effective);
            }
            Ok(EventOutcome::Applied)
        });
    }
    match data.status.as_deref() {
        Some("past_due") | Some("unpaid") => {
            with_entitlement(store, &data.id, occurred, now, |ent| {
                mark_past_due_row(ent, now);
                Ok(EventOutcome::Applied)
            })
        }
        Some("active") | Some("trialing") => {
            // Back in good standing — the same restore as a creation,
            // including replacing a revoked key. The event's price,
            // when it names one of ours, refreshes the recorded tier;
            // an event naming none leaves the record as it stands.
            let plan = policy.plan_for_items(&data.items.data);
            ensure_key(
                store,
                keys,
                policy,
                &data.id,
                plan.as_deref(),
                occurred,
                now,
            )?;
            Ok(EventOutcome::Applied)
        }
        Some("canceled") => {
            // The cancellation is effective at the period end; the
            // `deleted` event that follows at the true end revokes
            // immediately if this has not already fallen due.
            let effective = data.period_end().unwrap_or(occurred);
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
        _ => with_entitlement(store, &data.id, occurred, now, |ent| {
            // Any other update still lifts a withdrawn cancellation's
            // schedule (the cancellation branch above no longer fires
            // when `cancel_at_period_end` has been retracted).
            if ent.status == STATUS_CANCEL_SCHEDULED {
                ent.status = STATUS_ACTIVE.to_string();
                ent.revoke_at = None;
            }
            Ok(EventOutcome::Applied)
        }),
    }
}

/// Issues (or re-affirms, or replaces) the key for a subscription and,
/// on a fresh issuance, seals its delivery and registers the
/// subscription id as a claim token.
///
/// `plan` is the tier the calling event's price resolved to, when it
/// resolved one. Creation is price-guarded at every call site, so a
/// fresh entitlement always has one; the configured plan is the
/// defensive fallback, equal to the single-price bridge's behaviour.
/// For an existing entitlement the resolved plan refreshes the
/// recorded stamp — the record follows the price the subscription
/// pays now — and a replacement key is issued under the refreshed
/// stamp. A live key's own stamp changes only when the key is
/// replaced: the key store offers no plan mutation, by its design.
fn ensure_key(
    store: &BridgeStore,
    keys: &KeyStore,
    policy: &Policy,
    subscription_id: &str,
    plan: Option<&str>,
    occurred: i64,
    now: i64,
) -> Result<(), BridgeError> {
    match store.entitlement(subscription_id)? {
        None => {
            let plan = plan.unwrap_or(&policy.plan).to_string();
            let key_id = key_id_for(subscription_id, 1);
            let secret = issue_or_adopt(keys, &plan, &key_id)?;
            store.save_entitlement(
                &Entitlement {
                    subscription_id: subscription_id.to_string(),
                    key_id: key_id.clone(),
                    key_seq: 1,
                    plan,
                    status: STATUS_ACTIVE.to_string(),
                    revoke_at: None,
                    last_event_at: occurred,
                },
                now,
            )?;
            if let Some(secret) = secret {
                create_delivery(store, policy, subscription_id, &key_id, &secret, now)?;
                attach_pending_claims(store, subscription_id, &key_id)?;
            }
            Ok(())
        }
        Some(mut ent) => {
            ent.last_event_at = ent.last_event_at.max(occurred);
            if let Some(plan) = plan {
                ent.plan = plan.to_string();
            }
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
            let plan = ent.plan.clone();
            let secret = issue_or_adopt(keys, &plan, &key_id)?;
            ent.key_id = key_id.clone();
            ent.key_seq = seq;
            ent.status = STATUS_ACTIVE.to_string();
            ent.revoke_at = None;
            store.save_entitlement(&ent, now)?;
            if let Some(secret) = secret {
                create_delivery(store, policy, subscription_id, &key_id, &secret, now)?;
                attach_pending_claims(store, subscription_id, &key_id)?;
            }
            Ok(())
        }
    }
}

/// Marks a subscription past due (the ensure-then-mark path used when
/// a created event already carries a past-due status).
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
/// the object id for subscription events, `subscription` for checkout
/// sessions, and the billed subscription for invoices. `None` for
/// anything else (nothing to order against).
fn event_subscription_id(event: &WebhookEvent) -> Option<String> {
    let data = &event.data;
    match event.event_type.as_str() {
        "checkout.session.completed" => {
            data.get("subscription").and_then(|v| v.as_str()).map(str::to_string)
        }
        "invoice.payment_failed" | "invoice.payment_succeeded" => {
            serde_json::from_value::<InvoiceData>(data.clone())
                .ok()
                .and_then(|invoice| invoice.subscription_id().map(str::to_string))
        }
        t if t.starts_with("customer.subscription.") => {
            data.get("id").and_then(|v| v.as_str()).map(str::to_string)
        }
        _ => None,
    }
}

/// Issues a key, or adopts the row a previous partial attempt created.
///
/// Returns the plaintext only when this call created the key — the one
/// moment it exists. Adoption (the key row is already there and active)
/// returns `None`: its secret was sealed by the attempt that made it,
/// or is unrecoverable and an operator concern.
fn issue_or_adopt(
    keys: &KeyStore,
    plan: &str,
    key_id: &str,
) -> Result<Option<String>, BridgeError> {
    match keys.issue(key_id, plan) {
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

/// Attaches any checkout sessions recorded against this subscription
/// before its entitlement existed as claim tokens for the fresh
/// delivery, and retires the pending records.
fn attach_pending_claims(
    store: &BridgeStore,
    subscription_id: &str,
    key_id: &str,
) -> Result<(), BridgeError> {
    for token in store.pending_claims(subscription_id)? {
        store.add_claim_token(&token, key_id)?;
    }
    store.clear_pending_claims(subscription_id)?;
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

    const PRICE: &str = "price_codetopo";
    const GUARDRAILS_PRICE: &str = "price_codetopo_guardrails";
    const NOW: i64 = 1_800_000_000;

    fn policy() -> Policy {
        Policy {
            price_id: PRICE.to_string(),
            plan: "hosted".to_string(),
            guardrails_price_id: None,
            // Derived at runtime, not a literal (the hard-coded-key
            // scanner reads tests too, and rightly so).
            claim_key: {
                use sha2::Digest;
                sha2::Sha256::digest(b"codetopo-bridge service test key").into()
            },
        }
    }

    fn stores() -> (BridgeStore, KeyStore) {
        (
            BridgeStore::in_memory().expect("bridge store"),
            KeyStore::parse(""),
        )
    }

    /// The two-tier policy: the Guardrails price configured
    /// alongside the hosted one.
    fn policy_two_tier() -> Policy {
        let mut policy = policy();
        policy.guardrails_price_id = Some(GUARDRAILS_PRICE.to_string());
        policy
    }

    fn apply_p(
        policy: &Policy,
        store: &BridgeStore,
        keys: &KeyStore,
        event: &WebhookEvent,
        now: i64,
    ) -> EventOutcome {
        apply_event(store, keys, policy, None, event, now).expect("apply event")
    }

    fn subscription_event(
        event_id: &str,
        event_type: &str,
        sub: &str,
        created: i64,
        extra: serde_json::Value,
    ) -> WebhookEvent {
        subscription_event_priced(event_id, event_type, sub, created, PRICE, extra)
    }

    fn subscription_event_priced(
        event_id: &str,
        event_type: &str,
        sub: &str,
        created: i64,
        price: &str,
        extra: serde_json::Value,
    ) -> WebhookEvent {
        let mut data = serde_json::json!({
            "id": sub,
            "status": "active",
            "items": { "object": "list", "data": [{ "price": { "id": price } }] },
            "current_period_end": NOW + 30 * 86_400,
            "cancel_at_period_end": false,
        });
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            data[k] = v;
        }
        WebhookEvent {
            event_id: event_id.to_string(),
            event_type: event_type.to_string(),
            created,
            data,
        }
    }

    fn created_event(event_id: &str, sub: &str, created: i64) -> WebhookEvent {
        subscription_event(event_id, "customer.subscription.created", sub, created, serde_json::json!({}))
    }

    fn created_event_priced(event_id: &str, sub: &str, created: i64, price: &str) -> WebhookEvent {
        subscription_event_priced(
            event_id,
            "customer.subscription.created",
            sub,
            created,
            price,
            serde_json::json!({}),
        )
    }

    fn checkout_event(event_id: &str, session: &str, sub: &str, created: i64) -> WebhookEvent {
        WebhookEvent {
            event_id: event_id.to_string(),
            event_type: "checkout.session.completed".to_string(),
            created,
            data: serde_json::json!({
                "id": session,
                "mode": "subscription",
                "payment_status": "paid",
                "subscription": sub,
            }),
        }
    }

    fn checkout_event_with_email(
        event_id: &str,
        session: &str,
        sub: &str,
        created: i64,
        email: &str,
    ) -> WebhookEvent {
        let mut event = checkout_event(event_id, session, sub, created);
        event.data["customer_details"] = serde_json::json!({ "email": email });
        event
    }

    /// The transport double: records every message it is asked to
    /// send, and can be told to fail. Hermetic by construction.
    struct RecordingMailer {
        sent: std::sync::Mutex<Vec<crate::mail::KeyEmail>>,
        lifecycle: std::sync::Mutex<Vec<crate::mail::LifecycleEmail>>,
        failing: std::sync::Mutex<bool>,
    }

    impl RecordingMailer {
        fn new() -> Self {
            RecordingMailer {
                sent: std::sync::Mutex::new(Vec::new()),
                lifecycle: std::sync::Mutex::new(Vec::new()),
                failing: std::sync::Mutex::new(false),
            }
        }
        fn sent(&self) -> Vec<crate::mail::KeyEmail> {
            self.sent.lock().expect("sent lock").clone()
        }
        fn lifecycle(&self) -> Vec<crate::mail::LifecycleEmail> {
            self.lifecycle.lock().expect("lifecycle lock").clone()
        }
        fn set_failing(&self, failing: bool) {
            *self.failing.lock().expect("failing lock") = failing;
        }
    }

    impl crate::mail::KeyMailer for RecordingMailer {
        fn send_key_email(
            &self,
            email: &crate::mail::KeyEmail,
        ) -> Result<(), crate::mail::MailError> {
            if *self.failing.lock().expect("failing lock") {
                return Err(crate::mail::MailError("test transport is down".to_string()));
            }
            self.sent.lock().expect("sent lock").push(email.clone());
            Ok(())
        }

        fn send_lifecycle_email(
            &self,
            email: &crate::mail::LifecycleEmail,
        ) -> Result<(), crate::mail::MailError> {
            if *self.failing.lock().expect("failing lock") {
                return Err(crate::mail::MailError("test transport is down".to_string()));
            }
            self.lifecycle
                .lock()
                .expect("lifecycle lock")
                .push(email.clone());
            Ok(())
        }
    }

    /// A transport double that lingers inside the send, widening
    /// the window in which a second, concurrently processed event
    /// could also decide to send (D-3). Records what it sends.
    struct SlowMailer {
        sent: std::sync::Mutex<Vec<crate::mail::KeyEmail>>,
    }

    impl SlowMailer {
        fn new() -> Self {
            SlowMailer { sent: std::sync::Mutex::new(Vec::new()) }
        }
        fn sent(&self) -> Vec<crate::mail::KeyEmail> {
            self.sent.lock().expect("sent lock").clone()
        }
    }

    impl crate::mail::KeyMailer for SlowMailer {
        fn send_key_email(
            &self,
            email: &crate::mail::KeyEmail,
        ) -> Result<(), crate::mail::MailError> {
            std::thread::sleep(std::time::Duration::from_millis(300));
            self.sent.lock().expect("sent lock").push(email.clone());
            Ok(())
        }

        fn send_lifecycle_email(
            &self,
            _email: &crate::mail::LifecycleEmail,
        ) -> Result<(), crate::mail::MailError> {
            // The slow double exists to widen the key email's race
            // window; lifecycle sends are not its subject.
            Ok(())
        }
    }

    fn apply_with_mailer(
        store: &BridgeStore,
        keys: &KeyStore,
        mailer: &RecordingMailer,
        event: &WebhookEvent,
        now: i64,
    ) -> EventOutcome {
        apply_event(store, keys, &policy(), Some(mailer), event, now).expect("apply event")
    }

    fn invoice_event(event_id: &str, event_type: &str, sub: &str, created: i64) -> WebhookEvent {
        WebhookEvent {
            event_id: event_id.to_string(),
            event_type: event_type.to_string(),
            created,
            data: serde_json::json!({
                "id": format!("in_{event_id}"),
                "subscription": sub,
            }),
        }
    }

    fn apply(
        store: &BridgeStore,
        keys: &KeyStore,
        event: &WebhookEvent,
        now: i64,
    ) -> EventOutcome {
        apply_event(store, keys, &policy(), None, event, now).expect("apply event")
    }

    /// An invoice event naming its invoice id explicitly (the
    /// shared [`invoice_event`] helper derives the id from the
    /// event id; refund resolution tests need to name it).
    fn invoice_event_for(
        event_id: &str,
        event_type: &str,
        invoice: &str,
        sub: &str,
        created: i64,
    ) -> WebhookEvent {
        WebhookEvent {
            event_id: event_id.to_string(),
            event_type: event_type.to_string(),
            created,
            data: serde_json::json!({
                "id": invoice,
                "subscription": sub,
            }),
        }
    }

    /// A `charge.refunded` event: the charge, the invoice it
    /// settled, its full amount, the cumulative refunded total,
    /// and the refund list as Stripe nests it.
    fn refund_event(
        event_id: &str,
        charge: &str,
        invoice: Option<&str>,
        amount: i64,
        amount_refunded: i64,
        refunds: serde_json::Value,
        created: i64,
    ) -> WebhookEvent {
        WebhookEvent {
            event_id: event_id.to_string(),
            event_type: "charge.refunded".to_string(),
            created,
            data: serde_json::json!({
                "id": charge,
                "amount": amount,
                "amount_refunded": amount_refunded,
                "currency": "usd",
                "invoice": invoice,
                "refunds": { "object": "list", "data": refunds },
            }),
        }
    }

    /// A purchase whose checkout carried a buyer address, with the
    /// mailer watching: entitlement, key, key email, and the
    /// recorded address a refund email would write to.
    fn purchase_with_email(
        store: &BridgeStore,
        keys: &KeyStore,
        mailer: &RecordingMailer,
        sub: &str,
        session: &str,
        email: &str,
    ) {
        let created = created_event(&format!("evt_c_{sub}"), sub, NOW);
        assert_eq!(
            apply_with_mailer(store, keys, mailer, &created, NOW),
            EventOutcome::Applied
        );
        let checkout =
            checkout_event_with_email(&format!("evt_k_{sub}"), session, sub, NOW + 30, email);
        assert_eq!(
            apply_with_mailer(store, keys, mailer, &checkout, NOW + 30),
            EventOutcome::Applied
        );
    }

    /// A completed purchase, subscription event first: entitlement,
    /// key, and a delivery claimable by subscription and session id.
    fn purchase(store: &BridgeStore, keys: &KeyStore, sub: &str, session: &str) {
        let created = created_event(&format!("evt_c_{sub}"), sub, NOW);
        assert_eq!(apply(store, keys, &created, NOW), EventOutcome::Applied);
        let checkout = checkout_event(&format!("evt_k_{sub}"), session, sub, NOW + 30);
        assert_eq!(apply(store, keys, &checkout, NOW + 30), EventOutcome::Applied);
    }

    #[test]
    fn created_issues_a_hosted_key_and_a_claimable_delivery() {
        let (store, keys) = stores();
        let ev = created_event("evt_1", "sub_A", NOW);
        assert_eq!(apply(&store, &keys, &ev, NOW), EventOutcome::Applied);
        let ent = store.entitlement("sub_A").expect("read").expect("entitlement");
        assert_eq!(ent.key_id, "sub_A");
        assert_eq!(ent.plan, "hosted", "the record carries the tier stamp");
        assert_eq!(ent.status, STATUS_ACTIVE);
        // The buyer claims with the subscription id and gets a working key.
        let (key_id, secret) = claim_key(&store, &policy(), "sub_A", NOW).expect("claim");
        assert_eq!(key_id, "sub_A");
        let verified = keys.verify("sub_A", &secret).expect("claimed key verifies");
        assert_eq!(verified.plan, "hosted");
        // A second claim is refused.
        assert!(matches!(
            claim_key(&store, &policy(), "sub_A", NOW),
            Err(BridgeError::ClaimUnavailable)
        ));
    }

    #[test]
    fn checkout_first_records_the_session_and_it_claims_after_creation() {
        let (store, keys) = stores();
        // The money event lands before the subscription event: no
        // entitlement exists yet, and no key is minted.
        let checkout = checkout_event("evt_1", "cs_B", "sub_B", NOW);
        assert_eq!(apply(&store, &keys, &checkout, NOW), EventOutcome::Applied);
        assert_eq!(keys.len(), 0);
        assert!(store.entitlement("sub_B").expect("read").is_none());
        // Creation attaches the recorded session to the delivery.
        let created = created_event("evt_2", "sub_B", NOW + 30);
        assert_eq!(apply(&store, &keys, &created, NOW + 30), EventOutcome::Applied);
        let (_, secret) = claim_key(&store, &policy(), "cs_B", NOW + 60).expect("claim by session");
        assert!(keys.verify("sub_B", &secret).is_ok());
    }

    #[test]
    fn checkout_after_creation_attaches_the_session_token() {
        let (store, keys) = stores();
        purchase(&store, &keys, "sub_C", "cs_C");
        let (_, secret) = claim_key(&store, &policy(), "cs_C", NOW + 60).expect("claim by session");
        assert!(keys.verify("sub_C", &secret).is_ok());
    }

    #[test]
    fn checkout_requires_a_paid_subscription_session() {
        let (store, keys) = stores();
        for (event_id, patch) in [
            ("evt_m", serde_json::json!({ "mode": "payment" })),
            ("evt_u", serde_json::json!({ "payment_status": "unpaid" })),
            ("evt_n", serde_json::json!({ "subscription": null })),
        ] {
            let mut ev = checkout_event(event_id, "cs_X", "sub_X", NOW);
            for (k, v) in patch.as_object().expect("patch") {
                ev.data[k] = v.clone();
            }
            assert_eq!(apply(&store, &keys, &ev, NOW), EventOutcome::Ignored, "{event_id}");
        }
        assert_eq!(keys.len(), 0);
        assert!(store.entitlement("sub_X").expect("read").is_none());
        // And no pending session was recorded for the refused checkouts:
        // once the subscription is created, the session id claims nothing.
        let created = created_event("evt_c", "sub_X", NOW + 30);
        apply(&store, &keys, &created, NOW + 30);
        assert!(matches!(
            claim_key(&store, &policy(), "cs_X", NOW + 60),
            Err(BridgeError::ClaimUnavailable)
        ));
    }

    #[test]
    fn another_products_price_never_creates() {
        let (store, keys) = stores();
        let mut created = created_event("evt_1", "sub_E", NOW);
        created.data["items"] = serde_json::json!({
            "object": "list",
            "data": [{ "price": { "id": "price_someone_else" } }],
        });
        assert_eq!(apply(&store, &keys, &created, NOW), EventOutcome::Ignored);
        // The update rescue is guarded by the same price.
        let mut update = subscription_event(
            "evt_2",
            "customer.subscription.updated",
            "sub_E",
            NOW + 30,
            serde_json::json!({}),
        );
        update.data["items"] = created.data["items"].clone();
        assert_eq!(apply(&store, &keys, &update, NOW + 30), EventOutcome::Ignored);
        assert_eq!(keys.len(), 0);
        assert!(store.entitlement("sub_E").expect("read").is_none());
    }

    #[test]
    fn an_incomplete_subscription_creates_nothing_until_an_entitled_update() {
        let (store, keys) = stores();
        let created = subscription_event(
            "evt_1",
            "customer.subscription.created",
            "sub_F",
            NOW,
            serde_json::json!({ "status": "incomplete" }),
        );
        assert_eq!(apply(&store, &keys, &created, NOW), EventOutcome::Ignored);
        assert_eq!(keys.len(), 0);
        // The first payment settles; the update is the first entitled
        // sighting and creates, under the price guard.
        let update = subscription_event(
            "evt_2",
            "customer.subscription.updated",
            "sub_F",
            NOW + 60,
            serde_json::json!({ "status": "active" }),
        );
        assert_eq!(apply(&store, &keys, &update, NOW + 60), EventOutcome::Applied);
        let (_, secret) = claim_key(&store, &policy(), "sub_F", NOW + 60).expect("claim");
        assert!(keys.verify("sub_F", &secret).is_ok());
    }

    #[test]
    fn replayed_events_are_no_ops() {
        let (store, keys) = stores();
        let created = created_event("evt_1", "sub_D", NOW);
        apply(&store, &keys, &created, NOW);
        assert_eq!(apply(&store, &keys, &created, NOW), EventOutcome::Duplicate);
        let checkout = checkout_event("evt_2", "cs_D", "sub_D", NOW + 30);
        apply(&store, &keys, &checkout, NOW + 30);
        assert_eq!(apply(&store, &keys, &checkout, NOW + 30), EventOutcome::Duplicate);
        assert_eq!(keys.len(), 1, "one subscription, one key");
        let ent = store.entitlement("sub_D").expect("read").expect("entitlement");
        assert_eq!(ent.key_seq, 1);
    }

    #[test]
    fn updated_cancellation_schedules_revocation_at_the_period_end() {
        let (store, keys) = stores();
        purchase(&store, &keys, "sub_G", "cs_G");
        let (_, secret) = claim_key(&store, &policy(), "sub_G", NOW + 60).expect("claim");

        let ends = NOW + 20 * 86_400;
        let update = subscription_event(
            "evt_u",
            "customer.subscription.updated",
            "sub_G",
            NOW + 3600,
            serde_json::json!({
                "cancel_at_period_end": true,
                "current_period_end": ends,
            }),
        );
        assert_eq!(apply(&store, &keys, &update, NOW + 3600), EventOutcome::Applied);
        let ent = store.entitlement("sub_G").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_CANCEL_SCHEDULED);
        assert_eq!(ent.revoke_at, Some(ends));
        // Paid period still running: the key works, and the sweep agrees.
        assert!(keys.verify("sub_G", &secret).is_ok());
        assert_eq!(sweep(&store, &keys, ends - 1).expect("sweep").revoked, 0);
        assert!(keys.verify("sub_G", &secret).is_ok());
        // At the effective end the sweep revokes.
        assert_eq!(sweep(&store, &keys, ends).expect("sweep").revoked, 1);
        assert!(keys.verify("sub_G", &secret).is_err());
        let ent = store.entitlement("sub_G").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_REVOKED);
    }

    #[test]
    fn updated_withdraws_a_scheduled_cancellation() {
        let (store, keys) = stores();
        purchase(&store, &keys, "sub_H", "cs_H");
        let ends = NOW + 20 * 86_400;
        let cancel = subscription_event(
            "evt_u1",
            "customer.subscription.updated",
            "sub_H",
            NOW + 60,
            serde_json::json!({
                "cancel_at_period_end": true,
                "current_period_end": ends,
            }),
        );
        apply(&store, &keys, &cancel, NOW + 60);
        // The buyer undoes the cancellation: an update with the flag
        // retracted and an active status restores the entitlement.
        let undo = subscription_event(
            "evt_u2",
            "customer.subscription.updated",
            "sub_H",
            NOW + 120,
            serde_json::json!({ "cancel_at_period_end": false }),
        );
        assert_eq!(apply(&store, &keys, &undo, NOW + 120), EventOutcome::Applied);
        let ent = store.entitlement("sub_H").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_ACTIVE);
        assert_eq!(ent.revoke_at, None);
        assert_eq!(sweep(&store, &keys, ends + 1).expect("sweep").revoked, 0);
    }

    #[test]
    fn subscription_deleted_revokes_now() {
        let (store, keys) = stores();
        purchase(&store, &keys, "sub_I", "cs_I");
        let (_, secret) = claim_key(&store, &policy(), "sub_I", NOW + 60).expect("claim");
        let deleted = subscription_event(
            "evt_d",
            "customer.subscription.deleted",
            "sub_I",
            NOW + 120,
            serde_json::json!({ "status": "canceled" }),
        );
        assert_eq!(apply(&store, &keys, &deleted, NOW + 120), EventOutcome::Applied);
        assert!(keys.verify("sub_I", &secret).is_err());
        let ent = store.entitlement("sub_I").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_REVOKED);
    }

    #[test]
    fn past_due_grace_restore_and_expiry() {
        let (store, keys) = stores();
        purchase(&store, &keys, "sub_J", "cs_J");
        let (_, secret) = claim_key(&store, &policy(), "sub_J", NOW + 60).expect("claim");

        let due = subscription_event(
            "evt_u1",
            "customer.subscription.updated",
            "sub_J",
            NOW + 120,
            serde_json::json!({ "status": "past_due" }),
        );
        apply(&store, &keys, &due, NOW + 120);
        let ent = store.entitlement("sub_J").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_PAST_DUE);
        assert_eq!(ent.revoke_at, Some(NOW + 120 + PAST_DUE_GRACE_SECS));
        assert!(keys.verify("sub_J", &secret).is_ok(), "grace keeps the key live");

        // A repeat past-due does not restart the clock.
        let due2 = subscription_event(
            "evt_u2",
            "customer.subscription.updated",
            "sub_J",
            NOW + 180,
            serde_json::json!({ "status": "past_due" }),
        );
        apply(&store, &keys, &due2, NOW + 180);
        let ent = store.entitlement("sub_J").expect("read").expect("entitlement");
        assert_eq!(ent.revoke_at, Some(NOW + 120 + PAST_DUE_GRACE_SECS));

        // Returning to active clears the pending revocation.
        let back = subscription_event(
            "evt_u3",
            "customer.subscription.updated",
            "sub_J",
            NOW + 240,
            serde_json::json!({ "status": "active" }),
        );
        apply(&store, &keys, &back, NOW + 240);
        let ent = store.entitlement("sub_J").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_ACTIVE);
        assert_eq!(ent.revoke_at, None);
        assert_eq!(sweep(&store, &keys, NOW + 120 + PAST_DUE_GRACE_SECS + 1).expect("sweep").revoked, 0);
        assert!(keys.verify("sub_J", &secret).is_ok());
    }

    #[test]
    fn invoice_payment_failed_enters_grace_and_payment_succeeded_clears_it() {
        let (store, keys) = stores();
        purchase(&store, &keys, "sub_K", "cs_K");
        let (_, secret) = claim_key(&store, &policy(), "sub_K", NOW + 60).expect("claim");

        let failed = invoice_event("evt_f", "invoice.payment_failed", "sub_K", NOW + 120);
        assert_eq!(apply(&store, &keys, &failed, NOW + 120), EventOutcome::Applied);
        let ent = store.entitlement("sub_K").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_PAST_DUE);
        assert_eq!(ent.revoke_at, Some(NOW + 120 + PAST_DUE_GRACE_SECS));
        assert!(keys.verify("sub_K", &secret).is_ok(), "grace keeps the key live");

        let succeeded = invoice_event("evt_s", "invoice.payment_succeeded", "sub_K", NOW + 180);
        assert_eq!(apply(&store, &keys, &succeeded, NOW + 180), EventOutcome::Applied);
        let ent = store.entitlement("sub_K").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_ACTIVE);
        assert_eq!(ent.revoke_at, None);
        assert_eq!(sweep(&store, &keys, NOW + 120 + PAST_DUE_GRACE_SECS + 1).expect("sweep").revoked, 0);
        assert!(keys.verify("sub_K", &secret).is_ok());
    }

    #[test]
    fn past_due_past_grace_revokes_in_the_sweep() {
        let (store, keys) = stores();
        purchase(&store, &keys, "sub_L", "cs_L");
        let (_, secret) = claim_key(&store, &policy(), "sub_L", NOW + 60).expect("claim");
        let failed = invoice_event("evt_f", "invoice.payment_failed", "sub_L", NOW + 120);
        apply(&store, &keys, &failed, NOW + 120);
        let deadline = NOW + 120 + PAST_DUE_GRACE_SECS;
        assert_eq!(sweep(&store, &keys, deadline - 1).expect("sweep").revoked, 0);
        assert_eq!(sweep(&store, &keys, deadline).expect("sweep").revoked, 1);
        assert!(keys.verify("sub_L", &secret).is_err());
    }

    #[test]
    fn unpaid_status_also_enters_the_grace() {
        let (store, keys) = stores();
        purchase(&store, &keys, "sub_M", "cs_M");
        let unpaid = subscription_event(
            "evt_u",
            "customer.subscription.updated",
            "sub_M",
            NOW + 120,
            serde_json::json!({ "status": "unpaid" }),
        );
        apply(&store, &keys, &unpaid, NOW + 120);
        let ent = store.entitlement("sub_M").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_PAST_DUE);
        assert_eq!(ent.revoke_at, Some(NOW + 120 + PAST_DUE_GRACE_SECS));
    }

    #[test]
    fn a_revoked_subscription_restored_gets_a_replacement_key() {
        let (store, keys) = stores();
        purchase(&store, &keys, "sub_N", "cs_N");
        let (_, old_secret) = claim_key(&store, &policy(), "sub_N", NOW + 60).expect("claim");
        let deleted = subscription_event(
            "evt_d",
            "customer.subscription.deleted",
            "sub_N",
            NOW + 120,
            serde_json::json!({ "status": "canceled" }),
        );
        apply(&store, &keys, &deleted, NOW + 120);
        assert!(keys.verify("sub_N", &old_secret).is_err());

        let back = subscription_event(
            "evt_u",
            "customer.subscription.updated",
            "sub_N",
            NOW + 180,
            serde_json::json!({ "status": "active" }),
        );
        apply(&store, &keys, &back, NOW + 180);
        let ent = store.entitlement("sub_N").expect("read").expect("entitlement");
        assert_eq!(ent.key_id, "sub_N~2");
        assert_eq!(ent.status, STATUS_ACTIVE);
        let (key_id, new_secret) = claim_key(&store, &policy(), "sub_N", NOW + 180).expect("claim replacement");
        assert_eq!(key_id, "sub_N~2");
        assert_ne!(new_secret, old_secret);
        assert!(keys.verify("sub_N~2", &new_secret).is_ok());
        assert!(keys.verify("sub_N", &old_secret).is_err(), "the old key stays dead");
    }

    #[test]
    fn a_second_subscription_is_a_second_key() {
        let (store, keys) = stores();
        for sub in ["sub_O1", "sub_O2"] {
            let ev = created_event(&format!("evt_{sub}"), sub, NOW);
            apply(&store, &keys, &ev, NOW);
        }
        assert_eq!(keys.active_len(), 2);
    }

    /// A Guardrails purchase end to end: the checkout lands first,
    /// the creation mints under the Guardrails price, and the key
    /// the buyer claims is stamped with the Guardrails plan — on the
    /// key record and on the entitlement alike.
    #[test]
    fn guardrails_purchase_issues_a_guardrails_stamped_key() {
        let (store, keys) = stores();
        let pol = policy_two_tier();
        let checkout = checkout_event("evt_k", "cs_GR", "sub_GR", NOW);
        assert_eq!(apply_p(&pol, &store, &keys, &checkout, NOW), EventOutcome::Applied);
        assert_eq!(keys.len(), 0, "a checkout alone mints nothing");
        let created = created_event_priced("evt_c", "sub_GR", NOW + 30, GUARDRAILS_PRICE);
        assert_eq!(apply_p(&pol, &store, &keys, &created, NOW + 30), EventOutcome::Applied);
        let ent = store.entitlement("sub_GR").expect("read").expect("entitlement");
        assert_eq!(ent.plan, GUARDRAILS_PLAN);
        assert_eq!(ent.plan, "guardrails");
        let (key_id, secret) = claim_key(&store, &pol, "cs_GR", NOW + 60).expect("claim by session");
        assert_eq!(key_id, "sub_GR");
        let verified = keys.verify("sub_GR", &secret).expect("claimed key verifies");
        assert_eq!(verified.plan, "guardrails", "the key itself carries the tier");
    }

    /// With both tiers configured, a price that is neither is still
    /// another product's: creation and the update rescue both refuse.
    #[test]
    fn an_unknown_price_creates_nothing_with_both_tiers_configured() {
        let (store, keys) = stores();
        let pol = policy_two_tier();
        let created = created_event_priced("evt_1", "sub_U", NOW, "price_someone_else");
        assert_eq!(apply_p(&pol, &store, &keys, &created, NOW), EventOutcome::Ignored);
        let update = subscription_event_priced(
            "evt_2",
            "customer.subscription.updated",
            "sub_U",
            NOW + 30,
            "price_someone_else",
            serde_json::json!({}),
        );
        assert_eq!(apply_p(&pol, &store, &keys, &update, NOW + 30), EventOutcome::Ignored);
        assert_eq!(keys.len(), 0);
        assert!(store.entitlement("sub_U").expect("read").is_none());
    }

    /// Single-price behaviour is identical when the Guardrails price
    /// is unset: a subscription carrying that price id is another
    /// product's as far as this bridge is concerned.
    #[test]
    fn the_guardrails_price_is_not_ours_until_configured() {
        let (store, keys) = stores();
        let created = created_event_priced("evt_1", "sub_V", NOW, GUARDRAILS_PRICE);
        assert_eq!(apply(&store, &keys, &created, NOW), EventOutcome::Ignored);
        assert_eq!(keys.len(), 0);
        assert!(store.entitlement("sub_V").expect("read").is_none());
    }

    /// Revocation follows the subscription, not the tier: a
    /// Guardrails key dies on `subscription.deleted` exactly like a
    /// Team key does.
    #[test]
    fn subscription_deleted_revokes_a_guardrails_key() {
        let (store, keys) = stores();
        let pol = policy_two_tier();
        let created = created_event_priced("evt_c", "sub_GD", NOW, GUARDRAILS_PRICE);
        apply_p(&pol, &store, &keys, &created, NOW);
        let (_, secret) = claim_key(&store, &pol, "sub_GD", NOW + 60).expect("claim");
        assert!(keys.verify("sub_GD", &secret).is_ok());
        let deleted = subscription_event_priced(
            "evt_d",
            "customer.subscription.deleted",
            "sub_GD",
            NOW + 120,
            GUARDRAILS_PRICE,
            serde_json::json!({ "status": "canceled" }),
        );
        assert_eq!(apply_p(&pol, &store, &keys, &deleted, NOW + 120), EventOutcome::Applied);
        assert!(keys.verify("sub_GD", &secret).is_err());
        let ent = store.entitlement("sub_GD").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_REVOKED);
        assert_eq!(ent.plan, "guardrails", "the record keeps the tier after revocation");
    }

    /// Both tiers live side by side, each key stamped with its own
    /// tier; deleting one subscription leaves the other untouched.
    #[test]
    fn tiers_coexist_and_revocation_is_scoped_to_the_subscription() {
        let (store, keys) = stores();
        let pol = policy_two_tier();
        let team = created_event("evt_t", "sub_TM", NOW);
        apply_p(&pol, &store, &keys, &team, NOW);
        let guard = created_event_priced("evt_g", "sub_GX", NOW, GUARDRAILS_PRICE);
        apply_p(&pol, &store, &keys, &guard, NOW);
        let (_, team_secret) = claim_key(&store, &pol, "sub_TM", NOW + 60).expect("team claim");
        let (_, guard_secret) = claim_key(&store, &pol, "sub_GX", NOW + 60).expect("guardrails claim");
        assert_eq!(keys.verify("sub_TM", &team_secret).expect("team key").plan, "hosted");
        assert_eq!(
            keys.verify("sub_GX", &guard_secret).expect("guardrails key").plan,
            "guardrails"
        );
        let deleted = subscription_event(
            "evt_d",
            "customer.subscription.deleted",
            "sub_TM",
            NOW + 120,
            serde_json::json!({ "status": "canceled" }),
        );
        apply_p(&pol, &store, &keys, &deleted, NOW + 120);
        assert!(keys.verify("sub_TM", &team_secret).is_err(), "the deleted tier's key dies");
        assert!(keys.verify("sub_GX", &guard_secret).is_ok(), "the other tier is untouched");
    }

    /// A Guardrails subscription restored after revocation is
    /// re-issued at the Guardrails tier — the replacement takes the
    /// entitlement's recorded stamp, not the base plan.
    #[test]
    fn a_guardrails_replacement_keeps_the_guardrails_stamp() {
        let (store, keys) = stores();
        let pol = policy_two_tier();
        let created = created_event_priced("evt_c", "sub_GN", NOW, GUARDRAILS_PRICE);
        apply_p(&pol, &store, &keys, &created, NOW);
        let (_, old_secret) = claim_key(&store, &pol, "sub_GN", NOW + 60).expect("claim");
        let deleted = subscription_event_priced(
            "evt_d",
            "customer.subscription.deleted",
            "sub_GN",
            NOW + 120,
            GUARDRAILS_PRICE,
            serde_json::json!({ "status": "canceled" }),
        );
        apply_p(&pol, &store, &keys, &deleted, NOW + 120);
        assert!(keys.verify("sub_GN", &old_secret).is_err());
        let back = subscription_event_priced(
            "evt_u",
            "customer.subscription.updated",
            "sub_GN",
            NOW + 180,
            GUARDRAILS_PRICE,
            serde_json::json!({ "status": "active" }),
        );
        apply_p(&pol, &store, &keys, &back, NOW + 180);
        let ent = store.entitlement("sub_GN").expect("read").expect("entitlement");
        assert_eq!(ent.key_id, "sub_GN~2");
        assert_eq!(ent.plan, "guardrails");
        let (key_id, new_secret) = claim_key(&store, &pol, "sub_GN", NOW + 180).expect("claim replacement");
        assert_eq!(key_id, "sub_GN~2");
        let verified = keys.verify("sub_GN~2", &new_secret).expect("replacement verifies");
        assert_eq!(verified.plan, "guardrails");
    }

    /// A subscription that switches tier mid-life: the record
    /// follows the price it pays now, so the next replacement is
    /// issued at the new tier. (The live key's own stamp changes
    /// only when the key is replaced — the key store has no plan
    /// mutation, by design.)
    #[test]
    fn a_tier_switch_refreshes_the_record_for_the_next_key() {
        let (store, keys) = stores();
        let pol = policy_two_tier();
        let created = created_event("evt_c", "sub_SW", NOW);
        apply_p(&pol, &store, &keys, &created, NOW);
        let (_, secret) = claim_key(&store, &pol, "sub_SW", NOW + 60).expect("claim");
        assert_eq!(keys.verify("sub_SW", &secret).expect("team key").plan, "hosted");
        // The subscription moves to the Guardrails price.
        let switched = subscription_event_priced(
            "evt_u",
            "customer.subscription.updated",
            "sub_SW",
            NOW + 120,
            GUARDRAILS_PRICE,
            serde_json::json!({ "status": "active" }),
        );
        apply_p(&pol, &store, &keys, &switched, NOW + 120);
        let ent = store.entitlement("sub_SW").expect("read").expect("entitlement");
        assert_eq!(ent.plan, "guardrails", "the record follows the current price");
        assert_eq!(ent.key_id, "sub_SW", "no replacement while the key lives");
        // At the next replacement, the new stamp takes effect.
        let deleted = subscription_event_priced(
            "evt_d",
            "customer.subscription.deleted",
            "sub_SW",
            NOW + 180,
            GUARDRAILS_PRICE,
            serde_json::json!({ "status": "canceled" }),
        );
        apply_p(&pol, &store, &keys, &deleted, NOW + 180);
        let back = subscription_event_priced(
            "evt_r",
            "customer.subscription.updated",
            "sub_SW",
            NOW + 240,
            GUARDRAILS_PRICE,
            serde_json::json!({ "status": "active" }),
        );
        apply_p(&pol, &store, &keys, &back, NOW + 240);
        let (_, new_secret) = claim_key(&store, &pol, "sub_SW", NOW + 240).expect("claim replacement");
        let verified = keys.verify("sub_SW~2", &new_secret).expect("replacement verifies");
        assert_eq!(verified.plan, "guardrails");
    }

    #[test]
    fn stale_events_do_not_kill_a_live_key() {
        let (store, keys) = stores();
        let created = created_event("evt_1", "sub_P", NOW + 1200);
        apply(&store, &keys, &created, NOW + 1200);
        let (_, secret) = claim_key(&store, &policy(), "sub_P", NOW + 1200).expect("claim");
        // A deletion that *occurred* before the creation arrives late.
        let stale_delete = subscription_event(
            "evt_2",
            "customer.subscription.deleted",
            "sub_P",
            NOW,
            serde_json::json!({ "status": "canceled" }),
        );
        assert_eq!(apply(&store, &keys, &stale_delete, NOW + 2400), EventOutcome::Stale);
        assert!(keys.verify("sub_P", &secret).is_ok());
    }

    #[test]
    fn unclaimed_deliveries_expire_and_purge_their_plaintext() {
        let (store, keys) = stores();
        purchase(&store, &keys, "sub_Q", "cs_Q");
        purchase(&store, &keys, "sub_R", "cs_R");
        // Within the window the claim works.
        assert!(claim_key(&store, &policy(), "cs_Q", NOW + CLAIM_TTL_SECS - 1).is_ok());
        // Past it, the sweep purges and the claim is refused.
        let report = sweep(&store, &keys, NOW + CLAIM_TTL_SECS).expect("sweep");
        assert_eq!(report.purged_deliveries, 1);
        assert!(matches!(
            claim_key(&store, &policy(), "cs_R", NOW + CLAIM_TTL_SECS),
            Err(BridgeError::ClaimUnavailable)
        ));
    }

    /// Extracts the credential a key email carries, asserting its
    /// only appearances are the key itself and the worked first-call
    /// example — the same bar the composition tests set.
    fn emailed_credential(email: &crate::mail::KeyEmail, key_id: &str) -> String {
        let marker = format!("{key_id}:");
        let start = email.text.find(&marker).expect("credential in the email") + marker.len();
        let secret: String = email.text[start..]
            .chars()
            .take_while(|ch| ch.is_ascii_hexdigit())
            .collect();
        assert_eq!(secret.len(), 64, "the full secret, from the email");
        assert_eq!(
            email.text.matches(&format!("{marker}{secret}")).count(),
            2,
            "the credential appears as the key and in the first-call example only"
        );
        secret
    }

    #[test]
    fn the_key_email_goes_out_once_when_the_checkout_records_the_address() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        // Creation first: no address is known yet, so nothing sends.
        let created = created_event("evt_c", "sub_S", NOW);
        apply_with_mailer(&store, &keys, &mailer, &created, NOW);
        assert!(mailer.sent().is_empty());
        // The checkout records the address and completes the set.
        let checkout =
            checkout_event_with_email("evt_k", "cs_S", "sub_S", NOW + 30, "buyer@example.com");
        apply_with_mailer(&store, &keys, &mailer, &checkout, NOW + 30);
        assert_eq!(
            store.buyer_email("sub_S").expect("read").as_deref(),
            Some("buyer@example.com"),
            "the checkout's address is stored with the subscription"
        );
        let sent = mailer.sent();
        assert_eq!(sent.len(), 1, "exactly one key email");
        assert_eq!(sent[0].to, "buyer@example.com");
        let secret = emailed_credential(&sent[0], "sub_S");
        assert!(keys.verify("sub_S", &secret).is_ok(), "the emailed key works");
        let delivery = store.delivery("sub_S").expect("read").expect("delivery");
        assert_eq!(delivery.emailed_at, Some(NOW + 30));
        // Emailing did not consume the claim: the buyer can still
        // claim, and gets the same credential.
        let (_, claimed) = claim_key(&store, &policy(), "cs_S", NOW + 60).expect("claim");
        assert_eq!(claimed, secret);
    }

    #[test]
    fn the_key_email_goes_out_at_issuance_when_the_checkout_landed_first() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        let checkout =
            checkout_event_with_email("evt_k", "cs_T", "sub_T", NOW, "early@example.com");
        apply_with_mailer(&store, &keys, &mailer, &checkout, NOW);
        assert!(mailer.sent().is_empty(), "no key exists yet, so no key email");
        let created = created_event("evt_c", "sub_T", NOW + 30);
        apply_with_mailer(&store, &keys, &mailer, &created, NOW + 30);
        let sent = mailer.sent();
        assert_eq!(sent.len(), 1, "issuance completes the set and sends");
        assert_eq!(sent[0].to, "early@example.com");
        let secret = emailed_credential(&sent[0], "sub_T");
        assert!(keys.verify("sub_T", &secret).is_ok());
    }

    #[test]
    fn replayed_events_never_resend_the_key_email() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        let created = created_event("evt_c", "sub_U", NOW);
        let checkout =
            checkout_event_with_email("evt_k", "cs_U", "sub_U", NOW + 30, "buyer@example.com");
        apply_with_mailer(&store, &keys, &mailer, &created, NOW);
        apply_with_mailer(&store, &keys, &mailer, &checkout, NOW + 30);
        assert_eq!(mailer.sent().len(), 1);
        // Stripe redelivers both events (at-least-once): duplicates,
        // and the send count does not move.
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &checkout, NOW + 60),
            EventOutcome::Duplicate
        );
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &created, NOW + 60),
            EventOutcome::Duplicate
        );
        assert_eq!(mailer.sent().len(), 1, "no second send on replay");
        // A fresh, distinct event for the same subscription does not
        // re-send either: the delivery's marker is already set.
        let update = subscription_event(
            "evt_u",
            "customer.subscription.updated",
            "sub_U",
            NOW + 120,
            serde_json::json!({}),
        );
        apply_with_mailer(&store, &keys, &mailer, &update, NOW + 120);
        assert_eq!(mailer.sent().len(), 1);
    }

    #[test]
    fn a_failed_send_leaves_the_purchase_and_the_claim_intact() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        mailer.set_failing(true);
        let created = created_event("evt_c", "sub_V", NOW);
        let checkout =
            checkout_event_with_email("evt_k", "cs_V", "sub_V", NOW + 30, "buyer@example.com");
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &created, NOW),
            EventOutcome::Applied,
            "email failure never fails the purchase"
        );
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &checkout, NOW + 30),
            EventOutcome::Applied
        );
        assert!(mailer.sent().is_empty());
        let ent = store.entitlement("sub_V").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_ACTIVE);
        let delivery = store.delivery("sub_V").expect("read").expect("delivery");
        assert_eq!(delivery.emailed_at, None, "a failed send leaves the marker unset");
        // The claim path carries the purchase on its own.
        let (_, secret) = claim_key(&store, &policy(), "cs_V", NOW + 60).expect("claim");
        assert!(keys.verify("sub_V", &secret).is_ok());
    }

    #[test]
    fn a_failed_send_is_retried_by_a_later_event() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        mailer.set_failing(true);
        let created = created_event("evt_c", "sub_W", NOW);
        let checkout =
            checkout_event_with_email("evt_k", "cs_W", "sub_W", NOW + 30, "buyer@example.com");
        apply_with_mailer(&store, &keys, &mailer, &created, NOW);
        apply_with_mailer(&store, &keys, &mailer, &checkout, NOW + 30);
        assert!(mailer.sent().is_empty());
        mailer.set_failing(false);
        let update = subscription_event(
            "evt_u",
            "customer.subscription.updated",
            "sub_W",
            NOW + 120,
            serde_json::json!({}),
        );
        apply_with_mailer(&store, &keys, &mailer, &update, NOW + 120);
        assert_eq!(mailer.sent().len(), 1, "the retry sends once the transport recovers");
    }

    #[test]
    fn a_checkout_without_an_address_sends_nothing() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        let created = created_event("evt_c", "sub_X2", NOW);
        let checkout = checkout_event("evt_k", "cs_X2", "sub_X2", NOW + 30);
        apply_with_mailer(&store, &keys, &mailer, &created, NOW);
        apply_with_mailer(&store, &keys, &mailer, &checkout, NOW + 30);
        assert!(mailer.sent().is_empty());
        assert!(store.buyer_email("sub_X2").expect("read").is_none());
        // And the purchase is whole: the claim still delivers the key.
        let (_, secret) = claim_key(&store, &policy(), "cs_X2", NOW + 60).expect("claim");
        assert!(keys.verify("sub_X2", &secret).is_ok());
    }

    #[test]
    fn a_replacement_key_is_emailed_as_its_own_issuance() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        let created = created_event("evt_c", "sub_Y", NOW);
        let checkout =
            checkout_event_with_email("evt_k", "cs_Y", "sub_Y", NOW + 30, "buyer@example.com");
        apply_with_mailer(&store, &keys, &mailer, &created, NOW);
        apply_with_mailer(&store, &keys, &mailer, &checkout, NOW + 30);
        assert_eq!(mailer.sent().len(), 1);
        let deleted = subscription_event(
            "evt_d",
            "customer.subscription.deleted",
            "sub_Y",
            NOW + 120,
            serde_json::json!({ "status": "canceled" }),
        );
        apply_with_mailer(&store, &keys, &mailer, &deleted, NOW + 120);
        let back = subscription_event(
            "evt_u",
            "customer.subscription.updated",
            "sub_Y",
            NOW + 180,
            serde_json::json!({ "status": "active" }),
        );
        apply_with_mailer(&store, &keys, &mailer, &back, NOW + 180);
        let sent = mailer.sent();
        assert_eq!(sent.len(), 2, "the replacement key is a new issuance and is emailed");
        let secret = emailed_credential(&sent[1], "sub_Y~2");
        assert!(keys.verify("sub_Y~2", &secret).is_ok());
    }

    #[test]
    fn concurrent_events_send_the_key_email_once() {
        let (store, keys) = stores();
        // The purchase converges with no mailer attached, so the key
        // email is still owed when the race begins.
        let created = created_event("evt_c", "sub_Z", NOW);
        assert_eq!(apply(&store, &keys, &created, NOW), EventOutcome::Applied);
        let checkout =
            checkout_event_with_email("evt_k", "cs_Z", "sub_Z", NOW + 30, "buyer@example.com");
        assert_eq!(apply(&store, &keys, &checkout, NOW + 30), EventOutcome::Applied);

        let mailer = SlowMailer::new();
        let first = subscription_event(
            "evt_u1",
            "customer.subscription.updated",
            "sub_Z",
            NOW + 60,
            serde_json::json!({}),
        );
        let second = subscription_event(
            "evt_u2",
            "customer.subscription.updated",
            "sub_Z",
            NOW + 60,
            serde_json::json!({}),
        );
        // Two distinct events for one subscription, processed at the
        // same time, with a send slow enough for both to reach the
        // hook together. The atomic marker claim lets exactly one
        // of them send (D-3: the old check → send → mark sequence
        // sent twice here).
        std::thread::scope(|scope| {
            let a = scope.spawn(|| {
                apply_event(&store, &keys, &policy(), Some(&mailer), &first, NOW + 60)
            });
            let b = scope.spawn(|| {
                apply_event(&store, &keys, &policy(), Some(&mailer), &second, NOW + 60)
            });
            a.join().expect("first event thread").expect("first event applies");
            b.join().expect("second event thread").expect("second event applies");
        });
        let sent = mailer.sent();
        assert_eq!(sent.len(), 1, "exactly one key email under concurrency");
        assert_eq!(sent[0].to, "buyer@example.com");
        let delivery = store.delivery("sub_Z").expect("read").expect("delivery");
        assert_eq!(delivery.emailed_at, Some(NOW + 60));
    }

    /// A purchase with the mailer attached throughout and the
    /// buyer's address recorded at checkout: entitlement, key, and
    /// the key email — and no lifecycle news yet.
    fn purchase_with_mailer(
        store: &BridgeStore,
        keys: &KeyStore,
        mailer: &RecordingMailer,
        sub: &str,
        session: &str,
    ) {
        let created = created_event(&format!("evt_c_{sub}"), sub, NOW);
        assert_eq!(
            apply_with_mailer(store, keys, mailer, &created, NOW),
            EventOutcome::Applied
        );
        let checkout = checkout_event_with_email(
            &format!("evt_k_{sub}"),
            session,
            sub,
            NOW + 30,
            "buyer@example.com",
        );
        assert_eq!(
            apply_with_mailer(store, keys, mailer, &checkout, NOW + 30),
            EventOutcome::Applied
        );
        assert_eq!(mailer.sent().len(), 1, "the key email went out");
        assert!(
            mailer.lifecycle().is_empty(),
            "a purchase carries no lifecycle news"
        );
    }

    #[test]
    fn a_scheduled_cancellation_emails_the_buyer_the_period_end() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        purchase_with_mailer(&store, &keys, &mailer, "sub_LC1", "cs_LC1");
        let (_, secret) = claim_key(&store, &policy(), "sub_LC1", NOW + 60).expect("claim");

        let ends = NOW + 20 * 86_400;
        let update = subscription_event(
            "evt_u_lc1",
            "customer.subscription.updated",
            "sub_LC1",
            NOW + 3600,
            serde_json::json!({
                "cancel_at_period_end": true,
                "current_period_end": ends,
            }),
        );
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &update, NOW + 3600),
            EventOutcome::Applied
        );
        let lifecycle = mailer.lifecycle();
        assert_eq!(lifecycle.len(), 1, "one cancellation email");
        assert_eq!(lifecycle[0].to, "buyer@example.com");
        assert!(lifecycle[0].subject.contains("cancelled"));
        assert!(
            lifecycle[0]
                .text
                .contains(&format!("keeps working until {}", crate::mail::date_line(ends))),
            "the email states the entitlement's own period end"
        );
        // The news matches the state: the paid period is still live.
        assert!(keys.verify("sub_LC1", &secret).is_ok());
    }

    #[test]
    fn subscription_deleted_emails_the_buyer_that_access_has_ended() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        purchase_with_mailer(&store, &keys, &mailer, "sub_LC2", "cs_LC2");
        let (_, secret) = claim_key(&store, &policy(), "sub_LC2", NOW + 60).expect("claim");

        let deleted = subscription_event(
            "evt_d_lc2",
            "customer.subscription.deleted",
            "sub_LC2",
            NOW + 120,
            serde_json::json!({ "status": "canceled" }),
        );
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &deleted, NOW + 120),
            EventOutcome::Applied
        );
        let lifecycle = mailer.lifecycle();
        assert_eq!(lifecycle.len(), 1, "one ended email");
        assert!(lifecycle[0].subject.contains("ended"));
        assert!(lifecycle[0].text.contains("no longer works"));
        assert!(keys.verify("sub_LC2", &secret).is_err(), "the key is dead");
    }

    #[test]
    fn an_immediate_cancellation_emails_the_ending_not_the_scheduling() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        purchase_with_mailer(&store, &keys, &mailer, "sub_LC3", "cs_LC3");

        // The subscription reports itself canceled with the period
        // already over: the handler revokes now, and the email must
        // say the subscription ended — not that it will end.
        let update = subscription_event(
            "evt_u_lc3",
            "customer.subscription.updated",
            "sub_LC3",
            NOW + 120,
            serde_json::json!({
                "status": "canceled",
                "current_period_end": NOW + 60,
            }),
        );
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &update, NOW + 120),
            EventOutcome::Applied
        );
        let lifecycle = mailer.lifecycle();
        assert_eq!(lifecycle.len(), 1, "one ended email");
        assert!(lifecycle[0].subject.contains("ended"));
        let ent = store.entitlement("sub_LC3").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_REVOKED);
    }

    #[test]
    fn the_first_payment_failure_emails_once_and_a_repeat_does_not() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        purchase_with_mailer(&store, &keys, &mailer, "sub_LC4", "cs_LC4");

        let failed = invoice_event("evt_f1_lc4", "invoice.payment_failed", "sub_LC4", NOW + 120);
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &failed, NOW + 120),
            EventOutcome::Applied
        );
        let lifecycle = mailer.lifecycle();
        assert_eq!(lifecycle.len(), 1, "one payment-failed email");
        assert!(lifecycle[0].subject.contains("payment failed"));
        let grace_ends = NOW + 120 + PAST_DUE_GRACE_SECS;
        assert!(
            lifecycle[0]
                .text
                .contains(&format!("until {}", crate::mail::date_line(grace_ends))),
            "the email states the grace end the entitlement carries"
        );

        // A second failure signal is a distinct event and applies —
        // but the grace clock never restarts, and neither does the
        // email.
        let failed2 = invoice_event("evt_f2_lc4", "invoice.payment_failed", "sub_LC4", NOW + 180);
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &failed2, NOW + 180),
            EventOutcome::Applied
        );
        assert_eq!(mailer.lifecycle().len(), 1, "no second failure email");
    }

    #[test]
    fn a_past_due_update_emails_the_payment_failure() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        purchase_with_mailer(&store, &keys, &mailer, "sub_LC5", "cs_LC5");

        let due = subscription_event(
            "evt_u_lc5",
            "customer.subscription.updated",
            "sub_LC5",
            NOW + 120,
            serde_json::json!({ "status": "past_due" }),
        );
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &due, NOW + 120),
            EventOutcome::Applied
        );
        let lifecycle = mailer.lifecycle();
        assert_eq!(lifecycle.len(), 1, "one payment-failed email");
        assert!(lifecycle[0].subject.contains("payment failed"));
    }

    #[test]
    fn lifecycle_news_without_a_recorded_address_sends_nothing() {
        let (store, keys) = stores();
        // A purchase whose checkout carried no address: the whole
        // flow converges (this is the pre-address-capture shape),
        // and the later lifecycle events have nowhere to write to.
        purchase(&store, &keys, "sub_LC6", "cs_LC6");
        let mailer = RecordingMailer::new();
        let deleted = subscription_event(
            "evt_d_lc6",
            "customer.subscription.deleted",
            "sub_LC6",
            NOW + 120,
            serde_json::json!({ "status": "canceled" }),
        );
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &deleted, NOW + 120),
            EventOutcome::Applied,
            "the event still applies; only the email has nowhere to go"
        );
        assert!(mailer.lifecycle().is_empty());
        assert!(mailer.sent().is_empty());
    }

    #[test]
    fn a_refund_emails_the_buyer_and_moves_no_state() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        purchase_with_email(&store, &keys, &mailer, "sub_RF1", "cs_RF1", "buyer@example.com");
        // The settled invoice is what links the coming charge to
        // this subscription.
        let invoice = invoice_event_for(
            "evt_i_rf1",
            "invoice.payment_succeeded",
            "in_RF1",
            "sub_RF1",
            NOW + 60,
        );
        assert_eq!(apply(&store, &keys, &invoice, NOW + 60), EventOutcome::Applied);
        let refund = refund_event(
            "evt_r_rf1",
            "ch_RF1",
            Some("in_RF1"),
            1900,
            1900,
            serde_json::json!([
                { "id": "re_RF1", "amount": 1900, "created": NOW + 120, "status": "succeeded" },
            ]),
            NOW + 120,
        );
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &refund, NOW + 120),
            EventOutcome::Applied
        );
        let lifecycle = mailer.lifecycle();
        assert_eq!(lifecycle.len(), 1, "exactly the refund email");
        let email = &lifecycle[0];
        assert_eq!(email.to, "buyer@example.com");
        assert!(email.subject.contains("refund"), "the subject carries the news");
        assert!(email.text.contains("19.00 USD"), "the amount: {}", email.text);
        assert!(
            email.text.contains("codetopo Team (US$19/month)"),
            "the plan the charge was for"
        );
        assert!(
            email.text.contains("does not change your API key"),
            "a refund is not a cancellation, stated plainly"
        );
        assert!(email.text.contains("info@aimlds.org"), "the support route is stated");
        // No state moved: the entitlement stands active and the key
        // still claims and verifies.
        let ent = store.entitlement("sub_RF1").expect("read").expect("entitlement");
        assert_eq!(ent.status, STATUS_ACTIVE);
        let (key_id, secret) = claim_key(&store, &policy(), "sub_RF1", NOW + 180).expect("claim");
        assert!(keys.verify(&key_id, &secret).is_ok(), "the refunded key still works");
        // A redelivery of the same event is a duplicate and never
        // re-emails.
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &refund, NOW + 240),
            EventOutcome::Duplicate
        );
        assert_eq!(mailer.lifecycle().len(), 1, "the replay sent nothing");
    }

    #[test]
    fn a_refund_after_the_subscription_ended_says_the_key_has_stopped() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        purchase_with_email(&store, &keys, &mailer, "sub_RF2", "cs_RF2", "buyer@example.com");
        let invoice = invoice_event_for(
            "evt_i_rf2",
            "invoice.payment_succeeded",
            "in_RF2",
            "sub_RF2",
            NOW + 60,
        );
        assert_eq!(apply(&store, &keys, &invoice, NOW + 60), EventOutcome::Applied);
        // The Director's own order of acts: cancel first (the
        // subscription ends, the ended email goes out), refund after.
        let deleted = subscription_event(
            "evt_d_rf2",
            "customer.subscription.deleted",
            "sub_RF2",
            NOW + 90,
            serde_json::json!({ "status": "canceled" }),
        );
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &deleted, NOW + 90),
            EventOutcome::Applied
        );
        let refund = refund_event(
            "evt_r_rf2",
            "ch_RF2",
            Some("in_RF2"),
            1900,
            1900,
            serde_json::json!([
                { "id": "re_RF2", "amount": 1900, "created": NOW + 120, "status": "succeeded" },
            ]),
            NOW + 120,
        );
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &refund, NOW + 120),
            EventOutcome::Applied
        );
        let lifecycle = mailer.lifecycle();
        assert_eq!(lifecycle.len(), 2, "the ended email, then the refund email");
        let email = &lifecycle[1];
        assert!(email.subject.contains("refund"));
        assert!(
            email.text.contains("already stopped working"),
            "the key's fate is the ended subscription's truth: {}",
            email.text
        );
    }

    #[test]
    fn a_refund_that_resolves_nowhere_is_ignored_quietly() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        let one_refund = serde_json::json!([
            { "id": "re_X", "amount": 500, "created": NOW, "status": "succeeded" },
        ]);
        // An invoice this bridge has never seen (another product's
        // charge on the shared account).
        let stray = refund_event("evt_r_stray", "ch_X1", Some("in_theirs"), 500, 500, one_refund.clone(), NOW);
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &stray, NOW),
            EventOutcome::Ignored
        );
        // An invoice link exists — an invoice event named it — but
        // the subscription was never entitled.
        let failed = invoice_event_for("evt_i_ghost", "invoice.payment_failed", "in_ghost", "sub_ghost", NOW);
        assert_eq!(apply(&store, &keys, &failed, NOW), EventOutcome::Ignored);
        let ghost = refund_event("evt_r_ghost", "ch_X2", Some("in_ghost"), 500, 500, one_refund.clone(), NOW + 10);
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &ghost, NOW + 10),
            EventOutcome::Ignored
        );
        // A charge that settled no invoice at all.
        let bare = refund_event("evt_r_bare", "ch_X3", None, 500, 500, one_refund, NOW + 20);
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &bare, NOW + 20),
            EventOutcome::Ignored
        );
        assert!(mailer.lifecycle().is_empty(), "no email for any of them");
        assert!(mailer.sent().is_empty());
    }

    #[test]
    fn a_partial_refund_states_its_amount_and_a_second_refund_emails_its_own() {
        let (store, keys) = stores();
        let mailer = RecordingMailer::new();
        let created = created_event_priced("evt_c_rf4", "sub_RF4", NOW, GUARDRAILS_PRICE);
        assert_eq!(
            apply_p(&policy_two_tier(), &store, &keys, &created, NOW),
            EventOutcome::Applied
        );
        let checkout =
            checkout_event_with_email("evt_k_rf4", "cs_RF4", "sub_RF4", NOW + 30, "pro@example.com");
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &checkout, NOW + 30),
            EventOutcome::Applied
        );
        let invoice = invoice_event_for(
            "evt_i_rf4",
            "invoice.payment_succeeded",
            "in_RF4",
            "sub_RF4",
            NOW + 60,
        );
        assert_eq!(apply(&store, &keys, &invoice, NOW + 60), EventOutcome::Applied);
        let first = refund_event(
            "evt_r1_rf4",
            "ch_RF4",
            Some("in_RF4"),
            4900,
            2000,
            serde_json::json!([
                { "id": "re_1", "amount": 2000, "created": NOW + 120, "status": "succeeded" },
            ]),
            NOW + 120,
        );
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &first, NOW + 120),
            EventOutcome::Applied
        );
        // A second, distinct refund against the same charge: the
        // cumulative total moves to 3000, and the newest refund —
        // the one this event announces — is 1000.
        let second = refund_event(
            "evt_r2_rf4",
            "ch_RF4",
            Some("in_RF4"),
            4900,
            3000,
            serde_json::json!([
                { "id": "re_2", "amount": 1000, "created": NOW + 200, "status": "succeeded" },
                { "id": "re_1", "amount": 2000, "created": NOW + 120, "status": "succeeded" },
            ]),
            NOW + 200,
        );
        assert_eq!(
            apply_with_mailer(&store, &keys, &mailer, &second, NOW + 200),
            EventOutcome::Applied
        );
        let lifecycle = mailer.lifecycle();
        assert_eq!(lifecycle.len(), 2, "one email per refund event");
        assert_eq!(lifecycle[0].to, "pro@example.com");
        assert!(
            lifecycle[0].text.contains("20.00 USD of the 49.00 USD charge"),
            "the partial amount and the whole charge: {}",
            lifecycle[0].text
        );
        assert!(lifecycle[0].text.contains("partial refund"));
        assert!(
            lifecycle[0].text.contains("codetopo Guardrails (US$49/month)"),
            "the Guardrails plan is named"
        );
        assert!(
            lifecycle[1].text.contains("10.00 USD of the 49.00 USD charge"),
            "the second email states its own refund's amount, not the cumulative total"
        );
    }
}
