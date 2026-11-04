// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Key delivery by email: the second channel (owner directive,
//! 2026-10-18 — "one email is the receipt, and another one is the
//! key").
//!
//! Stripe's receipt tells the buyer they paid; this module's message
//! tells them what they bought. When SMTP submission is configured
//! (see [`crate::config::MailConfig`]), [`crate::service`] composes
//! one message per issued key and sends it to the address the
//! checkout session carried. The message carries the credential —
//! once as the key itself and once inside the worked first-call
//! example, nowhere else — the API base, the plan's limits, a
//! quickstart (the first call with the key filled in, three first
//! steps, and the MCP configuration the repo documents), and the
//! support address — plain text, plain words. A bare key teaches
//! nothing; the quickstart is what makes the purchase usable, and
//! the claim page carries the same material for the same reason.
//!
//! Email here is a *delivery copy*, never an authority. Nothing in
//! the bridge accepts an email address as proof of anything: claims
//! still run on the claim tokens, entitlements still key on the
//! subscription id, and a lost email is a support conversation, not
//! a credential reset. The store records the recipient address and
//! the send time (that record is what a support resend would work
//! from); it never records the credential — the only stored copy of
//! the secret remains the sealed delivery, with its claim-or-purge
//! lifetime unchanged.
//!
//! A failed send is a logged disappointment, never a failed
//! purchase: the webhook has already succeeded by the time a send is
//! attempted, and the claim page still holds the key.
//!
//! ## Lifecycle emails
//!
//! The key email opens the relationship; three transactional
//! messages maintain it, composed here and offered by
//! [`crate::service`] when the events they describe apply: a
//! cancellation that is scheduled (the key works until the paid
//! period ends, and the email says the date), a subscription that
//! has ended (the key no longer works), and a failed payment (the
//! key keeps working through the grace while Stripe retries, and
//! the email says how to fix the payment method). Each states what
//! happened, what it means for the buyer's key, and the one next
//! action — plain words, facts only, no marketing. They carry no
//! credential, so nothing in them needs the delivery machinery the
//! key email uses; they ride the same [`KeyMailer`] seam and the
//! same failure rules (a failed send is logged, never a failed
//! webhook).
//!
//! A fourth message completes the set: the refund. When a charge
//! against a codetopo subscription is refunded, the bridge resolves
//! the charge to the entitlement (charge → invoice → the
//! subscription recorded for that invoice → entitlement) and emails
//! the buyer the amount refunded, the plan it was for, and what the
//! refund does to the key — which, under the lifecycle rules, is
//! nothing by itself: a refund is not a cancellation, so the email
//! states the key's position under the entitlement's own status
//! (unchanged, working until its already-scheduled end, or already
//! stopped). Stripe's own refund email (an account-level setting,
//! switched on 2026-11-03) still fires as well; this message is the
//! one that explains what the refund means for the key.

use std::time::Duration;

use codetopo_server::limits::PlanLimits;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::message::{Mailbox, Message};
use lettre::{SmtpTransport, Transport};

use crate::config::{MailConfig, MailTls};

/// Where the hosted API lives; printed in the key email.
pub const API_BASE_URL: &str = "https://codetopo-api.aimlds.org";
/// Where a buyer writes when the key is lost; printed in the email.
pub const SUPPORT_EMAIL: &str = "info@aimlds.org";
/// The MCP client configuration exactly as the README and the
/// install guide document it. codetopo's MCP server is the free
/// local companion — stdio, on the buyer's own machine, and it
/// takes no key — so the quickstart prints the block as-is and says
/// so; the buyer's key belongs to the hosted API, not to this block.
pub const MCP_CONFIG_SNIPPET: &str =
    "{ \"mcpServers\": { \"codetopo\": { \"command\": \"codetopo-mcp\" } } }";

/// The direct manage/cancel link: Stripe's Customer Portal for
/// this product, activated on the Stripe side on 2026-11-03 (cancel
/// at period end, invoice history). Both buyer surfaces print it
/// via [`manage_subscription_line`]. The `None` arm of the line
/// remains as the honest fallback — it points the buyer at the
/// manage link in their Stripe receipt email — should the portal
/// ever be deactivated and this constant cleared.
pub const MANAGE_SUBSCRIPTION_URL: Option<&str> =
    Some("https://billing.stripe.com/p/login/5kQ6oH9SydMQ6yPav49Zm00");

