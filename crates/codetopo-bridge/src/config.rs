// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Bridge configuration, all of it from the environment.
//!
//! The data dir comes from the server crate's own [`Config`], so the
//! bridge and the server always name the same `keys.db` — there is no
//! second path variable to drift. Everything Stripe-shaped is required
//! and has no default: a bridge that boots without its webhook secret or
//! its price id would either reject every real event or mint keys for
//! the wrong product, and both failures are quieter than refusing to
//! start.

use std::net::SocketAddr;
use std::path::PathBuf;

use codetopo_server::config::Config as ServerConfig;
use codetopo_server::keys::DEFAULT_PLAN;

use crate::BridgeError;

/// Default bind for the bridge listener: loopback only, like the
/// server's. A deployment puts a reverse proxy in front (paid-path S5);
/// exposing the listener is that proxy's decision, not a default here.
pub const DEFAULT_BRIDGE_BIND: &str = "127.0.0.1:8788";

/// Default from-address for key-delivery email, on the domain the
/// business sells under. Configuration may override it; the address
/// must be one the SMTP credential is allowed to send as.
pub const DEFAULT_SMTP_FROM: &str = "codetopo@aimlds.org";
/// Default SMTP submission port (STARTTLS submission).
pub const DEFAULT_SMTP_PORT: u16 = 587;

/// How the SMTP conversation is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailTls {
    /// Plain connection upgraded with STARTTLS (submission, usually
    /// port 587). The default: the credential never crosses in clear.
    StartTls,
    /// TLS from the first byte (usually port 465).
    ImplicitTls,
    /// No encryption. Only sane for a loopback sink or a local test
    /// double — a configured password must never be sent this way to
    /// a remote host; the setting exists for hermetic tests.
    /// [`MailConfig::from_lookup`] enforces that: credentials plus
    /// `none` parse only for a loopback host.
    None,
}

/// SMTP submission settings for key-delivery email (see
/// [`crate::mail`]). Optional as a whole: with no host configured
/// the bridge runs exactly as before, keys delivered by the claim
/// page alone. The password lives only in the environment, like
/// every other bridge secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailConfig {
    /// Submission host.
    pub host: String,
    /// Submission port.
    pub port: u16,
    /// Submission username, when the host requires authentication.
    /// Set exactly when `password` is set (enforced at parse time).
    pub username: Option<String>,
    /// Submission password. Environment-only; never logged.
    pub password: Option<String>,
    /// From-address for key emails, on the aimlds.org domain.
    pub from: String,
    /// Transport encryption.
    pub tls: MailTls,
}

/// Resolved bridge configuration.
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// Directory holding the key store (from the server's config).
    pub data_dir: PathBuf,
    /// Path of the shared runtime key store, `<data_dir>/keys.db`.
    pub keys_db_path: PathBuf,
    /// Address the bridge listener binds.
    pub bind: SocketAddr,
    /// Stripe webhook endpoint signing secret (`whsec_…`). Verifies
    /// webhook signatures; never logged, never defaulted.
    pub webhook_secret: String,
    /// The Stripe price id that maps to the hosted plan. Price ids are
    /// configuration, not code (the Conduit pattern): a new price is an
    /// env change, not a release.
    pub price_id: String,
    /// Plan recorded on keys the bridge issues.
    pub plan: String,
    /// AES-256 key sealing unclaimed key deliveries, as 64 hex chars
    /// decoded to 32 bytes. Exists only in the environment.
    pub claim_key: [u8; 32],
    /// Key-delivery email settings, when SMTP submission is
    /// configured. `None` is a supported state — the claim page is
    /// then the only delivery channel, as before this feature.
    pub mail: Option<MailConfig>,
}

