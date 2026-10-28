// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Per-plan limits and the request envelope (paid-path plan S3, gaps
//! G4 and G5).
//!
//! Until now the server had no abuse envelope at all: no quota, no
//! timeout, no concurrency bound — safe only because the one caller
//! was the operator. This module is the table every limit is read
//! from, keyed by the plan recorded on the key (see [`crate::keys`]),
//! so a limit lives in exactly one place instead of being a literal
//! scattered through the handlers that enforce it.
//!
//! Two plans exist. `self-hosted` is the operator's own keys: every
//! limit is `None`, the envelope does not apply — an operator is
//! trusted with their own box. `hosted` is the paid tier from the
//! plan: US$19/month per key, 25 active indexes, 10,000 queries per
//! month, a 50,000-file cap per index, and the ingest caps S2 shipped
//! as interim constants (now this table's hosted row). A plan name
//! the table does not know — an admin can type anything after
//! `--plan` — resolves to the **hosted** row, never the operator's:
//! an unrecognized plan must fail toward the tenant envelope, not
//! toward unlimited.
//!
//! "Per month" is the **UTC calendar month**: the metering log's
//! timestamps are UTC, a key's usage in a month is the count of its
//! records whose `ts` falls in that month, and the quota resets when
//! the UTC month rolls over. There is no rolling window and no
//! per-key anniversary — the count is the log, so the rule had to be
//! one a reader of the log can apply with a text search.
//!
//! Two bounds are not per-plan because they protect the host, not a
//! tenant: [`GLOBAL_MAX_CONCURRENT_QUERIES`] caps concurrent query
//! requests across all keys. A query loads its whole graph into RAM
//! (~121 MB at 46k nodes by the project's scale gate; a hosted index
//! at the 50,000-file cap extrapolates to roughly 600 MB), and the
//! Aetheris host has 4 vCPUs and ~10 GB of headroom shared with a
//! local-model stack — 4 concurrent loads is one per vCPU, worst
//! case ≈2.4 GB of graph RAM, and more concurrency would only queue
//! CPU-bound work behind itself.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::auth::KeyPlan;
use crate::error::ApiError;
use crate::ingest::IngestCaps;
use crate::keys::SELF_HOSTED_PLAN;

/// Active indexes a hosted key may hold at once (the plan's offer).
pub const HOSTED_MAX_ACTIVE_INDEXES: usize = 25;

/// Metered queries a hosted key may make per UTC calendar month.
pub const HOSTED_MAX_QUERIES_PER_MONTH: u64 = 10_000;

/// Files one hosted index may be built from (the plan's repo cap).
pub const HOSTED_MAX_FILES_PER_INDEX: usize = 50_000;

/// Wall-clock budget for one hosted query. The scale gate's queries
/// answer in well under a second at 46k nodes; 30 s is generous
/// headroom over the largest graph the hosted file cap admits, while
/// still bounding how long one request may hold a graph-load slot.
pub const HOSTED_QUERY_TIMEOUT_SECS: u64 = 30;

/// Index operations one hosted key may run at a time. One, on
/// purpose: the permit is held from the active-index quota check
/// through the ownership record, so a key's creations are serialised
/// and the quota cannot be raced past; and on a shared 4-vCPU box,
/// one tenant's extraction never crowds out another's queries.
pub const HOSTED_MAX_CONCURRENT_INDEX_OPS: usize = 1;

/// Concurrent query requests admitted across all keys and plans —
/// the graph-load semaphore of gap G5. See the module docs for the
/// host arithmetic. Requests past the cap wait for a slot rather
/// than being refused: a query is cheap to queue and expensive to
/// retry, the opposite of an index operation.
pub const GLOBAL_MAX_CONCURRENT_QUERIES: usize = 4;

/// One plan's limits, resolved. `None` is always "no limit" — the
/// field is absent from the envelope, not set to a large number, so
/// an unlimited plan pays no bookkeeping for limits it does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanLimits {
    /// Most indexes the key may hold at once (not expired).
    pub max_active_indexes: Option<usize>,
    /// Most metered queries the key may make per UTC calendar month.
    /// Index creation is not a query and is never counted here.
    pub max_queries_per_month: Option<u64>,
    /// Most files one index may be built from.
    pub max_files_per_index: Option<usize>,
    /// Per-request wall-clock limit for query endpoints.
    pub query_timeout: Option<Duration>,
    /// Most index operations the key may run concurrently.
    pub max_concurrent_index_ops: Option<usize>,
    /// The ingest caps (upload bytes, tree bytes, archive entries,
    /// clone timeout) for this plan.
    pub ingest: IngestCaps,
}

