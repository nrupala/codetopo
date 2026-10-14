// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Hosted ingest: getting a stranger's repository onto the box
//! (paid-path plan S2, gap G1).
//!
//! Until now `POST /v1/index` accepted only a path on the server host,
//! which is a fine contract for the operator's own keys and a useless
//! one for a paying tenant: there was no way in. This module is the way
//! in. Both hosted modes land the tree in a server-owned workdir —
//! `<data_dir>/ingest/<index_id>/` — never at a caller-chosen host path:
//!
//! * **Clone.** A public `https://` git URL, shallow-cloned with the
//!   system `git`. The URL is validated before `git` ever runs: HTTPS
//!   only, no embedded credentials, port 443 only, and the host must
//!   resolve exclusively to public addresses (see
//!   [`validate_repo_url`]). The clone itself is hardened: argument
//!   array (never a shell), `GIT_TERMINAL_PROMPT=0` so a private repo
//!   fails instead of prompting, redirects refused so a checked URL
//!   cannot bounce the clone somewhere unchecked, submodules not
//!   recursed, LFS smudge skipped, a wall-clock timeout, and a cap on
//!   the cloned tree's size.
//! * **Archive upload.** A `.zip` or `.tar.gz` body, extracted entry by
//!   entry with every path proven to stay inside the workdir before it
//!   is written: traversal entries (`../…`, absolute paths) are
//!   refused, and so are link entries — a symlink planted by one entry
//!   and written through by the next is the classic extraction escape,
//!   so archives carrying links do not ingest at all. Extracted bytes
//!   are counted as they are written, not trusted from headers, so a
//!   lying header cannot smuggle an expansion bomb past the cap.
//!
//! The caps below are interim constants, named so the abuse-envelope
//! step (plan S3) can turn them into per-plan policy without hunting
//! literals. A failed ingest removes its workdir: a partial tree is
//! never left behind for a later sweep to mistake for a live index.

