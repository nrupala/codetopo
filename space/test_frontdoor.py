#!/usr/bin/env python3
# Copyright (C) 2026 Nrupal Akolkar
# SPDX-License-Identifier: AGPL-3.0-or-later
# Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.
"""Front door behaviour pins (release gate F-1 / F-8).

Runs the real frontdoor.py as a subprocess against a stub upstream
and pins the two behaviours the fleet release gate called out:

  * F-8: the landing page at / is served regardless of the request's
    Accept header — browsers, health checks, link checkers, and plain
    HTTP clients all see the demo as up.
  * F-1: an upstream 5xx response keeps its status but its body is
    replaced with a generic one — the upstream body is internal
    failure detail (host paths, store error text) and the public demo
    must never publish it. Non-5xx bodies pass through untouched.

Plus the standing surface pins: unknown paths 404, non-GET 405, and
the development log served byte-for-byte at /log.

Standard library only, like the front door itself.
"""

import http.client
import json
import os
import socket
import subprocess
import sys
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SPACE_DIR = os.path.dirname(os.path.abspath(__file__))
LANDING = open(os.path.join(SPACE_DIR, "index.html"), "rb").read()
LOG = open(os.path.join(SPACE_DIR, "log.html"), "rb").read()

STATS_BODY = json.dumps({"nodes": 42, "edges": 87}).encode("utf-8")
BOOM_BODY = json.dumps({
    "error": "internal",
    "message": "cannot open key store /srv/private/keys.db: disk I/O error",
}).encode("utf-8")


class StubUpstream(BaseHTTPRequestHandler):
    """The demo API, in miniature: healthy stats, or a 500 whose body
    carries exactly the internals F-1 is about (selected by ?boom=1,
    which the front door forwards untouched)."""

    def log_message(self, *args):
        pass

    def do_GET(self):
        if "boom=1" in self.path:
            body, status = BOOM_BODY, 500
        else:
            body, status = STATS_BODY, 200
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def free_port():
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
    sock.close()
    return port


def request(port, method, path, headers=None):
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
    try:
        conn.request(method, path, headers=headers or {})
        resp = conn.getresponse()
        return resp.status, resp.getheader("Content-Type") or "", resp.read()
    finally:
        conn.close()


class FrontDoorTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.upstream = ThreadingHTTPServer(("127.0.0.1", 0), StubUpstream)
        upstream_port = cls.upstream.server_address[1]
        cls.upstream_thread = cls.upstream
        import threading
        threading.Thread(target=cls.upstream.serve_forever, daemon=True).start()

        cls.port = free_port()
        env = dict(os.environ)
        env.update({
            "CODETOPO_DEMO_KEY": "demo:test-secret",
            "CODETOPO_INDEX_ID": "idx-test",
            "CODETOPO_API_BASE": "http://127.0.0.1:%d" % upstream_port,
            "FRONTDOOR_PORT": str(cls.port),
            "FRONTDOOR_DIR": SPACE_DIR,
        })
        cls.proc = subprocess.Popen(
            [sys.executable, os.path.join(SPACE_DIR, "frontdoor.py")],
            env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.time() + 15
        while time.time() < deadline:
            try:
                status, _, _ = request(cls.port, "GET", "/")
                if status == 200:
                    return
            except OSError:
                pass
            time.sleep(0.1)
        raise RuntimeError("front door did not come up")

    @classmethod
    def tearDownClass(cls):
        cls.proc.terminate()
        cls.proc.wait(timeout=10)
        cls.upstream.shutdown()

    # -- F-8: the landing page does not care about Accept -------------

    def test_landing_served_regardless_of_accept(self):
        for headers in ({}, {"Accept": "*/*"}, {"Accept": "application/json"},
                        {"Accept": "text/html"}):
            status, ctype, body = request(self.port, "GET", "/", headers)
            self.assertEqual(status, 200, "headers %r" % (headers,))
            self.assertTrue(ctype.startswith("text/html"), ctype)
            self.assertEqual(body, LANDING)

    # -- F-1: upstream 5xx bodies never reach the visitor -------------

    def test_upstream_5xx_body_is_replaced_status_kept(self):
        status, ctype, body = request(self.port, "GET", "/v1/graphs/demo/stats?boom=1")
        self.assertEqual(status, 500, "the upstream status is kept")
        self.assertEqual(json.loads(body),
                         {"error": "internal", "message": "internal error"})
        self.assertTrue(ctype.startswith("application/json"), ctype)
        self.assertNotIn(b"/srv/private", body)
        self.assertNotIn(b"disk I/O", body)

    def test_upstream_success_passes_through(self):
        status, _, body = request(self.port, "GET", "/v1/graphs/demo/stats")
        self.assertEqual(status, 200)
        self.assertEqual(body, STATS_BODY)

    # -- Standing surface pins -----------------------------------------

    def test_log_page_served(self):
        for path in ("/log", "/log/"):
            status, ctype, body = request(self.port, "GET", path)
            self.assertEqual(status, 200, path)
            self.assertTrue(ctype.startswith("text/html"), ctype)
            self.assertEqual(body, LOG)

    def test_unknown_path_404_and_post_405(self):
        status, _, _ = request(self.port, "GET", "/v1/graphs/demo/snapshot")
        self.assertEqual(status, 404, "snapshot stays outside the demo surface")
        status, _, _ = request(self.port, "GET", "/nope")
        self.assertEqual(status, 404)
        status, _, _ = request(self.port, "POST", "/")
        self.assertEqual(status, 405)


if __name__ == "__main__":
    unittest.main()
