// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Key delivery by email: the second channel (owner directive,
//! 2026-10-18 — "one email is the receipt, and another one is the
//! key").
//!
//! Stripe's receipt tells the buyer they paid; this module's message
//! tells them what they bought. When SMTP submission is configured
//! (see [`crate::config::MailConfig`]), [`crate::service`] composes
//! one message per issued key and sends it to the address the
//! checkout session carried. The message carries the credential in
//! full exactly once, the API base, the plan's limits, and the
//! support address — plain text, plain words.
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
    /// The plain-text body. Carries the credential exactly once.
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
}

/// Composes the key email for one issued key. The limits are read
/// from the plan's own table, so the email can never promise a
/// different envelope than the server enforces.
pub fn compose_key_email(to: &str, key_id: &str, secret: &str, limits: &PlanLimits) -> KeyEmail {
    let plan_line = match (limits.max_active_indexes, limits.max_queries_per_month) {
        (Some(indexes), Some(queries)) => format!(
            "Your hosted plan includes up to {} active indexes and {} queries per month.",
            indexes,
            thousands(queries)
        ),
        _ => "Your plan carries no fixed cap on active indexes or monthly queries.".to_string(),
    };
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
         If you lose the key or it stops working, contact support at\n\
         {SUPPORT_EMAIL} with your Stripe receipt.\n"
    );
    KeyEmail {
        to: to.to_string(),
        subject: "Your codetopo API key".to_string(),
        text,
    }
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

impl KeyMailer for SmtpMailer {
    fn send_key_email(&self, email: &KeyEmail) -> Result<(), MailError> {
        let to: Mailbox = email
            .to
            .parse()
            .map_err(|err| MailError(format!("buyer address is unusable: {err}")))?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(email.subject.clone())
            .header(ContentType::TEXT_PLAIN)
            .body(email.text.clone())
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
    fn the_message_carries_the_credential_exactly_once() {
        let email = composed();
        assert_eq!(email.to, "buyer@example.com");
        assert_eq!(
            email.text.matches("sub_Example:deadbeefcafe").count(),
            1,
            "the full credential appears once, never repeated"
        );
        // The bearer line names the shape, not the secret.
        assert!(email.text.contains("Authorization: Bearer <key>"));
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
            1,
            "the credential crosses the wire exactly once"
        );
        assert!(
            transcript.contains("Subject: Your codetopo API key"),
            "the subject line crosses the wire"
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
