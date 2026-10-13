<!-- Copyright (C) 2026 Nrupal Akolkar -->
<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
<!-- Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens. -->

# ACP trust setup: how an account-free agent does Terminal Auth

The ACP Registry admits only agents that return `authMethods` from
`initialize`, with at least one method of type `agent` or `terminal`
(see the registry's `AUTHENTICATION.md`). codetopo has no accounts,
no provider, and nothing to sign in to — inventing an OAuth flow to
satisfy the form would be false advertising. What codetopo *does*
have is exactly one secret: the HMAC key that signs proof
certificates (and verifies them). The setup below provisions that
key, and the key is genuinely consumed — it signs the snapshots the
agent takes.

## The shape

- **Advertisement.** Until setup is recorded, `initialize` returns
  one method — `codetopo-trust-setup`, type `terminal`, args
  `["login"]` — and only to clients that declare terminal-auth
  support (`clientCapabilities.auth.terminal`; the nested
  `auth.terminal` spelling is accepted too), as the spec requires.
  After setup, `authMethods` is empty.
- **Setup is never a gate.** Sessions work fully with zero setup;
  setup only unlocks signed snapshots. A client that cannot run
  terminal methods loses nothing.
- **The flow.** `codetopo-acp login` explains the model in plain
  words, reads the key with hidden entry (stdin fallback when there
  is no terminal), and records the outcome in the agent's config
  directory (`$CODETOPO_ACP_CONFIG_DIR`, else `$XDG_CONFIG_HOME/
  codetopo`, else `~/.config/codetopo`): `verify.key` (mode 0600)
  for a provisioned key, `setup.keyless` for the keyless choice.
  It prints the signer's fingerprint — the first 16 hex of the
  key's SHA-256, the same identifier certificates carry as
  `signer_key_id` — so the operator can confirm it matches the key
  they meant to trust.
- **Precedence.** `CODETOPO_HMAC_KEY` (the CLI's existing
  convention) wins over the stored key and counts as setup.
- **Consumption.** The snapshot intent passes the key to
  `codetopo_cli::snapshot_db`; the answer cites the certificate
  serial and signer fingerprint from the `.proof` sidecar. Without
  a key it says so plainly.

## Verification

- Unit tests pin the advertisement rules (offered before setup to
  terminal-capable clients under both capability spellings; absent
  without the capability; absent after setup) and the setup store
  (round-trip, replacement, fingerprint formula, 0600 on Unix).
- The live initialize frame carrying the terminal method validates
  against the official ACP v1 JSON schema, and the flow was driven
  end-to-end against the built binary in the shape the registry's
  own verifier uses (fresh HOME, flat terminal capability).