use std::io::{Cursor, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use url::{Host, Url};

use crate::error::ApiError;

/// Largest archive body `POST /v1/index` accepts, in bytes.
pub const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;

/// Largest tree an ingest may produce, in bytes — extracted bytes for
/// an archive, on-disk bytes for a clone. Counted as written/measured,
/// never trusted from a header.
pub const MAX_TREE_BYTES: u64 = 512 * 1024 * 1024;

/// Most entries one archive may carry. Bounds the work a single upload
/// can make the extractor do before any byte is counted.
pub const MAX_ARCHIVE_ENTRIES: usize = 100_000;

/// Wall-clock budget for one `git clone`. A clone that outlives it is
/// killed, not waited on: ingest is synchronous from the client's side
/// of the request, and an unbounded clone is an unbounded request.
pub const CLONE_TIMEOUT_SECS: u64 = 120;

// ---------------------------------------------------------------------------
// Repository URLs: validation, then clone
// ---------------------------------------------------------------------------

/// Parses and statically validates a `repo_url`.
///
/// The checks that need no network: the string parses, the scheme is
/// exactly `https`, no credentials are embedded (`user:pass@` in a URL
/// is both a secret leak into logs and an ambiguity about whose
/// repository this is), a host is present, and the port is 443 or the
/// HTTPS default. Address-class checks need resolution and live in
/// [`validate_repo_url`].
pub(crate) fn parse_repo_url(raw: &str) -> Result<Url, ApiError> {
    let url = Url::parse(raw.trim())
        .map_err(|_| ApiError::invalid_repo_url("not a valid URL"))?;
    if url.scheme() != "https" {
        return Err(ApiError::invalid_repo_url(
            "only https:// URLs are accepted",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ApiError::invalid_repo_url(
            "URLs with embedded credentials are not accepted",
        ));
    }
    if url.host().is_none() {
        return Err(ApiError::invalid_repo_url("the URL has no host"));
    }
    if let Some(port) = url.port() {
        if port != 443 {
            return Err(ApiError::invalid_repo_url(
                "only port 443 (the HTTPS default) is accepted",
            ));
        }
    }
    Ok(url)
}

/// Fully validates a `repo_url`, including where its host points.
///
/// The SSRF guard: every address the host resolves to must be a public
/// address. A name that resolves to even one loopback, private,
/// link-local, or otherwise non-public address is refused whole —
/// accepting the public answers and letting `git` pick would race the
/// refusal. IP-literal hosts are checked directly, with no lookup.
///
/// **Residual limit, stated plainly:** this is a check-then-clone.
/// `git` resolves the name again when it connects, and a hostile DNS
/// answer could change between the two resolutions (rebinding). The
/// check removes the honest-attacker and misconfiguration cases —
/// pointing the server at itself or at the LAN by name or by literal —
/// and the clone's refusal to follow redirects keeps the checked URL
/// the connected URL. Closing the rebinding window entirely needs
/// connect-time pinning, which `git`'s HTTPS transport does not offer;
/// S3's envelope work owns any further hardening here.
pub async fn validate_repo_url(raw: &str) -> Result<Url, ApiError> {
    let url = parse_repo_url(raw)?;
    match url.host() {
        Some(Host::Ipv4(ip)) => {
            require_public(IpAddr::V4(ip), url.host_str().unwrap_or(""))?;
        }
        Some(Host::Ipv6(ip)) => {
            require_public(IpAddr::V6(ip), url.host_str().unwrap_or(""))?;
        }
        Some(Host::Domain(domain)) => {
            let domain = domain.strip_suffix('.').unwrap_or(domain);
            if domain.eq_ignore_ascii_case("localhost") {
                return Err(non_public_host(domain));
            }
            let addresses = tokio::net::lookup_host((domain, 443))
                .await
                .map_err(|_| ApiError::invalid_repo_url("the host does not resolve"))?;
            let mut seen_any = false;
            for address in addresses {
                seen_any = true;
                require_public(address.ip(), domain)?;
            }
            if !seen_any {
                return Err(ApiError::invalid_repo_url("the host does not resolve"));
            }
        }
        None => return Err(ApiError::invalid_repo_url("the URL has no host")),
    }
    Ok(url)
}

fn require_public(ip: IpAddr, host: &str) -> Result<(), ApiError> {
    if is_public_ip(ip) {
        Ok(())
    } else {
        Err(non_public_host(host))
    }
}

fn non_public_host(host: &str) -> ApiError {
    ApiError::invalid_repo_url(&format!(
        "host '{host}' resolves to a non-public address; only public repositories are ingested"
    ))
}

/// True for addresses a public repository host may legitimately have:
/// globally routable, and none of the loopback / private / link-local /
/// shared / benchmarking / documentation / reserved classes an SSRF
/// guard exists to keep a server from dialing on a caller's behalf.
pub(crate) fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        || octets[0] == 0 // 0.0.0.0/8, "this network"
        || (octets[0] == 100 && (octets[1] & 0xC0) == 0x40) // 100.64.0.0/10, shared/CGNAT
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0) // 192.0.0.0/24, protocol assignments
        || (octets[0] == 198 && (octets[1] & 0xFE) == 18) // 198.18.0.0/15, benchmarking
        || octets[0] >= 240) // 240.0.0.0/4, reserved
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    // An IPv4-mapped IPv6 address is judged by the IPv4 rules: the
    // mapped form must not be a laundering route around them.
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let segments = ip.segments();
    !(ip.is_loopback()
        || ip.is_multicast()
        || ip.is_unspecified()
        || (segments[0] & 0xFE00) == 0xFC00 // fc00::/7, unique local
        || (segments[0] & 0xFFC0) == 0xFE80 // fe80::/10, link local
        || (segments[0] == 0x2001 && segments[1] == 0x0DB8)) // 2001:db8::/32, documentation
}

/// The default package name for a cloned repository: the URL's last
/// path segment, minus a `.git` suffix (`…/acme/api.git` → `api`).
pub(crate) fn package_name_for_url(url: &Url) -> String {
    let segment = url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|s| !s.is_empty()))
        .unwrap_or("");
    let name = segment.strip_suffix(".git").unwrap_or(segment);
    if name.is_empty() {
        "repo".to_string()
    } else {
        name.to_string()
    }
}

/// Clones `url` into `workdir` (created here) and checks the tree
/// against [`MAX_TREE_BYTES`]. On any failure the workdir is removed:
/// a partial clone is not an index and must not linger as one.
pub fn clone_into(url: &Url, workdir: &Path) -> Result<(), ApiError> {
    let result = clone_into_inner(url, workdir);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(workdir);
    }
    result
}

