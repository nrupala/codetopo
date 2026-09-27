// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! stdio transport for the MCP server.
//!
//! The hard part of MCP over stdio is not JSON-RPC — it is that stdout is
//! *also* the progress channel of the code underneath. [`codetopo_cli`]
//! reports indexing progress with `println!`, so a single stray line between
//! two frames makes the client's parser read `files_indexed: 42` as a response
//! and then wait forever for a reply to the request that was never answered.
//!
//! The fix is to stop sharing the descriptor. [`seal_stdout`] duplicates the
//! client's stdout to a private descriptor, points fd 1 at `/dev/null`, and
//! hands the real descriptor back. Responses go to the returned writer; a
//! callee's `println!` goes to the bit bucket. The client sees JSON and only
//! JSON, and the callee stays free of transport concerns — it has no idea it
//! is running under MCP.
//!
//! Sealing is process-global, so it is deliberately *not* exercised by this
//! module's unit tests: those share a process with every other test in the
//! crate, and silencing fd 1 would swallow their output too. The real binary
//! seals for real in `tests/mcp_stdio.rs`.

use std::fs::File;
use std::io::{self, BufRead, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};

/// The descriptor stdout is moved aside from, and the one rebound to the sink.
const STDOUT_FD: RawFd = 1;

/// A server whose response channel is already separated from `println!`.
///
/// The writer is returned rather than stored so the caller keeps the single
/// obvious path to the protocol stream: there is no second way to write a
/// response, and so no way to write one to fd 1 by accident.
pub struct Sealed {
    writer: File,
}

impl Write for Sealed {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// Move stdout aside so tool output can never corrupt the protocol stream.
///
/// On success the returned writer owns the client's real stdout and fd 1 now
/// points at `/dev/null`. When the rebinding cannot be done the original
/// stdout is left in place and the returned writer is a harmless duplicate of
/// it: a server that risks a stray progress line is recoverable, one that
/// refuses to start is not.
pub fn seal_stdout() -> io::Result<Sealed> {
    // SAFETY: `dup` takes only a descriptor, so it has no memory-safety
    // preconditions, and -1 on failure is checked by the caller.
    let saved = unsafe { libc::dup(STDOUT_FD) };
    if saved < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `saved` is a live, uniquely owned descriptor that nothing else
    // will close; handing it to `File` here gives it a single owner for the
    // rest of its life.
    let sealed = Sealed {
        writer: unsafe { File::from_raw_fd(saved) },
    };
    let sink = match File::options().read(true).write(true).open("/dev/null") {
        Ok(sink) => sink,
        Err(_) => return Ok(sealed),
    };
    // SAFETY: both descriptors are open and owned by this function, and
    // `dup2` only rebinds fd 1 — it closes neither argument, so no ownership
    // is duplicated or lost. A -1 return is handled on the next line.
    let moved = unsafe { libc::dup2(sink.as_raw_fd(), STDOUT_FD) };
    // The sink is redundant once fd 1 is the sink; dropping its descriptor
    // keeps the descriptor count flat over a long-lived server.
    drop(sink);
    if moved < 0 {
        return Ok(sealed);
    }
    Ok(sealed)
}

/// Read requests from `reader`, dispatch them, and answer on `writer`.
///
/// Returns at EOF: an MCP client closes stdin when it is done, and the server
/// has no reason to outlive its client. Blank lines are skipped, and a
/// response is written only when `dispatch` produces one — a notification has
/// no reply, and replying to it would be a protocol error.
///
/// Every response is flushed before the next request is read. A client that
/// pipes its input blocks until it sees output, so a response left in the
/// userspace buffer would deadlock any client that sends two requests before
/// reading the first answer.
pub fn serve<R: BufRead, W: Write, D>(
    mut reader: R,
    writer: &mut W,
    mut dispatch: D,
) -> io::Result<()>
where
    D: FnMut(&str) -> Option<String>,
{
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let request = line.trim();
        if request.is_empty() {
            continue;
        }
        if let Some(response) = dispatch(request) {
            // A closed pipe means the client exited without a goodbye, which
            // is a normal end of session. Surfacing it would turn every clean
            // shutdown into a failed run, so the write is best-effort and the
            // next read reports the EOF.
            let _ = writer.write_all(response.as_bytes());
            let _ = writer.write_all(b"\n");
            let _ = writer.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn echo(request: &str) -> Option<String> {
        Some(format!("{request} answered"))
    }

    #[test]
    fn one_response_per_request() {
        let mut out = Vec::new();
        serve(Cursor::new(b"{\"id\":1}\n{\"id\":2}\n"), &mut out, echo)
            .expect("in-memory reads cannot fail");
        assert_eq!(out, b"{\"id\":1} answered\n{\"id\":2} answered\n");
    }

    #[test]
    fn stops_at_end_of_input() {
        let calls = std::cell::Cell::new(0usize);
        let mut out = Vec::new();
        serve(Cursor::new(b""), &mut out, |_| {
            calls.set(calls.get() + 1);
            None
        })
        .expect("an empty stream is not an error");
        assert_eq!(calls.get(), 0, "EOF must not dispatch a request");
        assert!(out.is_empty(), "EOF must not write a response");
    }

    #[test]
    fn blank_lines_are_skipped() {
        let calls = std::cell::Cell::new(0usize);
        let mut out = Vec::new();
        serve(Cursor::new(b"\n   \n\n"), &mut out, |_| {
            calls.set(calls.get() + 1);
            Some("{}".to_string())
        })
        .expect("blank lines are not io errors");
        assert_eq!(calls.get(), 0);
        assert!(out.is_empty());
    }

    #[test]
    fn a_notification_is_never_answered() {
        let mut out = Vec::new();
        serve(
            Cursor::new(b"{\"method\":\"initialized\"}\n"),
            &mut out,
            |_| None,
        )
        .expect("in-memory reads cannot fail");
        assert!(out.is_empty(), "a notification must not be answered");
    }

    #[test]
    fn a_request_without_a_trailing_newline_is_still_served() {
        // A client that writes the last frame and closes stdin must not be
        // left waiting for a reply that was sitting in the buffer.
        let mut out = Vec::new();
        serve(Cursor::new(b"{\"id\":1}"), &mut out, echo)
            .expect("a partial line still ends at EOF");
        assert_eq!(out, b"{\"id\":1} answered\n");
    }

    #[test]
    fn a_response_lands_in_the_sealed_writer() {
        // The whole point of the seal: a response goes to the descriptor the
        // client is reading and nowhere else. Here that descriptor is a file
        // rather than the process stdout, so the assertion is exact.
        let path = std::env::temp_dir().join(format!("codetopo-stdio-{}", std::process::id()));
        let mut sealed = Sealed {
            writer: File::create(&path).expect("temp path is writable"),
        };
        serve(Cursor::new(b"{\"id\":1}\n"), &mut sealed, echo)
            .expect("in-memory reads cannot fail");
        drop(sealed);
        let written = std::fs::read_to_string(&path).expect("the file was just written");
        let _ = std::fs::remove_file(&path);
        assert_eq!(written, "{\"id\":1} answered\n");
    }
}