impl BridgeConfig {
    /// Reads configuration from the environment, failing closed.
    ///
    /// Recognised variables: the server's `CODETOPO_DATA_DIR` (and its
    /// siblings, via [`ServerConfig::from_env`]), plus
    /// `CODETOPO_BRIDGE_BIND`, `CODETOPO_STRIPE_WEBHOOK_SECRET`,
    /// `CODETOPO_STRIPE_PRICE_ID`, `CODETOPO_CLAIM_KEY`, and optionally
    /// `CODETOPO_PLAN` (default: the hosted plan). Key-delivery email
    /// is configured by the `CODETOPO_SMTP_*` family (see
    /// [`MailConfig::from_lookup`]); unconfigured, it is simply off.
    pub fn from_env() -> Result<Self, BridgeError> {
        let server = ServerConfig::from_env().map_err(|err| {
            BridgeError::Config(format!("server configuration is unusable: {err}"))
        })?;
        let bind_raw = env_string("CODETOPO_BRIDGE_BIND", DEFAULT_BRIDGE_BIND);
        let bind: SocketAddr = bind_raw.parse().map_err(|_| {
            BridgeError::Config(format!(
                "CODETOPO_BRIDGE_BIND must be a 'host:port' socket address, got '{bind_raw}'"
            ))
        })?;
        let webhook_secret = required("CODETOPO_STRIPE_WEBHOOK_SECRET")?;
        let price_id = required("CODETOPO_STRIPE_PRICE_ID")?;
        let plan = env_string("CODETOPO_PLAN", DEFAULT_PLAN);
        let claim_key = parse_claim_key(&required("CODETOPO_CLAIM_KEY")?)?;
        let mail = MailConfig::from_lookup(|name| std::env::var(name).ok())?;
        let keys_db_path = server.keys_db_path();
        Ok(BridgeConfig {
            data_dir: server.data_dir,
            keys_db_path,
            bind,
            webhook_secret,
            price_id,
            plan,
            claim_key,
            mail,
        })
    }
}

impl MailConfig {
    /// Resolves the `CODETOPO_SMTP_*` family through `lookup` (the
    /// environment, in production; a fixture map in tests).
    ///
    /// `CODETOPO_SMTP_HOST` is the switch: absent or empty, email is
    /// off and the rest of the family is ignored. With a host set,
    /// the family is validated strictly — a half-configured mailer
    /// is a startup error, never a silent no-op:
    ///
    /// - `CODETOPO_SMTP_PORT`: default [`DEFAULT_SMTP_PORT`]; must
    ///   parse as a port number.
    /// - `CODETOPO_SMTP_TLS`: `starttls` (default), `tls`, or
    ///   `none`; anything else is an error.
    /// - `CODETOPO_SMTP_USERNAME` / `CODETOPO_SMTP_PASSWORD`:
    ///   both or neither — submission authentication is
    ///   all-or-nothing. The password is never logged.
    /// - Credentials with `CODETOPO_SMTP_TLS=none` are refused for
    ///   any host but loopback: lettre would authenticate in the
    ///   clear, and a comment saying "loopback only" is not a check
    ///   (release-gate delta finding D-1).
    /// - `CODETOPO_SMTP_FROM`: default [`DEFAULT_SMTP_FROM`].
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, BridgeError> {
        let get = |name: &str| -> Option<String> {
            lookup(name).and_then(|value| {
                let trimmed = value.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            })
        };
        let Some(host) = get("CODETOPO_SMTP_HOST") else {
            return Ok(None);
        };
        let port = match get("CODETOPO_SMTP_PORT") {
            None => DEFAULT_SMTP_PORT,
            Some(raw) => raw.parse::<u16>().map_err(|_| {
                BridgeError::Config(format!(
                    "CODETOPO_SMTP_PORT must be a port number, got '{raw}'"
                ))
            })?,
        };
        let tls = match get("CODETOPO_SMTP_TLS").as_deref() {
            None | Some("starttls") => MailTls::StartTls,
            Some("tls") => MailTls::ImplicitTls,
            Some("none") => MailTls::None,
            Some(other) => {
                return Err(BridgeError::Config(format!(
                    "CODETOPO_SMTP_TLS must be 'starttls', 'tls', or 'none', got '{other}'"
                )));
            }
        };
        let username = get("CODETOPO_SMTP_USERNAME");
        let password = get("CODETOPO_SMTP_PASSWORD");
        if username.is_some() != password.is_some() {
            return Err(BridgeError::Config(
                "CODETOPO_SMTP_USERNAME and CODETOPO_SMTP_PASSWORD must be set together"
                    .to_string(),
            ));
        }
        if tls == MailTls::None && password.is_some() && !is_loopback_host(&host) {
            return Err(BridgeError::Config(format!(
                "CODETOPO_SMTP_TLS=none would send the configured credentials \
                 unencrypted to '{host}'; use 'starttls' or 'tls' for a remote \
                 host (TLS 'none' with credentials is accepted only for a \
                 loopback host)"
            )));
        }
        let from = get("CODETOPO_SMTP_FROM").unwrap_or_else(|| DEFAULT_SMTP_FROM.to_string());
        Ok(Some(MailConfig {
            host,
            port,
            username,
            password,
            from,
            tls,
        }))
    }
}

