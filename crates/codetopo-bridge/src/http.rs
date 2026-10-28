// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! The bridge's HTTP surface — three routes and a health check.
//!
//! - `POST /billing/webhook` — Stripe's deliveries. The signature is
//!   verified over the raw body before any JSON is trusted; an invalid
//!   signature is a `400` and changes nothing (a `500` is reserved for
//!   a verified event the bridge failed to apply, so Stripe retries
//!   it — the handlers converge under retries).
//! - `GET /claim` — the claim page the checkout success redirect lands
//!   on. The Payment Link redirect carries the checkout session id in
//!   the `session` parameter (Stripe's `{CHECKOUT_SESSION_ID}`
//!   placeholder; `token` is accepted as an alias), and the page
//!   renders with that value prefilled into the form — escaped, and
//!   never auto-submitted: claiming stays one deliberate button press.
//! - `POST /claim` — the one-time exchange of a claim token for the
//!   credential (see [`crate::service::claim_key`]). Every refusal is
//!   the same page at the same status.
//! - `GET /healthz` — liveness for the deployment's supervisor. It
//!   reports nothing but that the process is up.
//!
//! This listener is the bridge's own; `codetopo-server` gains no route
//! from any of this.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use codetopo_server::keys::KeyStore;
use serde::Deserialize;
use serde_json::json;

use crate::mail::KeyMailer;
use crate::stripe::{verify_signature, WebhookEvent};
use crate::service::{apply_event, claim_key, Policy};
use crate::store::BridgeStore;

/// Webhook bodies are events, not uploads: 1 MiB is generous.
const MAX_WEBHOOK_BODY: usize = 1 << 20;

/// Shared state for the bridge routes.
pub struct BridgeState {
    /// The bridge's tables in the shared key store file.
    pub store: BridgeStore,
    /// The runtime key store — the same library the admin CLI uses.
    pub keys: KeyStore,
    /// Price/plan/claim-key policy.
    pub policy: Policy,
    /// The Stripe webhook secret (never logged).
    pub webhook_secret: String,
    /// The key-delivery email transport, when SMTP is configured.
    /// `None` is the claim-page-only bridge, exactly as before.
    pub mailer: Option<Arc<dyn KeyMailer>>,
}

/// Builds the bridge router.
pub fn router(state: Arc<BridgeState>) -> Router {
    Router::new()
        .route("/billing/webhook", post(webhook))
        .route_layer(DefaultBodyLimit::max(MAX_WEBHOOK_BODY))
        .route("/claim", get(claim_page))
        .route("/claim", post(claim_submit))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}

/// `POST /billing/webhook`: verify, parse, apply.
async fn webhook(
    State(state): State<Arc<BridgeState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let signature = headers
        .get("stripe-signature")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !verify_signature(&state.webhook_secret, signature, &body, now_unix_secs()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid_signature" })),
        );
    }
    let event: WebhookEvent = match serde_json::from_slice(&body) {
        Ok(event) => event,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid_payload" })),
            );
        }
    };
    match apply_event(
        &state.store,
        &state.keys,
        &state.policy,
        state.mailer.as_deref(),
        &event,
        now_unix_secs(),
    ) {
        Ok(outcome) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "outcome": format!("{outcome:?}").to_lowercase() })),
        ),
        Err(err) => {
            // Server-side only, and BridgeError never carries secrets,
            // tokens, or signature material (see its docs).
            eprintln!(
                "codetopo-bridge: event {} ({}) failed: {err}",
                event.event_id, event.event_type
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal" })),
            )
        }
    }
}

/// The claim form's one field.
#[derive(Debug, Deserialize)]
pub struct ClaimForm {
    /// The checkout session id from the checkout, or the subscription
    /// id from the buyer's Stripe receipt.
    pub token: String,
}

/// `POST /claim`: the one-time exchange.
async fn claim_submit(
    State(state): State<Arc<BridgeState>>,
    Form(form): Form<ClaimForm>,
) -> impl IntoResponse {
    match claim_key(&state.store, &state.policy, &form.token, now_unix_secs()) {
        Ok((key_id, secret)) => (
            StatusCode::OK,
            Html(claim_success_page(&key_id, &secret)),
        ),
        Err(_) => (StatusCode::BAD_REQUEST, Html(claim_failure_page())),
    }
}

/// The claim page's query string: the checkout redirect's session id.
#[derive(Debug, Deserialize)]
pub struct ClaimQuery {
    /// Stripe's `{CHECKOUT_SESSION_ID}` placeholder lands here.
    pub session: Option<String>,
    /// Alias for `session`, for redirects that name the field as the
    /// form does.
    pub token: Option<String>,
}

