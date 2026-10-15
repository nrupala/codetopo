// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! End-to-end bridge behaviour over its real HTTP surface, with fake
//! Stripe payloads signed by a test secret — no network, no Stripe.
//! Proves the wire contract: a badly signed (or stale-signed) delivery
//! is a 400 that changes nothing; a signed purchase mints a key into
//! the shared store; the claim page exchanges the checkout session id
//! for the credential exactly once.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use codetopo_bridge::http::{router, BridgeState};
use codetopo_bridge::service::Policy;
use codetopo_bridge::store::BridgeStore;
use codetopo_server::keys::KeyStore;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tower::ServiceExt;

const SECRET: &str = "whsec_test_secret";
const PRICE: &str = "price_test_hosted";

struct Rig {
    app: axum::Router,
    keys: Arc<KeyStore>,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("keys.db");
    let keys = KeyStore::open(&db_path).expect("key store");
    let store = BridgeStore::open(&db_path).expect("bridge store");
    let state = Arc::new(BridgeState {
        store,
        keys,
        policy: Policy {
            price_id: PRICE.to_string(),
            plan: "hosted".to_string(),
            claim_key: {
                use sha2::Digest;
                Sha256::digest(b"codetopo-bridge http test key").into()
            },
        },
        webhook_secret: SECRET.to_string(),
    });
    let keys_handle = Arc::new(KeyStore::open(&db_path).expect("key store handle"));
    Rig {
        app: router(state),
        keys: keys_handle,
        _dir: dir,
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Stripe's scheme: HMAC-SHA256 over `"<t>.<body>"`, header `t=…,v1=…`.
fn sign_at(secret: &str, body: &str, t: i64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac");
    mac.update(t.to_string().as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("t={t},v1={hex}")
}

fn sign(secret: &str, body: &str) -> String {
    sign_at(secret, body, now_unix())
}

async fn post_webhook(app: &axum::Router, body: &str, signature: &str) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/billing/webhook")
                .header("content-type", "application/json")
                .header("stripe-signature", signature)
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

async fn post_claim(app: &axum::Router, token: &str) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/claim")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!("token={token}")))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn subscription_created(event_id: &str, sub: &str) -> String {
    serde_json::json!({
        "id": event_id,
        "type": "customer.subscription.created",
        "created": 1760000000,
        "data": { "object": {
            "id": sub,
            "status": "active",
            "items": { "object": "list", "data": [{ "price": { "id": PRICE } }] },
        } },
    })
    .to_string()
}

fn checkout_completed(event_id: &str, session: &str, sub: &str) -> String {
    serde_json::json!({
        "id": event_id,
        "type": "checkout.session.completed",
        "created": 1760000060,
        "data": { "object": {
            "id": session,
            "mode": "subscription",
            "payment_status": "paid",
            "subscription": sub,
        } },
    })
    .to_string()
}

#[tokio::test]
async fn a_badly_signed_delivery_is_refused_and_changes_nothing() {
    let rig = rig();
    let body = subscription_created("evt_1", "sub_T1");
    let (status, _) = post_webhook(&rig.app, &body, "t=1,v1=deadbeef").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = post_webhook(&rig.app, &body, &sign("the-wrong-secret", &body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // A correctly signed but stale delivery is refused alike: the
    // signature is genuine and the timestamp is an hour old.
    let stale = sign_at(SECRET, &body, now_unix() - 3600);
    let (status, _) = post_webhook(&rig.app, &body, &stale).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(rig.keys.len(), 0, "no key may be minted by a bad delivery");
}

#[tokio::test]
async fn purchase_to_claimed_key_over_http() {
    let rig = rig();
    let created = subscription_created("evt_1", "sub_T2");
    let (status, body) = post_webhook(&rig.app, &created, &sign(SECRET, &created)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"applied\""), "{body}");

    let checkout = checkout_completed("evt_2", "cs_T2", "sub_T2");
    let (status, body) = post_webhook(&rig.app, &checkout, &sign(SECRET, &checkout)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"applied\""), "{body}");

    // A replay of the same signed delivery is a duplicate, not a second key.
    let (status, body) = post_webhook(&rig.app, &checkout, &sign(SECRET, &checkout)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"duplicate\""), "{body}");
    assert_eq!(rig.keys.len(), 1);

    // The claim page renders and carries the form.
    let response = rig
        .app
        .clone()
        .oneshot(Request::builder().uri("/claim").body(Body::empty()).expect("request"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    // Claim with the session id: the credential appears, once.
    let (status, page) = post_claim(&rig.app, "cs_T2").await;
    assert_eq!(status, StatusCode::OK);
    let marker = "sub_T2:";
    let start = page.find(marker).expect("credential on the success page") + marker.len();
    let secret = &page[start..start + 64];
    assert!(
        rig.keys.verify("sub_T2", secret).is_ok(),
        "the claimed secret must verify against the shared store"
    );
    let (status, _) = post_claim(&rig.app, "cs_T2").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a second claim is refused");
    let (status, _) = post_claim(&rig.app, "cs_never_existed").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "an unknown token is refused alike");
}

#[tokio::test]
async fn healthz_answers() {
    let rig = rig();
    let response = rig
        .app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
}