fn clone_into_inner(url: &Url, workdir: &Path) -> Result<(), ApiError> {
    std::fs::create_dir_all(workdir).map_err(|err| {
        ApiError::internal(format!(
            "cannot create ingest workdir {}: {err}",
            workdir.display()
        ))
    })?;
    run_git_clone(
        url.as_str(),
        workdir,
        Duration::from_secs(CLONE_TIMEOUT_SECS),
    )?;
    let bytes = tree_bytes(workdir, MAX_TREE_BYTES);
    if bytes > MAX_TREE_BYTES {
        return Err(ApiError::invalid_repo_url(&format!(
            "the cloned tree is {bytes} bytes, past the {MAX_TREE_BYTES}-byte ingest limit"
        )));
    }
    Ok(())
}

/// The clone plumbing: runs `git clone` for an already-validated URL.
///
/// **Callers must validate first** ([`validate_repo_url`]); this
/// function deliberately takes the URL as an opaque argument so tests
/// can drive it at a local `git daemon`, and it applies none of the
/// public-host policy itself. What it always applies is process
/// hygiene: an argument array (the URL can never become a shell line
/// or an option — `--` ends option parsing), no terminal prompt, no
/// redirects, no submodule recursion, no LFS smudge (a clone-time
/// network fetch to endpoints the repository chooses), the `ext::`
/// transport disabled (it executes commands), and a kill past
/// `timeout`.
pub(crate) fn run_git_clone(url: &str, dest: &Path, timeout: Duration) -> Result<(), ApiError> {
    let dest_arg = dest
        .to_str()
        .ok_or_else(|| ApiError::internal("ingest workdir path is not valid UTF-8"))?;
    let mut child = Command::new("git")
        .args([
            "-c",
            "http.followRedirects=false",
            "-c",
            "protocol.ext.allow=never",
            "clone",
            "--depth",
            "1",
            "--single-branch",
            "--no-recurse-submodules",
            "--",
            url,
            dest_arg,
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| ApiError::internal(format!("cannot run git: {err}")))?;

    // Drain stderr on a thread: a clone that fills the pipe buffer
    // would block mid-write and look exactly like a hang, and the
    // captured tail is the error message when the clone fails.
    let stderr_pipe = child.stderr.take();
    let drain = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = drain.join();
                    return Err(ApiError::invalid_repo_url(&format!(
                        "git clone did not finish within {}s",
                        timeout.as_secs()
                    )));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => {
                let _ = drain.join();
                return Err(ApiError::internal(format!("cannot wait on git clone: {err}")));
            }
        }
    };
    let stderr = drain.join().unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&stderr);
    let last_line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("no error output");
    let last_line: String = last_line.chars().take(300).collect();
    Err(ApiError::invalid_repo_url(&format!(
        "git clone failed: {last_line}"
    )))
}

/// Total bytes of regular files under `root`, symlinks not followed.
/// Stops early once past `cap` — the caller only needs to know the cap
/// fell, not by how much.
fn tree_bytes(root: &Path, cap: u64) -> u64 {
    let mut total: u64 = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() {
                total += entry.metadata().map(|meta| meta.len()).unwrap_or(0);
                if total > cap {
                    return total;
                }
            }
        }
    }
    total
}

// ---------------------------------------------------------------------------
// Archive upload: sniff, then extract with containment
// ---------------------------------------------------------------------------

/// The archive formats ingest accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    /// `.zip`, read through the `zip` crate.
    Zip,
    /// `.tar.gz`, read through `tar` over a gzip decoder.
    TarGz,
}

/// Decides what a request body is: the declared content type first,
/// the magic bytes when the declaration is missing or generic
/// (`application/octet-stream`). `None` is neither archive format —
/// the handler answers `invalid_body`.
pub fn archive_kind(content_type: Option<&str>, bytes: &[u8]) -> Option<ArchiveKind> {
    match content_type {
        Some("application/zip") | Some("application/x-zip-compressed") => Some(ArchiveKind::Zip),
        Some("application/gzip") | Some("application/x-gzip") => Some(ArchiveKind::TarGz),
        _ => {
            if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
                Some(ArchiveKind::Zip)
            } else if bytes.starts_with(b"\x1f\x8b") {
                Some(ArchiveKind::TarGz)
            } else {
                None
            }
        }
    }
}