/// The manage-or-cancel line both buyer surfaces (the key email and
/// the claim page) carry, so cancelling never requires hunting
/// through a dashboard. The line prints the Customer Portal link
/// (see [`MANAGE_SUBSCRIPTION_URL`]); were that constant ever
/// cleared, the line would name the receipt-email route instead.
/// The end-of-period behaviour is the README's published
/// term: the key stays active until the end of the paid period and
/// is then revoked.
pub fn manage_subscription_line() -> String {
    match MANAGE_SUBSCRIPTION_URL {
        Some(url) => format!(
            "Manage or cancel your subscription anytime: {url} — cancelling \
             stops the next charge, and your key stays active until the end \
             of the paid period."
        ),
        None => "Manage or cancel your subscription anytime from the link \
                 in your Stripe receipt email — cancelling stops the next \
                 charge, and your key stays active until the end of the \
                 paid period."
            .to_string(),
    }
}

/// The indexing call — step two of the quickstart (step one is the
/// `GET /v1/schema` key check) — with the buyer's credential filled
/// in, ready to copy and run. The claim page prints the same
/// example: a buyer leaves either surface able to make the call,
/// not holding a bare key. `repo_url` is the hosted ingest mode:
/// a hosted key is refused `repo_path`, a path on the server host,
/// so buyer examples never show that shape. The answer carries the
/// `index_id` every later call names.
pub fn first_call_example(key_id: &str, secret: &str) -> String {
    format!(
        "curl -H \"Authorization: Bearer {key_id}:{secret}\" \\\n\
         \x20 -X POST {API_BASE_URL}/v1/index \\\n\
         \x20 -H 'Content-Type: application/json' \\\n\
         \x20 -d '{{\"repo_url\": \"https://github.com/your-name/your-repo\"}}'"
    )
}
/// How long one SMTP conversation may take before the send is a
/// failure. Webhook handling waits for the send (the event is
/// already recorded by then, so a redelivery is a duplicate and
/// cannot re-send), and twenty seconds is well inside the time
/// Stripe allows a receiver.
const SMTP_TIMEOUT: Duration = Duration::from_secs(20);

/// One composed key-delivery message. The from-address is not part
/// of the message: it is a property of the transport configuration
/// (the address the SMTP credential may send as), applied by the
/// mailer at send time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEmail {
    /// The buyer's address, from the checkout session.
    pub to: String,
    /// The subject line.
    pub subject: String,
    /// The plain-text body. Carries the credential twice: as the
    /// key, and inside the worked first-call example.
    pub text: String,
}

/// One composed lifecycle message: a cancellation scheduled, a
/// subscription ended, a payment failed. No credential is ever part
/// of one — these say what happened to the subscription, never what
/// the key is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleEmail {
    /// The buyer's address, recorded at checkout.
    pub to: String,
    /// The subject line.
    pub subject: String,
    /// The plain-text body.
    pub text: String,
}

/// A send failure. The message is safe to log on the bridge host: it
/// carries transport detail (server responses, OS errors) and never
/// a credential — neither the API key nor the SMTP password is part
/// of any error this module produces.
#[derive(Debug, thiserror::Error)]
#[error("key email: {0}")]
pub struct MailError(pub String);

/// The sending seam. Production is [`SmtpMailer`]; tests substitute
/// a recording double, so the state machine's send behaviour is
/// proven without a network.
pub trait KeyMailer: Send + Sync {
    /// Sends one composed key email. Called at most once per issued
    /// key (the service tracks the send on the delivery row).
    fn send_key_email(&self, email: &KeyEmail) -> Result<(), MailError>;
    /// Sends one composed lifecycle email. Called at most once per
    /// applied event that carries the news (the service derives the
    /// news from the event and the entitlement it produced).
    fn send_lifecycle_email(&self, email: &LifecycleEmail) -> Result<(), MailError>;
}

