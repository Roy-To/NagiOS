#!/usr/bin/env python3
"""Host-side deterministic contract tests for the M18-A HTTP fixture."""

from http.server import ThreadingHTTPServer
from threading import Thread
from urllib.request import Request, urlopen
import unittest

from server import FIXTURE_ROOT, FixtureHandler


class FixtureServerTests(unittest.TestCase):
    def setUp(self):
        handler = lambda *args, **kwargs: FixtureHandler(
            *args, directory=str(FIXTURE_ROOT), **kwargs
        )
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self.thread = Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.base_url = f"http://127.0.0.1:{self.server.server_port}"

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)

    def test_redirect_delivers_the_controlled_download(self):
        with urlopen(f"{self.base_url}/redirect", timeout=2) as response:
            content = response.read().decode("utf-8")
            self.assertEqual(response.url, f"{self.base_url}/index.html")
            self.assertEqual(response.status, 200)
        self.assertIn("Nagi M18A Controlled Remote Fixture", content)
        self.assertIn("Nagi M18A remote fixture", content)

    def test_upload_endpoint_returns_the_received_body(self):
        request = Request(
            f"{self.base_url}/upload",
            data=b"nagi-m18a-upload",
            headers={"Content-Type": "application/octet-stream"},
            method="POST",
        )
        with urlopen(request, timeout=2) as response:
            self.assertEqual(response.status, 200)
            self.assertEqual(response.read(), b"upload received: nagi-m18a-upload")


if __name__ == "__main__":
    unittest.main()