/// Extracts an archive body into `workdir` (created here) and returns
/// the directory to index. On any failure the workdir is removed.
///
/// Archives conventionally wrap the tree in one top-level directory
/// (`repo-main/…`); when the extracted workdir holds exactly one entry
/// and it is a directory, that directory is the index root, so node
/// paths match what a clone of the same repository produces. Files at
/// the archive root index from the workdir itself.
pub fn extract_into(kind: ArchiveKind, bytes: &[u8], workdir: &Path) -> Result<PathBuf, ApiError> {
    let result = extract_into_inner(kind, bytes, workdir);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(workdir);
    }
    result
}

fn extract_into_inner(
    kind: ArchiveKind,
    bytes: &[u8],
    workdir: &Path,
) -> Result<PathBuf, ApiError> {
    std::fs::create_dir_all(workdir).map_err(|err| {
        ApiError::internal(format!(
            "cannot create ingest workdir {}: {err}",
            workdir.display()
        ))
    })?;
    match kind {
        ArchiveKind::Zip => extract_zip(bytes, workdir, MAX_ARCHIVE_ENTRIES, MAX_TREE_BYTES),
        ArchiveKind::TarGz => {
            extract_tar_gz(bytes, workdir, MAX_ARCHIVE_ENTRIES, MAX_TREE_BYTES)
        }
    }?;
    Ok(index_root(workdir))
}

/// The single-top-level-directory unwrap described on [`extract_into`].
fn index_root(workdir: &Path) -> PathBuf {
    let Ok(entries) = std::fs::read_dir(workdir) else {
        return workdir.to_path_buf();
    };
    let entries: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    if entries.len() == 1 && entries[0].is_dir() {
        return entries.into_iter().next().expect("one entry");
    }
    workdir.to_path_buf()
}

/// Reduces an archive entry's path to a relative path proven to stay
/// inside the extraction root: `Normal` components only, `.` ignored.
/// Absolute paths, `..`, and platform prefixes are refused (`None`) —
/// this check, applied before any join, is the zip-slip/tar-slip guard.
fn enclosed_relative(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// Copies `reader` into `file`, counting actual bytes against
/// `budget`. Reading past the budget is the error, so a decompressed
/// size that exceeds its declared size is caught by the same check as
/// an honestly huge file.
fn copy_capped(
    reader: &mut impl Read,
    file: &mut std::fs::File,
    budget: u64,
) -> Result<u64, ApiError> {
    let mut written: u64 = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|err| ApiError::invalid_body(&format!("cannot read archive entry: {err}")))?;
        if read == 0 {
            return Ok(written);
        }
        written += read as u64;
        if written > budget {
            return Err(ApiError::invalid_body(&format!(
                "archive expands past the {MAX_TREE_BYTES}-byte ingest limit"
            )));
        }
        file.write_all(&buffer[..read])
            .map_err(|err| ApiError::internal(format!("cannot write extracted file: {err}")))?;
    }
}

fn escape_error() -> ApiError {
    ApiError::invalid_body("archive entry path escapes the extraction root")
}

fn extract_zip(
    bytes: &[u8],
    dest: &Path,
    max_entries: usize,
    max_bytes: u64,
) -> Result<(), ApiError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|err| {
        ApiError::invalid_body(&format!("not a readable zip archive: {err}"))
    })?;
    if archive.len() > max_entries {
        return Err(ApiError::invalid_body(&format!(
            "archive carries {} entries, past the {max_entries}-entry ingest limit",
            archive.len()
        )));
    }
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|err| ApiError::invalid_body(&format!("cannot read archive entry: {err}")))?;
        // enclosed_name is the zip crate's own sanitised view; the
        // component check over it is repeated here anyway, so both
        // formats pass through the one guard in this module.
        let enclosed = entry.enclosed_name().ok_or_else(escape_error)?;
        let rel = enclosed_relative(&enclosed).ok_or_else(escape_error)?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|err| {
                ApiError::internal(format!("cannot create extracted directory: {err}"))
            })?;
            continue;
        }
        if entry.is_symlink() {
            return Err(ApiError::invalid_body(
                "archive contains a symbolic link, which ingest does not accept",
            ));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                ApiError::internal(format!("cannot create extracted directory: {err}"))
            })?;
        }
        let mut file = std::fs::File::create(&target).map_err(|err| {
            ApiError::internal(format!("cannot write extracted file: {err}"))
        })?;
        total += copy_capped(&mut entry, &mut file, max_bytes - total)?;
    }
    Ok(())
}