/// Composes the key email for one issued key. The limits are read
/// from the plan's own table, so the email can never promise a
/// different envelope than the server enforces. The quickstart
/// section is composed from the same constants the claim page
/// prints, so the two surfaces teach the same first steps.
pub fn compose_key_email(to: &str, key_id: &str, secret: &str, limits: &PlanLimits) -> KeyEmail {
    let plan_line = match (limits.max_active_indexes, limits.max_queries_per_month) {
        (Some(indexes), Some(queries)) => format!(
            "Your hosted plan includes up to {} active indexes and {} queries per month.",
            indexes,
            thousands(queries)
        ),
        _ => "Your plan carries no fixed cap on active indexes or monthly queries.".to_string(),
    };
    let first_call = first_call_example(key_id, secret);
    // The manage line prints the live Customer Portal link (see
    // MANAGE_SUBSCRIPTION_URL); it would name the Stripe receipt
    // route only if that constant were ever cleared.
    let manage_line = manage_subscription_line();
    let text = format!(
        "Your codetopo Hosted subscription is active, and your API key is ready.\n\
         \n\
         Your key:\n\
         \n\
         {key_id}:{secret}\n\
         \n\
         Store it somewhere safe. The key is shown in full only in this email\n\
         and once on the claim page from your checkout; after that, only a\n\
         hash of it exists, and it cannot be displayed again.\n\
         \n\
         Send the key with every request to the codetopo API:\n\
         \n\
         \x20   Authorization: Bearer <key>\n\
         \n\
         API base: {API_BASE_URL}\n\
         \n\
         {plan_line}\n\
         \n\
         Using your key\n\
         \n\
         Three first steps:\n\
         \n\
         1. Check your key works —\n\
         GET {API_BASE_URL}/v1/schema\n\
         with your Authorization header (the Bearer line above). With\n\
         your key it answers with the API's schema; without one, the\n\
         same call is refused.\n\
         \n\
         2. Index a repository — this call, with your key filled in:\n\
         \n\
         {first_call}\n\
         \n\
         The reply carries an index_id, with files_indexed, nodes and\n\
         edges counted. Every later call names that index_id. (A public\n\
         git URL is the way in — the hosted API cannot index a path on\n\
         your own machine.)\n\
         \n\
         3. Ask the graph questions, with the same Authorization header:\n\
         GET {API_BASE_URL}/v1/graphs/<index_id>/search?q=<term>\n\
         finds symbols by name, and\n\
         GET {API_BASE_URL}/v1/graphs/<index_id>/stats\n\
         sums the index up. From a symbol id,\n\
         GET {API_BASE_URL}/v1/graphs/<index_id>/symbol?node=<symbol-id>\n\
         reads that symbol's dossier — kind, location, and what it\n\
         depends on; from there you can also ask what calls it, what it\n\
         calls, and what breaks if it changes.\n\
         \n\
         Using an MCP client? codetopo's MCP server is the free local\n\
         companion — the same engine on your own machine, and it takes\n\
         no key (your key is for the hosted API above). Install it with\n\
         `cargo install codetopo-mcp`, then add this to your client's\n\
         MCP settings:\n\
         \n\
         {MCP_CONFIG_SNIPPET}\n\
         \n\
         {manage_line}\n\
         \n\
         If you lose the key or it stops working, contact support at\n\
         {SUPPORT_EMAIL} with your Stripe receipt.\n"
    );
    KeyEmail {
        to: to.to_string(),
        subject: "Your codetopo API key".to_string(),
        text,
    }
}

/// The undo line for the cancellation-scheduled email, on the same
/// seam as [`manage_subscription_line`]: the Customer Portal URL
/// when one is configured, the Stripe receipt route until then.
fn undo_cancellation_line() -> String {
    match MANAGE_SUBSCRIPTION_URL {
        Some(url) => format!(
            "If you cancelled by mistake, you can undo the cancellation here: {url}"
        ),
        None => "If you cancelled by mistake, you can undo the cancellation \
                 from the link in your Stripe receipt email."
            .to_string(),
    }
}

/// The fix-your-payment line for the payment-failed email, on the
/// same seam again: the Customer Portal URL (configured — see
/// [`MANAGE_SUBSCRIPTION_URL`]); the invoice-email route remains
/// only as the fallback should the constant ever be cleared.
fn fix_payment_line() -> String {
    match MANAGE_SUBSCRIPTION_URL {
        Some(url) => format!("Update your payment method here: {url}"),
        None => "Update your payment method from the link in the invoice \
                 email Stripe sent for this payment."
            .to_string(),
    }
}

