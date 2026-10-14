// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! The Paddle billing bridge (paid-path plan S4, gap G6).
//!
//! `codetopo-server` sells the hosted tier; this binary is how a stranger
//! paying through Paddle becomes a key in the server's runtime key store
//! — and how the key stops working when the money stops. It follows the
//! estate's proven pattern (the Conduit Router billing webhook, ADR-0008
//! in that repo): signature-verified webhooks, price ids as configuration,
//! entitlements bound to the Paddle **subscription id** and nothing else
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
//! tokens are registered per delivery: the checkout's **transaction id**
//! (attached when `transaction.completed` is processed) and the
//! **subscription id** (known at issuance; it is also the id on the
//! buyer's Paddle receipt). The Paddle checkout success redirect lands
//! on the bridge's claim page carrying the transaction id (`_ptxn`); the
//! page exchanges it — once — for the credential. Claiming, or seven
//! days passing unclaimed, destroys the ciphertext and every token; the
//! hash in `api_keys` is all that remains. A sweep (at startup and every
//! minute) applies due revocations and purges expired deliveries.
//!
//! Threat model, plainly: a claim token is a bearer capability. Whoever
//! presents an unclaimed token inside its seven-day window gets the key,
//! once. The tokens are Paddle's own 128-bit-class ids, disclosed only
//! to the buyer (checkout redirect, receipt); the window is bounded by
//! single use and the seven-day purge, and a leaked `keys.db` alone
//! yields nothing — deliveries are ciphertext under an environment key,
//! and keys are hashes. The accepted residual is the redirect URL itself
//! sitting in browser history, the same exposure class as every
//! receipt-link claim flow (Conduit's included); binding claims to a
//! verified email is the documented future narrowing, deliberately not
//! built here — email is a client-writable hint at checkout and must
//! never become an authority (ADR-0008 amendment, 2026-10-06).
//!
//! ## Event semantics
//!
//! Signature first: the `Paddle-Signature` header is HMAC-verified over
//! the raw body before any JSON is trusted (see [`paddle`]); an invalid
//! signature is a `400` and changes nothing. Then, per event:
//!
//! | event | action |
//! |---|---|
//! | `subscription.created` / `subscription.activated` | ensure a `hosted` key for the subscription (issue on first sight; a replacement if the previous key was revoked); clear any past-due / cancel-scheduled marks |
//! | `transaction.completed` (carrying the codetopo price + a subscription id) | same ensure; register the transaction id as a claim token for the key's delivery |
//! | `subscription.canceled` | revoke at the effective end (scheduled change / billing-period end): store the scheduled revocation; the key keeps working until then. Effective now → revoke now |
//! | `subscription.past_due` | mark past due; revoke after a 7-day grace unless the subscription returns to active first |
//! | `subscription.resumed`, `subscription.updated` to active | restore: clear pending revocations; a key already revoked is replaced |
//! | anything else, or another product's price | recorded, no state change |
//!
//! Replays are no-ops (processed event ids), and an event older than
//! the last one applied to its subscription is stale and skipped, so
//! Paddle's at-least-once, unordered delivery cannot resurrect or kill
//! a key out of turn. One subscription maps to one live key at a time;
//! a second subscription is a second key.

pub mod config;
pub mod crypto;
pub mod http;
pub mod paddle;
pub mod service;
pub mod store;

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