fn extract_tar_gz(
    bytes: &[u8],
    dest: &Path,
    max_entries: usize,
    max_bytes: u64,
) -> Result<(), ApiError> {
    let decoder = flate2::read::GzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|err| ApiError::invalid_body(&format!("not a readable tar.gz archive: {err}")))?;
    let mut total: u64 = 0;
    let mut count: usize = 0;
    for entry in entries {
        let mut entry = entry
            .map_err(|err| ApiError::invalid_body(&format!("cannot read archive entry: {err}")))?;
        count += 1;
        if count > max_entries {
            return Err(ApiError::invalid_body(&format!(
                "archive carries more than {max_entries} entries, the ingest limit"
            )));
        }
        let entry_path = entry
            .path()
            .map_err(|err| ApiError::invalid_body(&format!("cannot read archive entry path: {err}")))?
            .into_owned();
        let rel = enclosed_relative(&entry_path).ok_or_else(escape_error)?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dest.join(&rel);
        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(&target).map_err(|err| {
                    ApiError::internal(format!("cannot create extracted directory: {err}"))
                })?;
            }
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|err| {
                        ApiError::internal(format!("cannot create extracted directory: {err}"))
                    })?;
                }
                let mut file = std::fs::File::create(&target).map_err(|err| {
                    ApiError::internal(format!("cannot write extracted file: {err}"))
                })?;
                total += copy_capped(&mut entry, &mut file, max_bytes - total)?;
            }
            // Symlinks, hard links, devices, fifos: refused wholesale.
            // A link written by one entry is a path out of the root for
            // every later entry, and ingest needs none of them.
            _ => {
                return Err(ApiError::invalid_body(
                    "archive contains a link or special entry, which ingest does not accept",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_ip_classes() {
        for public in ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:4700:4700::1111"] {
            assert!(is_public_ip(public.parse().expect("ip")), "{public} is public");
        }
        for non_public in [
            "127.0.0.1",       // loopback
            "10.0.0.5",        // RFC 1918
            "172.16.0.9",      // RFC 1918
            "192.168.1.1",     // RFC 1918
            "169.254.1.1",     // link local
            "100.64.0.1",      // CGNAT shared space
            "0.0.0.0",         // unspecified
            "0.1.2.3",         // "this network"
            "224.0.0.1",       // multicast
            "192.0.2.1",       // documentation TEST-NET-1
            "198.18.0.1",      // benchmarking
            "240.0.0.1",       // reserved
            "255.255.255.255", // broadcast
            "::1",             // v6 loopback
            "::",              // v6 unspecified
            "fe80::1",         // v6 link local
            "fc00::1",         // v6 unique local
            "fd12:3456::1",    // v6 unique local
            "ff02::1",         // v6 multicast
            "2001:db8::1",     // v6 documentation
            "::ffff:127.0.0.1", // v4-mapped loopback
            "::ffff:10.0.0.1", // v4-mapped private
        ] {
            assert!(
                !is_public_ip(non_public.parse().expect("ip")),
                "{non_public} is not public"
            );
        }
        assert!(
            is_public_ip("::ffff:8.8.8.8".parse().expect("ip")),
            "a v4-mapped public address stays public"
        );
    }

    #[test]
    fn parse_rejects_non_https_and_credentialed_urls() {
        for (raw, why) in [
            ("http://github.com/a/b", "plain http"),
            ("ssh://git@github.com/a/b", "ssh scheme"),
            ("git://github.com/a/b", "git scheme"),
            ("file:///srv/repo", "file scheme"),
            ("ext::sh -c id", "ext transport"),
            ("https://user:secret@github.com/a/b", "embedded credentials"),
            ("https://user@github.com/a/b", "embedded user"),
            ("https://github.com:8443/a/b", "non-443 port"),
            ("not a url", "unparseable"),
            ("https://", "no host"),
        ] {
            let err = parse_repo_url(raw).expect_err(why);
            assert_eq!(err.code(), "invalid_repo_url", "{raw}: {why}");
        }
    }

    #[test]
    fn parse_accepts_plain_https_urls() {
        for raw in [
            "https://github.com/nrupala/codetopo",
            "https://github.com/nrupala/codetopo.git",
            "https://github.com:443/nrupala/codetopo",
            "https://git.example.com:443/group/repo.git",
        ] {
            assert!(parse_repo_url(raw).is_ok(), "{raw}");
        }
    }

    /// IP-literal hosts need no DNS, so the full validation path —
    /// resolution included — is exercisable offline.
    #[tokio::test]
    async fn validate_refuses_non_public_literal_hosts() {
        for raw in [
            "https://127.0.0.1/repo.git",
            "https://10.1.2.3/repo.git",
            "https://[::1]/repo.git",
            "https://localhost/repo.git",
            "https://localhost./repo.git",
        ] {
            let err = validate_repo_url(raw)
                .await
                .expect_err("non-public host refused");
            assert_eq!(err.code(), "invalid_repo_url", "{raw}");
        }
    }

    #[test]
    fn package_name_comes_from_the_url_tail() {
        let cases = [
            ("https://github.com/nrupala/codetopo", "codetopo"),
            ("https://github.com/nrupala/codetopo.git", "codetopo"),
            ("https://github.com/nrupala/codetopo/", "codetopo"),
            ("https://example.com/", "repo"),
            ("https://example.com/.git", "repo"),
        ];
        for (raw, expected) in cases {
            let url = parse_repo_url(raw).expect("url parses");
            assert_eq!(package_name_for_url(&url), expected, "{raw}");
        }
    }

    #[test]
    fn archive_kind_reads_type_then_magic() {
        assert_eq!(
            archive_kind(Some("application/zip"), b"anything"),
            Some(ArchiveKind::Zip)
        );
        assert_eq!(
            archive_kind(Some("application/gzip"), b"anything"),
            Some(ArchiveKind::TarGz)
        );
        assert_eq!(
            archive_kind(Some("application/octet-stream"), b"PK\x03\x04rest"),
            Some(ArchiveKind::Zip)
        );
        assert_eq!(
            archive_kind(None, b"\x1f\x8brest"),
            Some(ArchiveKind::TarGz)
        );
        assert_eq!(archive_kind(None, b"{\"json\":true}"), None);
        assert_eq!(archive_kind(Some("text/plain"), b"hello"), None);
    }

    // -- archive fixtures, built in memory -------------------------------

    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, contents) in entries {
            writer.start_file(*name, options).expect("start file");
            writer.write_all(contents).expect("write file");
        }
        writer.finish().expect("finish zip").into_inner()
    }

    fn tar_gz_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut builder = tar::Builder::new(&mut encoder);
            for (name, contents) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(contents.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, name, *contents)
                    .expect("append entry");
            }
            builder.finish().expect("finish tar");
        }
        encoder.finish().expect("finish gzip")
    }

    fn extract(kind: ArchiveKind, bytes: &[u8]) -> (tempfile::TempDir, Result<PathBuf, ApiError>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let workdir = dir.path().join("work");
        let result = extract_into(kind, bytes, &workdir);
        (dir, result)
    }

    #[test]
    fn zip_extracts_and_unwraps_a_single_top_directory() {
        let bytes = zip_bytes(&[
            ("repo-main/src/lib.rs", b"pub fn f() {}"),
            ("repo-main/README.md", b"# repo"),
        ]);
        let (_dir, result) = extract(ArchiveKind::Zip, &bytes);
        let root = result.expect("extract");
        assert_eq!(root.file_name().expect("name"), "repo-main");
        assert!(root.join("src/lib.rs").is_file());
        assert!(root.join("README.md").is_file());
    }

    #[test]
    fn tar_gz_extracts_files_at_the_root() {
        let bytes = tar_gz_bytes(&[("src/main.rs", b"fn main() {}"), ("Cargo.toml", b"[package]")]);
        let (dir, result) = extract(ArchiveKind::TarGz, &bytes);
        let root = result.expect("extract");
        assert_eq!(root, dir.path().join("work"), "no unwrap without a single top dir");
        assert!(root.join("src/main.rs").is_file());
    }

    #[test]
    fn zip_traversal_is_refused_and_leaves_nothing() {
        let bytes = zip_bytes(&[("../evil.rs", b"pub fn evil() {}")]);
        let (dir, result) = extract(ArchiveKind::Zip, &bytes);
        let err = result.expect_err("traversal refused");
        assert_eq!(err.code(), "invalid_body");
        assert!(
            !dir.path().join("work").exists(),
            "a failed ingest removes its workdir"
        );
        assert!(
            !dir.path().join("evil.rs").exists(),
            "nothing was written outside the workdir"
        );
    }

    /// A one-entry tarball with the entry name written byte-for-byte,
    /// for names the `tar` crate's own *builder* refuses to write —
    /// which is exactly the class a hostile archive arrives in, built
    /// by other tools. One ustar header block, the data, padding, and
    /// the two zero blocks that end a tar stream, gzipped.
    fn raw_tar_gz_bytes(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", contents.len()).as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        for byte in &mut header[148..156] {
            *byte = b' ';
        }
        let checksum: u32 = header.iter().map(|byte| *byte as u32).sum();
        header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());

        let mut tar = Vec::new();
        tar.extend_from_slice(&header);
        tar.extend_from_slice(contents);
        tar.resize(tar.len() + (512 - contents.len() % 512) % 512, 0);
        tar.resize(tar.len() + 1024, 0);

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&tar).expect("gzip write");
        encoder.finish().expect("finish gzip")
    }

    #[test]
    fn tar_traversal_is_refused() {
        let bytes = raw_tar_gz_bytes("ok/../../evil.rs", b"x");
        let (dir, result) = extract(ArchiveKind::TarGz, &bytes);
        assert_eq!(result.expect_err("traversal refused").code(), "invalid_body");
        assert!(
            !dir.path().join("evil.rs").exists(),
            "nothing was written outside the workdir"
        );
    }

    #[test]
    fn tar_symlink_is_refused() {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut builder = tar::Builder::new(&mut encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_cksum();
            builder
                .append_link(&mut header, "link", "/etc/passwd")
                .expect("append link");
            builder.finish().expect("finish tar");
        }
        let bytes = encoder.finish().expect("finish gzip");
        let (_dir, result) = extract(ArchiveKind::TarGz, &bytes);
        assert_eq!(result.expect_err("link refused").code(), "invalid_body");
    }

    #[test]
    fn extraction_counts_actual_bytes_against_the_cap() {
        // Two small entries, a one-byte budget: the second file's bytes
        // cross it even though each entry alone looks harmless.
        let bytes = zip_bytes(&[("a.rs", b"aaaa"), ("b.rs", b"bbbb")]);
        let dir = tempfile::tempdir().expect("temp dir");
        let dest = dir.path().join("work");
        std::fs::create_dir_all(&dest).expect("workdir");
        let err = extract_zip(&bytes, &dest, MAX_ARCHIVE_ENTRIES, 5).expect_err("cap enforced");
        assert_eq!(err.code(), "invalid_body");
    }

    #[test]
    fn entry_count_cap_is_enforced() {
        let bytes = zip_bytes(&[("a.rs", b"a"), ("b.rs", b"b"), ("c.rs", b"c")]);
        let dir = tempfile::tempdir().expect("temp dir");
        let dest = dir.path().join("work");
        std::fs::create_dir_all(&dest).expect("workdir");
        let err = extract_zip(&bytes, &dest, 2, MAX_TREE_BYTES).expect_err("cap enforced");
        assert_eq!(err.code(), "invalid_body");
    }

    #[test]
    fn garbage_is_not_an_archive() {
        let (_dir, result) = extract(ArchiveKind::Zip, b"definitely not a zip");
        assert_eq!(result.expect_err("refused").code(), "invalid_body");
        let (_dir, result) = extract(ArchiveKind::TarGz, b"definitely not a tarball");
        assert_eq!(result.expect_err("refused").code(), "invalid_body");
    }

    // -- clone plumbing, against a local git daemon ----------------------

    /// A directory holding one committed repository, served by
    /// `git daemon` on an ephemeral loopback port. The daemon transport
    /// is not HTTPS and its host is not public — exactly why the tests
    /// drive `run_git_clone` (the plumbing) rather than the validating
    /// entry point: validation is covered above, the subprocess here.
    struct DaemonRepo {
        _dir: tempfile::TempDir,
        url: String,
        child: std::process::Child,
    }

    impl Drop for DaemonRepo {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn daemon_repo() -> DaemonRepo {
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("served");
        std::fs::create_dir_all(source.join("src")).expect("source dirs");
        std::fs::write(source.join("src/lib.rs"), "pub fn served() {}\n").expect("write source");
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(&source)
                .env("GIT_AUTHOR_NAME", "ingest test")
                .env("GIT_AUTHOR_EMAIL", "ingest@example.com")
                .env("GIT_COMMITTER_NAME", "ingest test")
                .env("GIT_COMMITTER_EMAIL", "ingest@example.com")
                .status()
                .expect("git runs");
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "--quiet", "."]);
        run(&["add", "."]);
        run(&["commit", "--quiet", "-m", "fixture"]);

        // An ephemeral port: bind, read the number, release, hand it to
        // the daemon. A small race, accepted in tests (production git
        // never binds ports).
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("addr")
            .port();
        let child = Command::new("git")
            .args([
                "daemon",
                "--export-all",
                "--reuseaddr",
                &format!("--base-path={}", dir.path().display()),
                &format!("--port={port}"),
                "--listen=127.0.0.1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("git daemon spawns");
        let url = format!("git://127.0.0.1:{port}/served");
        // Wait until the daemon answers: clone attempts double as the
        // readiness probe, against a throwaway destination.
        let probe = dir.path().join("probe");
        let mut ready = false;
        for _ in 0..50 {
            if run_git_clone(&url, &probe, Duration::from_secs(5)).is_ok() {
                let _ = std::fs::remove_dir_all(&probe);
                ready = true;
                break;
            }
            let _ = std::fs::remove_dir_all(&probe);
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(ready, "git daemon never answered a probe clone");
        DaemonRepo {
            _dir: dir,
            url,
            child,
        }
    }

    #[test]
    fn clone_plumbing_lands_the_tree() {
        let daemon = daemon_repo();
        let dir = tempfile::tempdir().expect("temp dir");
        let dest = dir.path().join("clone");
        run_git_clone(&daemon.url, &dest, Duration::from_secs(30)).expect("clone succeeds");
        assert!(dest.join("src/lib.rs").is_file(), "the tree landed");
        let bytes = tree_bytes(&dest, MAX_TREE_BYTES);
        assert!(bytes > 0 && bytes < MAX_TREE_BYTES);
    }

    #[test]
    fn clone_of_a_missing_repo_is_an_error_not_a_hang() {
        let daemon = daemon_repo();
        let dir = tempfile::tempdir().expect("temp dir");
        let dest = dir.path().join("clone");
        let err = run_git_clone(
            &daemon.url.replace("/served", "/absent"),
            &dest,
            Duration::from_secs(10),
        )
        .expect_err("missing repo refused");
        assert_eq!(err.code(), "invalid_repo_url");
    }

    #[test]
    fn clone_timeout_kills_the_child() {
        // A listener that accepts and never speaks: git waits for the
        // daemon greeting forever, so only the timeout can end it.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let held = std::thread::spawn(move || {
            let _held_streams: Vec<std::net::TcpStream> = listener
                .incoming()
                .filter_map(Result::ok)
                .take(1)
                .collect();
            std::thread::sleep(Duration::from_secs(10));
        });
        let dir = tempfile::tempdir().expect("temp dir");
        let dest = dir.path().join("clone");
        let started = Instant::now();
        let err = run_git_clone(
            &format!("git://127.0.0.1:{port}/served"),
            &dest,
            Duration::from_secs(1),
        )
        .expect_err("a silent peer times out");
        assert_eq!(err.code(), "invalid_repo_url");
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "the clone was killed, not waited out: {:?}",
            started.elapsed()
        );
        drop(held);
    }
}