/// Composes the cancellation-scheduled email: the buyer cancelled,
/// no further charges will be made, and the key keeps working until
/// `ends_at` — the end of the period already paid for, which is
/// when the entitlement's own revocation falls due.
pub fn compose_cancellation_scheduled_email(to: &str, ends_at: i64) -> LifecycleEmail {
    let ends = date_line(ends_at);
    let undo_line = undo_cancellation_line();
    let text = format!(
        "Your codetopo subscription is cancelled.\n\
         \n\
         No further charges will be made.\n\
         \n\
         Your API key keeps working until {ends} — the end of the period\n\
         you have already paid for. When that day comes, the key stops\n\
         working.\n\
         \n\
         {undo_line}\n\
         \n\
         Questions: contact support at {SUPPORT_EMAIL}.\n"
    );
    LifecycleEmail {
        to: to.to_string(),
        subject: "Your codetopo subscription is cancelled".to_string(),
        text,
    }
}

/// Composes the subscription-ended email: the subscription is over
/// (an immediate cancellation, or the deleted event that closes a
/// scheduled one), and the key no longer works.
pub fn compose_subscription_ended_email(to: &str) -> LifecycleEmail {
    let text = format!(
        "Your codetopo subscription has ended.\n\
         \n\
         Your API key no longer works: requests sent with it are refused.\n\
         No further charges will be made.\n\
         \n\
         If this was a mistake, or you want to use codetopo again, contact\n\
         support at {SUPPORT_EMAIL}.\n"
    );
    LifecycleEmail {
        to: to.to_string(),
        subject: "Your codetopo subscription has ended".to_string(),
        text,
    }
}

/// Composes the payment-failed email: the latest payment did not go
/// through, Stripe will retry it, and the key keeps working through
/// the bridge's past-due grace, which ends at `grace_ends_at`. If
/// the payment has not succeeded by then, the key stops working.
pub fn compose_payment_failed_email(to: &str, grace_ends_at: i64) -> LifecycleEmail {
    let grace_ends = date_line(grace_ends_at);
    let fix_line = fix_payment_line();
    let text = format!(
        "Your latest codetopo subscription payment did not go through.\n\
         \n\
         Stripe will try the payment again. Your API key keeps working\n\
         while the payment is retried, and until {grace_ends}. If the\n\
         payment has not succeeded by then, the key stops working.\n\
         \n\
         {fix_line}\n\
         \n\
         Questions: contact support at {SUPPORT_EMAIL}.\n"
    );
    LifecycleEmail {
        to: to.to_string(),
        subject: "Your codetopo payment failed".to_string(),
        text,
    }
}

/// What a refund does to the buyer's key, read from the
/// entitlement's status at the moment the refund lands. A refund
/// moves no bridge state — it is not a cancellation — so every arm
/// is a statement about state other events created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefundKeyFate {
    /// The subscription stands as it was: the key keeps working and
    /// the subscription renews as before unless the buyer cancels.
    Unaffected,
    /// A revocation was already scheduled (a cancellation's period
    /// end, or the past-due grace end): the key works until then.
    WorksUntil(i64),
    /// The subscription had already ended: the key no longer works,
    /// and the refund changes nothing about that.
    AlreadyStopped,
}

/// A minor-unit amount as a buyer reads it — `49.00 USD`. The
/// bridge's currencies are two-decimal (USD, CAD, EUR); the code is
/// printed rather than a symbol so any currency states itself
/// truthfully.
pub fn format_money(amount_minor: i64, currency: &str) -> String {
    let amount = amount_minor.max(0);
    format!("{}.{:02} {}", amount / 100, amount % 100, currency.to_uppercase())
}

/// The public name of a plan stamp, for buyer-facing copy. The
/// bridge sells two tiers; an unrecognised stamp (an operator's
/// custom plan name) is printed as-is rather than dressed as a
/// tier it is not.
pub fn plan_display_name(plan: &str) -> String {
    match plan {
        // `service::GUARDRAILS_PLAN`'s stamp, named here because
        // this module owns buyer copy.
        "guardrails" => "codetopo Guardrails (US$49/month)".to_string(),
        "hosted" => "codetopo Team (US$19/month)".to_string(),
        other => format!("codetopo plan {other}"),
    }
}

