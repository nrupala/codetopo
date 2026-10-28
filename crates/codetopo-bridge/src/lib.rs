// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! The Stripe billing bridge (paid-path plan S4, gap G6).
//!
//! `codetopo-server` sells the hosted tier; this binary is how a stranger
//! paying through Stripe becomes a key in the server's runtime key store
//! — and how the key stops working when the money stops. It follows the
//! estate's proven pattern (the Conduit Router billing webhook, ADR-0008
//! in that repo): signature-verified webhooks, price ids as configuration,
//! entitlements bound to the Stripe **subscription id** and nothing else
//! (never an email address — Conduit's email-squatting hijack is why),
//! and idempotency by provider event id.
//!
//! ## Shape
//!
//! The bridge is a *separate binary* with its own small HTTP listener,
//! so the API server gains no new surface: S1's posture (key
//! administration is never an HTTP endpoint on `codetopo-server`) is
//! untouched. Every key action goes through
//! [`codetopo_server::keys::KeyStore`] — the same library code the
//! server's admin subcommand uses — against the same `keys.db` file, so
//! an issued or revoked key is visible to the running server on its next
//! request with no restart and no signalling. The bridge's own state
//! (processed events, entitlements, key deliveries) lives in additional
//! tables in that same file; the server only ever reads `api_keys` and
//! is unaware of them.
//!
//! ## The claim flow (how a buyer receives a key)
//!
//! The key store keeps only salted hashes (S1), so a key's plaintext
//! exists exactly once: the moment [`KeyStore::issue`] returns it. The
//! bridge seals that plaintext with AES-256-GCM under a key that exists
//! only in the bridge's environment (`CODETOPO_CLAIM_KEY`) and stores
//! the ciphertext as a *delivery*, addressable by claim tokens. Two
//! tokens are registered per delivery: the checkout's **session id**
//! (attached when `checkout.session.completed` is processed, or drained
//! from the pending record if the checkout landed first) and the
//! **subscription id** (known at issuance; it is also the id on the
//! buyer's Stripe receipt). The Stripe checkout success redirect lands
//! on the bridge's claim page carrying the session id (the
//! `{CHECKOUT_SESSION_ID}` placeholder in the success URL); the page
//! exchanges it — once — for the credential. Claiming, or seven
//! days passing unclaimed, destroys the ciphertext and every token; the
//! hash in `api_keys` is all that remains. A sweep (at startup and every
//! minute) applies due revocations and purges expired deliveries.
//!
//! ## Key delivery by email (owner directive, 2026-10-18)
//!
//! Stripe's receipt proves payment; it does not carry the key. So
//! when SMTP submission is configured (`CODETOPO_SMTP_*`, see
//! [`config::MailConfig`]), the bridge also *emails* the credential
//! to the address the checkout session carried
//! (`customer_details.email`, falling back to `customer_email`),
//! captured by the checkout handler and stored per subscription —
//! the record a support resend would work from. The send happens
//! once per issuance, at the moment the entitlement, the live
//! delivery, and the address have all converged (either the
//! checkout or the creation event can complete that set), and the
//! delivery row carries the send marker, so replays and later
//! events can never send it twice; a replacement key is a new
//! issuance and is emailed as one. Email is a second *delivery
//! channel* only — never an authority: nothing in the bridge
//! accepts an address as proof of anything, claims still run on
//! the tokens, and a failed or missing send changes nothing about
//! the purchase (the failure is logged with its event id, and the
//! claim page still holds the key). Unconfigured, the bridge is
//! claim-page-only, exactly as before.
//!
//! Threat model, plainly: a claim token is a bearer capability. Whoever
//! presents an unclaimed token inside its seven-day window gets the key,
//! once. The tokens are Stripe's own 128-bit-class ids, disclosed only
//! to the buyer (checkout redirect, receipt); the window is bounded by
//! single use and the seven-day purge, and a leaked `keys.db` alone
//! yields nothing — deliveries are ciphertext under an environment key,
//! and keys are hashes. The accepted residual is the redirect URL itself
//! sitting in browser history, the same exposure class as every
//! receipt-link claim flow (Conduit's included). Email delivery adds
//! one more copy of the credential in transit — the buyer's mailbox —
//! accepted on the owner's explicit instruction (buyers expect the
//! key to arrive like the receipt did); it does not widen authority:
//! binding *claims* to an email remains deliberately unbuilt, because
//! email is a client-writable hint at checkout and must never become
//! an authority (ADR-0008 amendment, 2026-10-06).
//!
//! ## Event semantics
//!
//! Signature first: the `Stripe-Signature` header is HMAC-verified over
//! the raw body, inside a five-minute timestamp tolerance, before any
//! JSON is trusted (see [`stripe`]); an invalid signature is a `400` and
//! changes nothing. Then, per event:
//!
//! | event | action |
//! |---|---|
//! | `customer.subscription.created` (carrying the codetopo price, entitled status) | ensure a `hosted` key for the subscription (issue on first sight; a replacement if the previous key was revoked) |
//! | `checkout.session.completed` (`subscription` mode, `payment_status: paid`) | register the session id as a claim token for the key's delivery — attached immediately, or recorded as pending when the subscription event has not created the entitlement yet; never creates (the payload carries no price) |
//! | `customer.subscription.updated` | active/trialing → restore: clear pending revocations, replace a revoked key (an entitled update for an unknown subscription creates under the same price guard — Stripe can deliver `created` before the first payment settles); past_due/unpaid → revoke after a 7-day grace unless restored first; `cancel_at_period_end` → revoke at the period end; canceled → revoke at the period end |
//! | `customer.subscription.deleted` | revoke now |
//! | `invoice.payment_failed` | mark past due: the same 7-day grace |
//! | `invoice.payment_succeeded` | clear the grace when the entitlement is in it |
//! | anything else, or another product's price | recorded, no state change |
//!
//! The webhook endpoint must be subscribed to all of the event types
//! above — in particular `customer.subscription.created`: without it,
//! checkouts record claim tokens and no entitlement is ever created.
//!
//! Replays are no-ops (processed event ids), and an event older than
//! the last one applied to its subscription is stale and skipped, so
//! Stripe's at-least-once, unordered delivery cannot resurrect or kill
//! a key out of turn. One subscription maps to one live key at a time;
//! a second subscription is a second key.

pub mod config;
pub mod crypto;
pub mod http;
pub mod mail;
pub mod service;
pub mod store;
pub mod stripe;

use codetopo_server::error::ApiError;

/// Failures inside the bridge. Never carries a secret, a claim token, or
/// a signature: messages are safe to log on the bridge host.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// Configuration is missing or unusable; the bridge refuses to boot.
    #[error("configuration: {0}")]
    Config(String),
    /// The bridge's own tables in the key store failed.
    #[error("bridge store: {0}")]
    Store(String),
    /// A key-store operation failed (wraps the server crate's error).
    #[error("key store: {0}")]
    Key(#[from] ApiError),
    /// Sealing or opening a key delivery failed.
    #[error("claim crypto: {0}")]
    Crypto(String),
    /// A claim was refused. One variant for every refusal — unknown,
    /// already claimed, and expired tokens are indistinguishable to the
    /// claimant, so the claim endpoint is not an oracle for which
    /// tokens exist.
    #[error("this claim is no longer available")]
    ClaimUnavailable,
}