/// `GET /claim`: the page the checkout success redirect lands on.
/// With a session id in the query string the form renders prefilled;
/// without one it renders exactly as the static page.
async fn claim_page(Query(query): Query<ClaimQuery>) -> Html<String> {
    let prefill = query.session.as_deref().or(query.token.as_deref());
    Html(claim_page_html(prefill))
}

/// The claim landing page, with `prefill` (when present) rendered
/// into the token field's `value` attribute, HTML-escaped. The value
/// arrives from a redirect URL — it is data in a page, and pages
/// escape data: a hostile value stays inert text inside the
/// attribute and can neither break out of it nor inject markup.
fn claim_page_html(prefill: Option<&str>) -> String {
    match prefill {
        None => CLAIM_PAGE.to_string(),
        Some(value) => CLAIM_PAGE.replacen(
            "id=\"token\"",
            &format!("id=\"token\" value=\"{}\"", html_escape(value)),
            1,
        ),
    }
}

/// The success page: the credential, once, with handling instructions.
/// The credential is HTML-escaped even though its alphabet is tame —
/// it is data in a page, and pages escape data.
fn claim_success_page(key_id: &str, secret: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>Your codetopo API key</title></head><body>\
         <h1>Your codetopo API key</h1>\
         <p>Copy this credential now and store it somewhere safe. \
         It is shown exactly once — the bridge keeps only a hash, so it \
         cannot be displayed again.</p>\
         <pre><code>{}:{}</code></pre>\
         <p>Send it as <code>Authorization: Bearer &lt;key&gt;</code> \
         with every request to the codetopo API.</p>\
         </body></html>",
        html_escape(key_id),
        html_escape(secret)
    )
}

/// The one refusal page for every failed claim.
fn claim_failure_page() -> String {
    "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
     <title>Claim unavailable</title></head><body>\
     <h1>This claim is no longer available</h1>\
     <p>The code may have been entered incorrectly, already used, or \
     expired (claims live for seven days after purchase). If you \
     completed a purchase and never received your key, contact support \
     with your Stripe receipt.</p>\
     </body></html>"
        .to_string()
}

/// The claim landing page. The inline script prefills the token from
/// the checkout redirect's `session_id` parameter (or `tx`); the buyer
/// can also paste the subscription id from their Stripe receipt.
const CLAIM_PAGE: &str = "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Claim your codetopo API key</title></head><body>\
<h1>Claim your codetopo API key</h1>\
<p>Your purchase is complete. Enter the session id from your \
checkout confirmation — it is in the page address after checkout — or \
the subscription id from your Stripe receipt. Your key is shown \
exactly once.</p>\
<form method=\"post\" action=\"/claim\">\
<label>Session or subscription id \
<input name=\"token\" id=\"token\" size=\"44\" autocomplete=\"off\" required></label>\
<button type=\"submit\">Show my key</button></form>\
<script>\
const params = new URLSearchParams(location.search);\
const prefill = params.get('session_id') || params.get('tx');\
if (prefill) { document.getElementById('token').value = prefill; }\
</script>\
</body></html>";

/// Escapes the five HTML-significant characters.
fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Current time as Unix seconds; 0 if the clock predates the epoch.
fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_neutralises_markup() {
        assert_eq!(
            html_escape("<a href=\"x\">&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;"
        );
        assert_eq!(html_escape("sub_01abc:deadbeef"), "sub_01abc:deadbeef");
    }

    #[test]
    fn the_claim_page_carries_the_form_and_the_prefill() {
        assert!(CLAIM_PAGE.contains("name=\"token\""));
        assert!(CLAIM_PAGE.contains("session_id"));
        assert!(CLAIM_PAGE.contains("method=\"post\""));
    }

    #[test]
    fn no_prefill_renders_the_static_page_unchanged() {
        assert_eq!(claim_page_html(None), CLAIM_PAGE);
    }

    #[test]
    fn prefill_lands_in_the_token_fields_value() {
        let page = claim_page_html(Some("cs_test_abc123"));
        assert!(
            page.contains("name=\"token\" id=\"token\" value=\"cs_test_abc123\""),
            "{page}"
        );
    }

    #[test]
    fn a_hostile_prefill_is_rendered_only_escaped() {
        let payload = "\"><script>alert(1)</script>";
        let page = claim_page_html(Some(payload));
        assert!(
            !page.contains(payload),
            "the raw payload must never appear: {page}"
        );
        assert!(
            page.contains("value=\"&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;\""),
            "the payload may appear only in escaped form: {page}"
        );
    }
}