const HOSTED_LIMITS: PlanLimits = PlanLimits {
    max_active_indexes: Some(HOSTED_MAX_ACTIVE_INDEXES),
    max_queries_per_month: Some(HOSTED_MAX_QUERIES_PER_MONTH),
    max_files_per_index: Some(HOSTED_MAX_FILES_PER_INDEX),
    query_timeout: Some(Duration::from_secs(HOSTED_QUERY_TIMEOUT_SECS)),
    max_concurrent_index_ops: Some(HOSTED_MAX_CONCURRENT_INDEX_OPS),
    ingest: IngestCaps::HOSTED,
};

const SELF_HOSTED_LIMITS: PlanLimits = PlanLimits {
    max_active_indexes: None,
    max_queries_per_month: None,
    max_files_per_index: None,
    query_timeout: None,
    max_concurrent_index_ops: None,
    ingest: IngestCaps::UNLIMITED,
};

/// The limits a plan name resolves to. Only `self-hosted` is
/// unlimited; `hosted` — and any name the table does not know — gets
/// the tenant envelope (see the module docs for why unknown fails
/// toward hosted).
pub fn plan_limits(plan: &str) -> PlanLimits {
    match plan {
        SELF_HOSTED_PLAN => SELF_HOSTED_LIMITS,
        _ => HOSTED_LIMITS,
    }
}

/// The process-wide envelope state: the global query semaphore and
/// one index-operation semaphore per key.
///
/// The per-key semaphores are created on first use with the key's
/// plan cap and kept for the process lifetime. The map grows by one
/// entry per key that ever indexes — bounded by the number of keys
/// the operator has issued, which is the same order as the key store
/// itself.
#[derive(Debug)]
pub struct Envelope {
    query_permits: Arc<Semaphore>,
    index_permits: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl Envelope {
    /// An envelope with the production global query cap.
    pub fn new() -> Self {
        Envelope::with_query_permits(GLOBAL_MAX_CONCURRENT_QUERIES)
    }

    /// An envelope with a chosen global query cap — the seam tests
    /// use to exercise exhaustion without four real graph loads.
    pub fn with_query_permits(permits: usize) -> Self {
        Envelope {
            query_permits: Arc::new(Semaphore::new(permits)),
            index_permits: Mutex::new(HashMap::new()),
        }
    }

    /// Waits for a global query slot. The semaphore is never closed,
    /// so acquisition cannot fail.
    pub async fn acquire_query(&self) -> OwnedSemaphorePermit {
        self.query_permits
            .clone()
            .acquire_owned()
            .await
            .expect("the query semaphore is never closed")
    }

    /// Takes a global query slot without waiting, if one is free.
    pub fn try_acquire_query(&self) -> Option<OwnedSemaphorePermit> {
        self.query_permits.clone().try_acquire_owned().ok()
    }

    /// Takes this key's index-operation slot without waiting.
    ///
    /// `None` means the key already runs `max_concurrent` index
    /// operations (or the permit map's lock was poisoned, in which
    /// case refusing is again the safe direction). Index operations
    /// refuse rather than queue: a queued creation would hold a
    /// connection open for the length of someone else's clone, and a
    /// client can retry a `429` the moment its own first build ends.
    pub fn try_acquire_index_op(
        &self,
        key_id: &str,
        max_concurrent: usize,
    ) -> Option<OwnedSemaphorePermit> {
        let semaphore = {
            let mut map = self.index_permits.lock().ok()?;
            map.entry(key_id.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(max_concurrent)))
                .clone()
        };
        semaphore.try_acquire_owned().ok()
    }
}

impl Default for Envelope {
    fn default() -> Self {
        Envelope::new()
    }
}