/// Composes the refund email: a charge against the buyer's
/// subscription was refunded. States the amount this refund
/// returned (and, for a partial refund, the charge it was part of
/// and that the rest stands), the plan the charge was for, and
/// what the refund does to the key — per [`RefundKeyFate`], always
/// a statement about the subscription's own state, because the
/// refund itself changes none of it.
pub fn compose_refund_email(
    to: &str,
    plan: &str,
    refunded_minor: i64,
    charge_minor: i64,
    currency: &str,
    partial: bool,
    fate: RefundKeyFate,
) -> LifecycleEmail {
    let plan_name = plan_display_name(plan);
    let amount_line = if partial {
        format!(
            "Refunded: {} of the {} charge — a partial refund, for your\n\
             {plan_name} plan. The rest of the charge stands.",
            format_money(refunded_minor, currency),
            format_money(charge_minor, currency),
        )
    } else {
        format!(
            "Refunded: {} — the full amount of the charge — for your\n\
             {plan_name} plan.",
            format_money(refunded_minor, currency),
        )
    };
    let fate_line = match fate {
        RefundKeyFate::Unaffected => {
            let manage_line = manage_subscription_line();
            format!(
                "This refund does not change your API key. A refund is not\n\
                 a cancellation: your key keeps working, and your subscription\n\
                 is unchanged — it renews as before unless you cancel it.\n\
                 \n\
                 {manage_line}"
            )
        }
        RefundKeyFate::WorksUntil(ends_at) => {
            let ends = date_line(ends_at);
            format!(
                "This refund does not change your API key's dates. Your key\n\
                 keeps working until {ends} — the end already in effect for\n\
                 your subscription — and then stops."
            )
        }
        RefundKeyFate::AlreadyStopped => {
            "Your API key has already stopped working — your subscription\n\
             had ended before this refund — and the refund does not\n\
             change that."
                .to_string()
        }
    };
    let text = format!(
        "A refund has been issued for your codetopo subscription.\n\
         \n\
         {amount_line}\n\
         \n\
         The money returns to the card you paid with; how long it takes\n\
         to appear is your bank's timing, not ours.\n\
         \n\
         {fate_line}\n\
         \n\
         Questions: contact support at {SUPPORT_EMAIL}.\n"
    );
    LifecycleEmail {
        to: to.to_string(),
        subject: "Your codetopo refund".to_string(),
        text,
    }
}

/// A Unix timestamp as a plain date — `15 January 2027` — in UTC.
/// The bridge speaks Unix seconds everywhere; a buyer reads a date.
/// The civil-from-days conversion is Howard Hinnant's algorithm,
/// exact for any date the timestamps here can name.
pub fn date_line(unix: i64) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let days = unix.div_euclid(86_400);
    // Shift the epoch to 0000-03-01, so leap days end each era-year.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // day of era, [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // year of era, [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year (Mar 1 = 0), [0, 365]
    let mp = (5 * doy + 2) / 153; // month index (Mar = 0), [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if month <= 2 { y + 1 } else { y };
    format!("{day} {} {year}", MONTHS[(month - 1) as usize])
}

/// `10000` as `10,000`, for the plan line.
fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// SMTP submission, configured only from the environment (see
/// [`crate::config::MailConfig`]). One connection per send:
/// purchases are rare, and a fresh connection cannot be a stale one.
pub struct SmtpMailer {
    config: MailConfig,
    from: Mailbox,
}

impl SmtpMailer {
    /// Builds a mailer from resolved configuration. The configured
    /// from-address is parsed now, so a malformed one is a startup
    /// finding (the caller logs it and runs without email) instead of
    /// a surprise at the first purchase.
    pub fn new(config: MailConfig) -> Result<Self, MailError> {
        let from: Mailbox = config
            .from
            .parse()
            .map_err(|err| MailError(format!("CODETOPO_SMTP_FROM is unusable: {err}")))?;
        Ok(SmtpMailer { config, from })
    }
}

impl SmtpMailer {
    /// One plain-text send, shared by every message this module
    /// composes: the envelope differs per message, the transport
    /// never does.
    fn send_message(&self, to: &str, subject: &str, text: &str) -> Result<(), MailError> {
        let to: Mailbox = to
            .parse()
            .map_err(|err| MailError(format!("buyer address is unusable: {err}")))?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(text.to_string())
            .map_err(|err| MailError(format!("cannot compose message: {err}")))?;

        let config = &self.config;
        let mut builder = match config.tls {
            MailTls::StartTls => SmtpTransport::starttls_relay(&config.host)
                .map_err(|err| MailError(format!("cannot build transport: {err}")))?,
            MailTls::ImplicitTls => SmtpTransport::relay(&config.host)
                .map_err(|err| MailError(format!("cannot build transport: {err}")))?,
            MailTls::None => SmtpTransport::builder_dangerous(&config.host),
        };
        builder = builder.port(config.port).timeout(Some(SMTP_TIMEOUT));
        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            builder = builder.credentials(Credentials::new(username.clone(), password.clone()));
        }
        builder
            .build()
            .send(&message)
            .map_err(|err| MailError(format!("SMTP send failed: {err}")))?;
        Ok(())
    }
}