/// Whether an SMTP host names this machine: `localhost`, a
/// 127.0.0.0/8 address, or `::1` — the loopback sink shape the
/// hermetic tests use, and the only shape in which credentials may
/// cross unencrypted (TLS `none`). Anything unparseable is not
/// loopback: the check fails closed.
fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => false,
    }
}

/// A required variable: absent or empty is a configuration error.
fn required(name: &str) -> Result<String, BridgeError> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        _ => Err(BridgeError::Config(format!(
            "{name} must be set (no default exists)"
        ))),
    }
}

fn env_string(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => default.to_string(),
    }
}

/// Decodes the 64-hex-character claim key. Hand-rolled hex (one small
/// decoder, shared with [`crate::stripe`]) — a malformed key is a
/// startup error, never a silent truncation.
fn parse_claim_key(raw: &str) -> Result<[u8; 32], BridgeError> {
    let bytes = crate::stripe::hex_decode(raw).ok_or_else(|| {
        BridgeError::Config("CODETOPO_CLAIM_KEY must be 64 hex characters".to_string())
    })?;
    bytes.try_into().map_err(|_| {
        BridgeError::Config("CODETOPO_CLAIM_KEY must be 64 hex characters (32 bytes)".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn claim_key_must_be_exactly_32_hex_bytes() {
        let good = "ab".repeat(32);
        assert_eq!(parse_claim_key(&good).expect("32 bytes").len(), 32);
        assert!(parse_claim_key("abcd").is_err(), "too short");
        assert!(parse_claim_key(&"zz".repeat(32)).is_err(), "not hex");
        assert!(parse_claim_key(&"ab".repeat(33)).is_err(), "too long");
    }

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn mail_is_off_without_a_host() {
        assert_eq!(MailConfig::from_lookup(lookup(&[])).expect("parse"), None);
        // The rest of the family alone does not switch it on.
        let vars = [("CODETOPO_SMTP_PORT", "2525"), ("CODETOPO_SMTP_FROM", "x@aimlds.org")];
        assert_eq!(MailConfig::from_lookup(lookup(&vars)).expect("parse"), None);
    }

    #[test]
    fn mail_config_defaults_are_submission_shaped() {
        let vars = [("CODETOPO_SMTP_HOST", "smtp.example.com")];
        let config = MailConfig::from_lookup(lookup(&vars))
            .expect("parse")
            .expect("configured");
        assert_eq!(config.host, "smtp.example.com");
        assert_eq!(config.port, DEFAULT_SMTP_PORT);
        assert_eq!(config.tls, MailTls::StartTls);
        assert_eq!(config.from, DEFAULT_SMTP_FROM);
        assert_eq!(config.username, None);
        assert_eq!(config.password, None);
    }

    #[test]
    fn mail_config_reads_the_full_family() {
        let vars = [
            ("CODETOPO_SMTP_HOST", "smtp.example.com"),
            ("CODETOPO_SMTP_PORT", "465"),
            ("CODETOPO_SMTP_TLS", "tls"),
            ("CODETOPO_SMTP_USERNAME", "codetopo@aimlds.org"),
            ("CODETOPO_SMTP_PASSWORD", "app-password"),
            ("CODETOPO_SMTP_FROM", "keys@aimlds.org"),
        ];
        let config = MailConfig::from_lookup(lookup(&vars))
            .expect("parse")
            .expect("configured");
        assert_eq!(config.port, 465);
        assert_eq!(config.tls, MailTls::ImplicitTls);
        assert_eq!(config.username.as_deref(), Some("codetopo@aimlds.org"));
        assert_eq!(config.password.as_deref(), Some("app-password"));
        assert_eq!(config.from, "keys@aimlds.org");
    }

    #[test]
    fn mail_config_rejects_a_half_built_family() {
        let host = ("CODETOPO_SMTP_HOST", "smtp.example.com");
        // Username without its password (and the mirror image).
        let vars = [host, ("CODETOPO_SMTP_USERNAME", "codetopo@aimlds.org")];
        assert!(MailConfig::from_lookup(lookup(&vars)).is_err());
        let vars = [host, ("CODETOPO_SMTP_PASSWORD", "app-password")];
        assert!(MailConfig::from_lookup(lookup(&vars)).is_err());
        // A port that is not a number.
        let vars = [host, ("CODETOPO_SMTP_PORT", "soon")];
        assert!(MailConfig::from_lookup(lookup(&vars)).is_err());
        // A TLS mode the transport does not speak.
        let vars = [host, ("CODETOPO_SMTP_TLS", "carrier-pigeon")];
        assert!(MailConfig::from_lookup(lookup(&vars)).is_err());
    }

    #[test]
    fn mail_config_rejects_credentials_over_plaintext_to_a_remote_host() {
        // D-1: TLS `none` + credentials would AUTH in the clear to
        // whatever host is configured. Rejected at parse time, with
        // the problem named.
        let vars = [
            ("CODETOPO_SMTP_HOST", "smtp.example.com"),
            ("CODETOPO_SMTP_TLS", "none"),
            ("CODETOPO_SMTP_USERNAME", "codetopo@aimlds.org"),
            ("CODETOPO_SMTP_PASSWORD", "app-password"),
        ];
        let err = MailConfig::from_lookup(lookup(&vars))
            .expect_err("credentials + plaintext + remote host is refused");
        assert!(
            err.to_string().contains("unencrypted"),
            "the error names the problem"
        );
        // Encrypted modes to the same host with the same credentials
        // are the supported shape and keep parsing.
        for tls in ["starttls", "tls"] {
            let vars = [
                ("CODETOPO_SMTP_HOST", "smtp.example.com"),
                ("CODETOPO_SMTP_TLS", tls),
                ("CODETOPO_SMTP_USERNAME", "codetopo@aimlds.org"),
                ("CODETOPO_SMTP_PASSWORD", "app-password"),
            ];
            assert!(
                MailConfig::from_lookup(lookup(&vars)).is_ok(),
                "credentials over {tls} parse"
            );
        }
    }

    #[test]
    fn mail_config_allows_credentials_over_plaintext_on_loopback_only() {
        // The hermetic-sink shape: plaintext on this machine only,
        // where nothing crosses a wire.
        for host in ["127.0.0.1", "localhost", "::1"] {
            let vars = [
                ("CODETOPO_SMTP_HOST", host),
                ("CODETOPO_SMTP_TLS", "none"),
                ("CODETOPO_SMTP_USERNAME", "codetopo@aimlds.org"),
                ("CODETOPO_SMTP_PASSWORD", "app-password"),
            ];
            let config = MailConfig::from_lookup(lookup(&vars))
                .expect("loopback parse")
                .expect("configured");
            assert_eq!(config.tls, MailTls::None, "host {host}");
        }
        // Plaintext without credentials exposes no secret; a remote
        // host in that shape still parses (e.g. a trusted relay on a
        // private network is the operator's call, not a leak).
        let vars = [
            ("CODETOPO_SMTP_HOST", "smtp.example.com"),
            ("CODETOPO_SMTP_TLS", "none"),
        ];
        let config = MailConfig::from_lookup(lookup(&vars))
            .expect("parse")
            .expect("configured");
        assert_eq!(config.tls, MailTls::None);
        assert_eq!(config.username, None);
    }
}
