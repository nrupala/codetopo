// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Local trust setup for the ACP agent.
//!
//! codetopo has no accounts and nothing to sign in to: no server, no
//! provider, no credentials. The one secret in the whole system is
//! the HMAC key that signs codetopo proof certificates (and verifies
//! them) — see `codetopo_cli::write_snapshot_file`. The ACP agent's
//! terminal-auth setup (`codetopo-acp login`) provisions that key on
//! this machine only:
//!
//! - With a key, snapshots the agent takes are signed, and the key's
//!   fingerprint (first 16 hex of its SHA-256 — the same identifier
//!   certificates carry as `signer_key_id`) names the signer without
//!   revealing the key.
//! - Keyless setup is a first-class choice: the agent then takes
//!   unsigned snapshots, exactly as it did before setup existed.
//!
//! Setup state lives in the agent's config directory
//! (`$CODETOPO_ACP_CONFIG_DIR`, else `$XDG_CONFIG_HOME/codetopo`,
//! else `~/.config/codetopo`): `verify.key` (mode 0600) when a key
//! was provisioned, `setup.keyless` when the operator chose keyless.
//! The `CODETOPO_HMAC_KEY` environment variable — the convention the
//! CLI already uses — always wins over the stored key and counts as
//! setup on its own. Nothing here touches the network.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The agent's config directory, resolved from the environment.
pub fn config_dir() -> PathBuf {
    if let Ok(d) = std::env::var("CODETOPO_ACP_CONFIG_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("codetopo");
        }
    }
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return PathBuf::from(h).join(".config").join("codetopo");
        }
    }
    std::env::temp_dir().join("codetopo-acp-config")
}

fn key_file(dir: &Path) -> PathBuf {
    dir.join("verify.key")
}

fn keyless_file(dir: &Path) -> PathBuf {
    dir.join("setup.keyless")
}

/// The HMAC key from the environment, when set and non-empty.
fn env_key() -> Option<Vec<u8>> {
    std::env::var("CODETOPO_HMAC_KEY")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(String::into_bytes)
}

/// Whether trust setup has been recorded in `dir` (or via the
/// environment): a provisioned key, a keyless marker, or the
/// `CODETOPO_HMAC_KEY` convention all count.
pub fn setup_recorded_in(dir: &Path) -> bool {
    env_key().is_some() || key_file(dir).exists() || keyless_file(dir).exists()
}

/// Whether trust setup has been recorded (default config dir).
pub fn setup_recorded() -> bool {
    setup_recorded_in(&config_dir())
}

/// The effective HMAC key for signing/verifying in `dir`:
/// environment first, then the provisioned key file.
pub fn hmac_key_in(dir: &Path) -> Option<Vec<u8>> {
    if let Some(k) = env_key() {
        return Some(k);
    }
    let text = std::fs::read_to_string(key_file(dir)).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.as_bytes().to_vec())
    }
}

/// The effective HMAC key (default config dir).
pub fn hmac_key() -> Option<Vec<u8>> {
    hmac_key_in(&config_dir())
}

/// A key's public identifier: the first 16 hex characters of its
/// SHA-256 — exactly the `signer_key_id` certificates carry.
pub fn signer_key_id(key: &[u8]) -> String {
    codetopo_store::sha256_hex(key)[..16].to_string()
}

/// Record the setup outcome in `dir`: `Some(key)` provisions the key
/// (file mode 0600 on Unix), `None` records the keyless choice. Each
/// replaces the other — re-running setup is how the choice changes.
pub fn record_setup_in(dir: &Path, key: Option<&str>) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => {
            let path = key_file(dir);
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&path)?;
            f.write_all(k.as_bytes())?;
            f.write_all(b"\n")?;
            let _ = std::fs::remove_file(keyless_file(dir));
        }
        None => {
            std::fs::write(
                keyless_file(dir),
                "keyless setup recorded by codetopo-acp login\n",
            )?;
            let _ = std::fs::remove_file(key_file(dir));
        }
    }
    Ok(())
}

/// The interactive terminal setup behind `codetopo-acp login`
/// (ACP Terminal Auth: the client re-runs the agent binary with
/// `login` in an interactive terminal; a zero exit status signals
/// success). Returns the process exit code.
pub fn run_trust_setup() -> i32 {
    let dir = config_dir();
    println!("codetopo — proof-verification setup");
    println!();
    println!("codetopo has no account and nothing to sign in to. The only");
    println!("secret in the system is the HMAC key that signs proof");
    println!("certificates for snapshots — and lets you verify them.");
    println!("Provisioning it here means snapshots this agent takes are");
    println!("signed. It is stored only on this machine:");
    println!("  {}", dir.display());
    println!();
    let entered = match rpassword::prompt_password(
        "HMAC proof-verification key (blank = keyless setup): ",
    ) {
        Ok(s) => s,
        Err(_) => {
            // No interactive terminal (piped or embedded launch):
            // fall back to one line on stdin so the setup can still
            // complete, noting that the input will echo there.
            eprintln!("HMAC proof-verification key (blank = keyless setup): ");
            eprintln!("(no interactive terminal — reading from stdin; the key will echo)");
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(_) => line,
                Err(e) => {
                    eprintln!("could not read the key: {e}");
                    return 1;
                }
            }
        }
    };
    let key = entered.trim();
    match record_setup_in(&dir, if key.is_empty() { None } else { Some(key) }) {
        Ok(()) => {
            if key.is_empty() {
                println!("Keyless setup recorded. Snapshots will be unsigned;");
                println!("run `codetopo-acp login` again to provision a key.");
            } else {
                println!(
                    "Key provisioned. Signer fingerprint: {}",
                    signer_key_id(key.as_bytes())
                );
                println!("Certificates signed under this key carry that");
                println!("fingerprint as their signer_key_id — check it");
                println!("matches before trusting a certificate.");
            }
            0
        }
        Err(e) => {
            eprintln!("could not record setup in {}: {e}", dir.display());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "codetopo-acp-trust-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn keyless_setup_records_and_reads_back() {
        let dir = temp_dir("keyless");
        assert!(!setup_recorded_in(&dir));
        assert!(hmac_key_in(&dir).is_none());
        record_setup_in(&dir, None).expect("record keyless");
        assert!(setup_recorded_in(&dir));
        assert!(hmac_key_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_setup_records_and_reads_back() {
        let dir = temp_dir("keyed");
        record_setup_in(&dir, Some("s3cret")).expect("record key");
        assert!(setup_recorded_in(&dir));
        assert_eq!(hmac_key_in(&dir), Some(b"s3cret".to_vec()));
        // The fingerprint is the certificate signer_key_id formula.
        assert_eq!(
            signer_key_id(b"s3cret"),
            &codetopo_store::sha256_hex(b"s3cret")[..16]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn re_setup_replaces_the_previous_choice() {
        let dir = temp_dir("replace");
        record_setup_in(&dir, Some("k1")).expect("key first");
        record_setup_in(&dir, None).expect("then keyless");
        assert!(hmac_key_in(&dir).is_none());
        record_setup_in(&dir, Some("k2")).expect("then key again");
        assert_eq!(hmac_key_in(&dir), Some(b"k2".to_vec()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn key_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("perms");
        record_setup_in(&dir, Some("s3cret")).expect("record key");
        let mode = std::fs::metadata(key_file(&dir))
            .expect("key file")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
