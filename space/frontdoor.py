#!/usr/bin/env python3
# Copyright (C) 2026 2351485 ALBERTA INC.
# SPDX-License-Identifier: AGPL-3.0-or-later
# Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.
"""Read-only front door for the codetopo Hugging Face Space.

codetopo-server (the HTTP API) is Bearer-key authenticated on every route
and its one write operation — POST /v1/index — indexes an arbitrary
filesystem path. Neither property is safe to expose on a public port, so
the server stays on loopback inside the container and this front door is
the only public listener. It:

  * serves the static landing page (index.html) at /,
  * serves the public development log (log.html) at /log,
  * proxies an allowlist of GET query routes for the single demo index
    (the bundled copy of codetopo's own source, indexed at container
    start by entrypoint.sh) under the public alias /v1/graphs/demo/...,
  * injects the container-local demo key server-side — the key never
    appears in any response, page, or file outside the container,
  * answers 405 to every non-GET request and 404 to every other path, so
    indexing, snapshot export, and arbitrary paths are unreachable.

Standard library only; no dependencies to audit.
"""

import http.client
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

API_BASE = os.environ.get("CODETOPO_API_BASE", "http://127.0.0.1:8080")
DEMO_KEY = os.environ.get("CODETOPO_DEMO_KEY", "")
INDEX_ID = os.environ.get("CODETOPO_INDEX_ID", "")
PORT = int(os.environ.get("FRONTDOOR_PORT", "8787"))
STATIC_DIR = os.environ.get("FRONTDOOR_DIR", os.path.dirname(os.path.abspath(__file__)))

# The only upstream query tails a visitor may reach. `snapshot` is
# deliberately absent: it exports the whole graph in one response.
ALLOWED_TAILS = {
    "stats",
    "descendants",
    "ancestors",
    "blast-radius",
    "path",
    "verify",
    "neighborhood",
    "symbol",
    "search",
}


def json_response(handler, status, payload):
    body = json.dumps(payload).encode("utf-8")
    handler.send_response(status)
    handler.send_header("Content-Type", "application/json")
    handler.send_header("Content-Length", str(len(body)))
    handler.end_headers()
    handler.wfile.write(body)


class FrontDoor(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "codetopo-frontdoor/1"

    def log_message(self, fmt, *args):  # keep stdout clean; log to stderr
        sys.stderr.write("frontdoor: %s\n" % (fmt % args))

    # -- method gates: the demo is read-only ------------------------------
    def _method_not_allowed(self):
        json_response(self, 405, {"error": "method_not_allowed",
                                  "message": "this demo is read-only; GET only"})

    do_POST = _method_not_allowed
    do_PUT = _method_not_allowed
    do_PATCH = _method_not_allowed
    do_DELETE = _method_not_allowed
    do_HEAD = _method_not_allowed
    do_OPTIONS = _method_not_allowed

    # -- routing -----------------------------------------------------------
    def do_GET(self):
        parts = urlsplit(self.path)
        path = parts.path
        if path in ("/", "/index.html"):
            return self._serve_landing()
        if path in ("/log", "/log/"):
            return self._serve_static("log.html", "development log missing")
        if path == "/v1/schema":
            return self._proxy(parts)
        prefix = "/v1/graphs/demo/"
        if path.startswith(prefix):
            tail = path[len(prefix):]
            if tail in ALLOWED_TAILS:
                return self._proxy(parts)
            return json_response(self, 404, {"error": "not_found",
                                             "message": "not part of the read-only demo surface"})
        return json_response(self, 404, {"error": "not_found",
                                         "message": "unknown path"})

    def _serve_landing(self):
        return self._serve_static("index.html", "landing page missing")

    def _serve_static(self, filename, missing_message):
        try:
            with open(os.path.join(STATIC_DIR, filename), "rb") as fh:
                body = fh.read()
        except OSError:
            return json_response(self, 500, {"error": "internal",
                                             "message": missing_message})
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _proxy(self, parts):
        # Rewrite the public demo alias to the real index id, forward the
        # query string untouched, and inject the demo key server-side.
        upstream_path = parts.path.replace("/v1/graphs/demo/", "/v1/graphs/%s/" % INDEX_ID, 1)
        if parts.query:
            upstream_path += "?" + parts.query
        base = urlsplit(API_BASE)
        conn = http.client.HTTPConnection(base.hostname, base.port or 80, timeout=30)
        try:
            conn.request("GET", upstream_path,
                         headers={"Authorization": "Bearer " + DEMO_KEY})
            resp = conn.getresponse()
            body = resp.read()
            status = resp.status
            ctype = resp.getheader("Content-Type") or "application/json"
        except OSError:
            return json_response(self, 502, {"error": "upstream_unavailable",
                                             "message": "the demo API is not reachable yet; it indexes codetopo's source at container start — try again in a few seconds"})
        finally:
            conn.close()
        if status >= 500:
            # An upstream 5xx body is the server's internal failure
            # detail (host paths, store error text) — operator material
            # that a public demo must never publish (release gate F-1).
            # Keep the status, substitute a generic body. Non-5xx
            # bodies are the API's coded client errors and pass through.
            return json_response(self, status, {"error": "internal",
                                                "message": "internal error"})
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main():
    if not DEMO_KEY or not INDEX_ID:
        sys.stderr.write("frontdoor: CODETOPO_DEMO_KEY and CODETOPO_INDEX_ID must be set (entrypoint.sh does this)\n")
        return 1
    server = ThreadingHTTPServer(("0.0.0.0", PORT), FrontDoor)
    sys.stderr.write("frontdoor: listening on 0.0.0.0:%d, demo index %s\n" % (PORT, INDEX_ID))
    server.serve_forever()
    return 0


if __name__ == "__main__":
    sys.exit(main())