/// Middleware over the query routes: the global concurrency cap and
/// the caller's per-plan query timeout.
///
/// Runs inside the auth layer (which has already attached
/// [`KeyPlan`]), so the timeout is the authenticated key's own plan
/// value; a request with no plan attached — the router served
/// without the auth wrapper, as in some tests — gets the cap but no
/// timeout. The permit is held for the whole request. On timeout the
/// answer is `504 request_timeout`: the handler future is dropped,
/// though blocking work it already spawned runs to completion in
/// the background (a `spawn_blocking` task cannot be cancelled), so
/// a timed-out request's tail may briefly outlive its permit — the
/// cap bounds admitted requests, not physics.
pub async fn query_envelope(
    State(envelope): State<Arc<Envelope>>,
    request: Request,
    next: Next,
) -> Response {
    let plan = request
        .extensions()
        .get::<KeyPlan>()
        .map(|plan| plan.as_str().to_string());
    let timeout = plan
        .as_deref()
        .map(|name| plan_limits(name).query_timeout)
        .unwrap_or(None);
    let _permit = envelope.acquire_query().await;
    match within(timeout, next.run(request)).await {
        Some(response) => response,
        None => ApiError::request_timeout(
            plan.as_deref().unwrap_or(""),
            timeout.map(|limit| limit.as_secs()).unwrap_or(0),
        )
        .into_response(),
    }
}

/// `Some` of the future's output if it finishes within `limit`;
/// `None` (no limit) always waits. Factored out of the middleware so
/// the boundary is testable without a 30-second test.
async fn within<T>(limit: Option<Duration>, future: impl Future<Output = T>) -> Option<T> {
    match limit {
        None => Some(future.await),
        Some(limit) => tokio::time::timeout(limit, future).await.ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::DEFAULT_PLAN;

    #[test]
    fn hosted_plan_carries_the_offer_numbers() {
        let limits = plan_limits(DEFAULT_PLAN);
        assert_eq!(limits.max_active_indexes, Some(25));
        assert_eq!(limits.max_queries_per_month, Some(10_000));
        assert_eq!(limits.max_files_per_index, Some(50_000));
        assert_eq!(
            limits.query_timeout,
            Some(Duration::from_secs(30)),
            "a query timeout exists and is the documented value"
        );
        assert_eq!(limits.max_concurrent_index_ops, Some(1));
        assert_eq!(limits.ingest, IngestCaps::HOSTED);
    }

    #[test]
    fn self_hosted_plan_is_unlimited() {
        let limits = plan_limits(SELF_HOSTED_PLAN);
        assert_eq!(limits.max_active_indexes, None);
        assert_eq!(limits.max_queries_per_month, None);
        assert_eq!(limits.max_files_per_index, None);
        assert_eq!(limits.query_timeout, None);
        assert_eq!(limits.max_concurrent_index_ops, None);
        assert_eq!(limits.ingest, IngestCaps::UNLIMITED);
    }

    #[test]
    fn an_unknown_plan_gets_the_tenant_envelope_not_the_operators() {
        // An admin can provision any plan string; a typo must not
        // mint an unlimited key.
        assert_eq!(plan_limits("hosed"), plan_limits(DEFAULT_PLAN));
        assert_eq!(plan_limits(""), plan_limits(DEFAULT_PLAN));
    }

    #[test]
    fn global_query_permits_run_out_at_the_cap() {
        let envelope = Envelope::with_query_permits(2);
        let first = envelope.try_acquire_query().expect("first slot");
        let second = envelope.try_acquire_query().expect("second slot");
        assert!(envelope.try_acquire_query().is_none(), "cap reached");
        drop(first);
        assert!(
            envelope.try_acquire_query().is_some(),
            "a freed slot can be taken again"
        );
        drop(second);
    }

    #[test]
    fn index_op_permits_are_per_key_and_refuse_when_taken() {
        let envelope = Envelope::new();
        let held = envelope
            .try_acquire_index_op("tenant-a", 1)
            .expect("first index op takes the slot");
        assert!(
            envelope.try_acquire_index_op("tenant-a", 1).is_none(),
            "the same key's second op is refused at a cap of one"
        );
        assert!(
            envelope.try_acquire_index_op("tenant-b", 1).is_some(),
            "another key has its own slot"
        );
        drop(held);
        assert!(
            envelope.try_acquire_index_op("tenant-a", 1).is_some(),
            "the slot returns when the op ends"
        );
    }

    #[tokio::test]
    async fn within_reports_a_miss_past_the_limit() {
        let slow = within(
            Some(Duration::from_millis(10)),
            tokio::time::sleep(Duration::from_secs(60)),
        )
        .await;
        assert!(slow.is_none(), "a future past its limit is a miss");
        let fast = within(Some(Duration::from_secs(60)), async { 7 }).await;
        assert_eq!(fast, Some(7));
        let unbounded = within(None, async { 9 }).await;
        assert_eq!(unbounded, Some(9), "no limit always waits");
    }
}