impl KeyMailer for SmtpMailer {
    fn send_key_email(&self, email: &KeyEmail) -> Result<(), MailError> {
        self.send_message(&email.to, &email.subject, &email.text)
    }

    fn send_lifecycle_email(&self, email: &LifecycleEmail) -> Result<(), MailError> {
        self.send_message(&email.to, &email.subject, &email.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codetopo_server::limits::plan_limits;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn composed() -> KeyEmail {
        compose_key_email(
            "buyer@example.com",
            "sub_Example",
            "deadbeefcafe",
            &plan_limits("hosted"),
        )
    }

    #[test]
    fn the_credential_appears_only_as_the_key_and_in_the_first_call() {
        let email = composed();
        assert_eq!(email.to, "buyer@example.com");
        assert_eq!(
            email.text.matches("sub_Example:deadbeefcafe").count(),
            2,
            "the full credential appears as the key and inside the worked \
             first-call example — and nowhere else"
        );
        // The bearer line names the shape, not the secret.
        assert!(email.text.contains("Authorization: Bearer <key>"));
    }

    #[test]
    fn the_message_teaches_the_buyer_to_use_the_key() {
        // Failure messages never interpolate the body: it carries a
        // credential, and assertion output is a log surface.
        let email = composed();
        assert!(
            email.text.contains(&format!("{API_BASE_URL}/v1/index")),
            "the first call targets the hosted index route"
        );
        assert!(
            email.text.contains("\"repo_url\""),
            "the example indexes by public git URL, the hosted mode"
        );
        assert!(
            email.text.contains("/v1/schema"),
            "the key-check step names the schema route"
        );
        assert!(
            email.text.contains("search?q="),
            "the search step names its route"
        );
        assert!(
            email.text.contains("/stats"),
            "the stats step names its route"
        );
        assert!(
            email.text.contains("symbol?node="),
            "the dossier route is named as the way onward"
        );
        assert!(
            email.text.contains(MCP_CONFIG_SNIPPET),
            "the documented MCP configuration is printed verbatim"
        );
        assert!(
            email.text.contains("cargo install codetopo-mcp"),
            "the MCP section says how the local server is installed"
        );
        assert!(
            email.text.contains("Manage or cancel your subscription"),
            "the manage-or-cancel line is present"
        );
    }

    #[test]
    fn the_message_states_the_api_the_limits_and_support() {
        // Failure messages never interpolate the body: it carries a
        // credential, and assertion output is a log surface.
        let email = composed();
        assert!(email.text.contains(API_BASE_URL), "the API base is stated");
        assert!(email.text.contains("25 active indexes"), "the index limit is stated");
        assert!(
            email.text.contains("10,000 queries per month"),
            "the query limit is stated"
        );
        assert!(email.text.contains(SUPPORT_EMAIL), "the support address is stated");
        assert!(email.subject.contains("codetopo"), "the subject names the product");
    }

    #[test]
    fn thousands_groups_by_three() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(10_000), "10,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn date_line_names_real_dates() {
        assert_eq!(date_line(0), "1 January 1970");
        // The leap day of a leap year, and the day after it.
        assert_eq!(date_line(1_709_164_800), "29 February 2024");
        assert_eq!(date_line(1_709_251_200), "1 March 2024");
        // A timestamp mid-day names that day (UTC), not the next.
        assert_eq!(date_line(1_800_000_000), "15 January 2027");
    }

    #[test]
    fn the_cancellation_email_states_the_period_end_and_the_key_keeps_working() {
        // NOW = 1_800_000_000 (15 January 2027); the paid period
        // ends twenty days later.
        let ends = 1_800_000_000 + 20 * 86_400;
        let email = compose_cancellation_scheduled_email("buyer@example.com", ends);
        assert_eq!(email.to, "buyer@example.com");
        assert!(email.subject.contains("cancelled"), "the subject carries the news");
        assert!(
            email.text.contains("No further charges"),
            "the money fact is stated"
        );
        assert!(
            email.text.contains("keeps working until 4 February 2027"),
            "the key's fate is stated with the period-end date"
        );
        assert!(
            email.text.contains("undo the cancellation"),
            "the one next action for a mistaken cancel is named"
        );
        assert!(email.text.contains(SUPPORT_EMAIL), "the support route is stated");
    }

    #[test]
    fn the_ended_email_states_the_key_no_longer_works() {
        let email = compose_subscription_ended_email("buyer@example.com");
        assert!(email.subject.contains("ended"), "the subject carries the news");
        assert!(
            email.text.contains("no longer works"),
            "the key's fate is stated plainly"
        );
        assert!(
            email.text.contains("No further charges"),
            "the money fact is stated"
        );
        assert!(email.text.contains(SUPPORT_EMAIL), "the next action is support");
    }

    #[test]
    fn the_payment_failed_email_states_the_grace_and_the_fix_route() {
        let grace_ends = 1_800_000_000 + 7 * 86_400;
        let email = compose_payment_failed_email("buyer@example.com", grace_ends);
        assert!(email.subject.contains("payment failed"), "the subject carries the news");
        assert!(
            email.text.contains("keeps working"),
            "the key's fate during the grace is stated"
        );
        assert!(
            email.text.contains("until 22 January 2027"),
            "the grace end is stated as a date"
        );
        assert!(
            email.text.contains("stops working"),
            "what happens if the payment never lands is stated"
        );
        // The Customer Portal is configured (MANAGE_SUBSCRIPTION_URL
        // carries its live link since PR #112 shipped), so the fix
        // route is the portal: the buyer updates the payment method
        // there. (Before the portal was activated, this assertion
        // pinned the invoice-email fallback route.)
        assert!(
            email.text.contains("Update your payment method here:")
                && email.text.contains("billing.stripe.com/p/login/"),
            "the fix route is the configured Customer Portal link"
        );
        assert!(email.text.contains(SUPPORT_EMAIL), "the support route is stated");
    }

    #[test]
    fn money_formats_in_minor_units_with_its_currency() {
        assert_eq!(format_money(4900, "usd"), "49.00 USD");
        assert_eq!(format_money(1905, "usd"), "19.05 USD");
        assert_eq!(format_money(2000, "cad"), "20.00 CAD");
        assert_eq!(format_money(0, "usd"), "0.00 USD");
    }

    #[test]
    fn the_refund_email_states_the_full_amount_the_plan_and_the_untouched_key() {
        let email = compose_refund_email(
            "buyer@example.com",
            "hosted",
            1900,
            1900,
            "usd",
            false,
            RefundKeyFate::Unaffected,
        );
        assert!(email.subject.contains("refund"), "the subject carries the news");
        assert!(email.text.contains("19.00 USD"), "the refunded amount is stated");
        assert!(
            email.text.contains("codetopo Team (US$19/month)"),
            "the plan the charge was for is named"
        );
        assert!(
            email.text.contains("does not change your API key"),
            "the refund is not a cancellation, stated plainly"
        );
        assert!(email.text.contains(SUPPORT_EMAIL), "the support route is stated");
    }

    #[test]
    fn the_partial_refund_email_states_the_partial_amount_truthfully() {
        let email = compose_refund_email(
            "buyer@example.com",
            "guardrails",
            2000,
            4900,
            "usd",
            true,
            RefundKeyFate::Unaffected,
        );
        assert!(
            email.text.contains("20.00 USD of the 49.00 USD charge"),
            "the partial amount and the whole charge are both stated"
        );
        assert!(
            email.text.contains("partial refund"),
            "the refund is called partial, not dressed as full"
        );
        assert!(
            email.text.contains("codetopo Guardrails (US$49/month)"),
            "the Guardrails plan is named"
        );
        assert!(
            email.text.contains("The rest of the charge stands"),
            "what happens to the un-refunded part is stated"
        );
    }

    #[test]
    fn the_refund_email_reads_the_key_fate_from_the_subscription_state() {
        let ends = 1_800_000_000 + 30 * 86_400;
        let scheduled = compose_refund_email(
            "buyer@example.com",
            "hosted",
            1900,
            1900,
            "usd",
            false,
            RefundKeyFate::WorksUntil(ends),
        );
        assert!(
            scheduled.text.contains("keeps working until 14 February 2027"),
            "a scheduled end is stated as its date"
        );
        let stopped = compose_refund_email(
            "buyer@example.com",
            "hosted",
            1900,
            1900,
            "usd",
            false,
            RefundKeyFate::AlreadyStopped,
        );
        assert!(
            stopped.text.contains("already stopped working"),
            "an ended subscription's key is stated as stopped"
        );
    }

    /// A one-shot plaintext SMTP sink on loopback: speaks just
    /// enough SMTP for a real lettre conversation and reports the
    /// transcript, so the production transport is proven end to end
    /// with no outbound mail.
    fn smtp_sink() -> (u16, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind sink");
        let port = listener.local_addr().expect("sink addr").port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .expect("timeout");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut writer = stream;
            let mut transcript = String::new();
            let send = |writer: &mut std::net::TcpStream, line: &str| {
                writer.write_all(line.as_bytes()).expect("sink write");
            };
            send(&mut writer, "220 sink ready\r\n");
            let mut in_data = false;
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
                transcript.push_str(&line);
                if in_data {
                    if line == ".\r\n" {
                        in_data = false;
                        send(&mut writer, "250 OK queued\r\n");
                    }
                    continue;
                }
                let upper = line.to_ascii_uppercase();
                if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                    send(&mut writer, "250 localhost\r\n");
                } else if upper.starts_with("MAIL FROM") || upper.starts_with("RCPT TO") {
                    send(&mut writer, "250 OK\r\n");
                } else if upper.starts_with("DATA") {
                    in_data = true;
                    send(&mut writer, "354 end data with a single dot\r\n");
                } else if upper.starts_with("QUIT") {
                    send(&mut writer, "221 bye\r\n");
                    break;
                } else {
                    send(&mut writer, "250 OK\r\n");
                }
            }
            let _ = tx.send(transcript);
        });
        (port, rx)
    }

    #[test]
    fn the_smtp_mailer_delivers_to_a_local_sink() {
        let (port, rx) = smtp_sink();
        let mailer = SmtpMailer::new(MailConfig {
            host: "127.0.0.1".to_string(),
            port,
            username: None,
            password: None,
            from: "codetopo@aimlds.org".to_string(),
            tls: MailTls::None,
        })
        .expect("mailer");
        mailer.send_key_email(&composed()).expect("send to sink");
        let transcript = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("sink transcript");
        // Failure messages never interpolate the transcript: it
        // carries a credential, and assertion output is a log
        // surface — the same rule the composition tests state
        // (release-gate delta finding D-2).
        assert!(
            transcript.contains("MAIL FROM:<codetopo@aimlds.org>"),
            "the envelope sender is the configured from address"
        );
        assert!(
            transcript.contains("RCPT TO:<buyer@example.com>"),
            "the envelope recipient is the buyer"
        );
        assert_eq!(
            transcript.matches("sub_Example:deadbeefcafe").count(),
            2,
            "the credential crosses the wire as the key and in the example"
        );
        assert!(
            transcript.contains("Subject: Your codetopo API key"),
            "the subject line crosses the wire"
        );
    }

    #[test]
    fn the_smtp_mailer_delivers_a_lifecycle_email_to_a_local_sink() {
        let (port, rx) = smtp_sink();
        let mailer = SmtpMailer::new(MailConfig {
            host: "127.0.0.1".to_string(),
            port,
            username: None,
            password: None,
            from: "codetopo@aimlds.org".to_string(),
            tls: MailTls::None,
        })
        .expect("mailer");
        let email = compose_subscription_ended_email("buyer@example.com");
        mailer.send_lifecycle_email(&email).expect("send to sink");
        let transcript = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("sink transcript");
        assert!(
            transcript.contains("RCPT TO:<buyer@example.com>"),
            "the envelope recipient is the buyer"
        );
        assert!(
            transcript.contains("Subject: Your codetopo subscription has ended"),
            "the lifecycle subject crosses the wire"
        );
    }

    #[test]
    fn a_refused_connection_is_a_send_failure_not_a_panic() {
        // Bind then drop a listener so the port is (almost surely) closed.
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("addr").port()
        };
        let mailer = SmtpMailer::new(MailConfig {
            host: "127.0.0.1".to_string(),
            port,
            username: None,
            password: None,
            from: "codetopo@aimlds.org".to_string(),
            tls: MailTls::None,
        })
        .expect("mailer");
        assert!(mailer.send_key_email(&composed()).is_err());
    }
}
