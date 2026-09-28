"""Deterministic loopback HTTP/HTTPS fixtures for M18 browser acceptance."""

from __future__ import annotations

from hashlib import sha256
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import re
import ssl
from threading import Thread
import time
from urllib.parse import urlsplit


FIXTURE_ROOT = Path(__file__).resolve().parent / "fixtures"
DOWNLOAD_BODY = b"Nagi M18-C deterministic download fixture\n"
MAX_UPLOAD_BYTES = 64 * 1024
_PAGE_BODIES = {
    "/pages/one": (
        "M18-C Fixture One",
        b"<!doctype html><meta charset=utf-8><title>M18-C Fixture One</title>"
        b"<p>Deterministic fixture page one.</p>",
    ),
    "/pages/final": (
        "M18-C Fixture Final",
        b"<!doctype html><meta charset=utf-8><title>M18-C Fixture Final</title>"
        b"<p>Redirect chain completed.</p>",
    ),
    "/pages/secure": (
        "M18-C Fixture Secure",
        b"<!doctype html><meta charset=utf-8><title>M18-C Fixture Secure</title>"
        b"<p>Locally trusted HTTPS fixture.</p>",
    ),
}


class _FixtureHTTPServer(ThreadingHTTPServer):
    allow_reuse_address = True
    daemon_threads = True
    block_on_close = False

    def handle_error(self, request, client_address):
        # Client timeouts and intentional malformed-response cases may close
        # the socket before the handler writes its complete response.
        return


class _FixtureHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, _format, *_args):
        return

    def do_GET(self):
        path = urlsplit(self.path).path
        if path in _PAGE_BODIES:
            _title, body = _PAGE_BODIES[path]
            self._send(200, body, "text/html; charset=utf-8")
            return

        redirects = {
            "/redirect/one": (302, "/redirect/two"),
            "/redirect/two": (307, "/pages/final"),
            "/redirect/loop-a": (302, "/redirect/loop-b"),
            "/redirect/loop-b": (302, "/redirect/loop-a"),
        }
        if path == "/redirect/https":
            location = (
                f"https://{self.server.fixture_advertised_host}:"
                f"{self.server.fixture_https_port}/pages/secure"
            )
            self._send(302, b"", "text/plain; charset=utf-8", {"Location": location})
            return
        if path in redirects:
            status, location = redirects[path]
            self._send(status, b"", "text/plain; charset=utf-8", {"Location": location})
            return
        if path.startswith("/status/"):
            try:
                status = int(path.rsplit("/", 1)[1])
            except ValueError:
                self._send(400, b"invalid status fixture\n", "text/plain; charset=utf-8")
                return
            if status not in (404, 429, 500, 503):
                self._send(400, b"unsupported status fixture\n", "text/plain; charset=utf-8")
                return
            self._send(status, f"fixture status {status}\n".encode(), "text/plain; charset=utf-8")
            return
        if path == "/timeout":
            time.sleep(0.3)
            self._send(200, b"late fixture response\n", "text/plain; charset=utf-8")
            return
        if path == "/malformed":
            self.close_connection = True
            try:
                self.wfile.write(
                    b"HTTP/1.1 not-a-status\r\nConnection: close\r\n\r\n"
                    b"malformed fixture\n"
                )
            except OSError:
                pass
            return
        if path == "/download":
            self._send(
                200,
                DOWNLOAD_BODY,
                "application/octet-stream",
                {"Content-Disposition": 'attachment; filename="fixture-download.txt"'},
            )
            return
        self._send(404, b"fixture route not found\n", "text/plain; charset=utf-8")

    def do_POST(self):
        if urlsplit(self.path).path != "/upload":
            self._send(404, b"fixture route not found\n", "text/plain; charset=utf-8")
            return
        content_length = self.headers.get("Content-Length")
        if content_length is None or re.fullmatch(r"[0-9]+", content_length) is None:
            self._send(400, b"invalid Content-Length\n", "text/plain; charset=utf-8")
            return
        length = int(content_length)
        if length > MAX_UPLOAD_BYTES:
            self._send(413, b"upload fixture limit exceeded\n", "text/plain; charset=utf-8")
            return
        payload = self.rfile.read(length)
        if len(payload) != length:
            self._send(400, b"truncated upload body\n", "text/plain; charset=utf-8")
            return
        response = json.dumps(
            {"bytes_received": length, "sha256": sha256(payload).hexdigest()},
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        self._send(200, response, "application/json; charset=utf-8")

    def _send(self, status, body, content_type, headers=None):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        self.close_connection = True
        try:
            self.wfile.write(body)
        except OSError:
            pass


class FixtureServer:
    """Owns local fixture servers and closes sockets/threads on exit."""

    download_body = DOWNLOAD_BODY
    max_upload_bytes = MAX_UPLOAD_BYTES

    def __init__(self, bind_host="127.0.0.1", advertised_host=None):
        if bind_host == "0.0.0.0" and not advertised_host:
            raise ValueError("a guest-reachable bind requires an explicit advertised_host")
        self.certificate = FIXTURE_ROOT / "tls" / "server-cert.pem"
        self._private_key = FIXTURE_ROOT / "tls" / "server-key.pem"
        self.bind_host = bind_host
        self.advertised_host = advertised_host or bind_host
        self._http = None
        self._https = None
        self._http_thread = None
        self._https_thread = None

    @property
    def http_port(self):
        self._require_started()
        return self._http.server_address[1]

    @property
    def https_port(self):
        self._require_started()
        return self._https.server_address[1]

    @property
    def http_base(self):
        return f"http://{self.advertised_host}:{self.http_port}"

    @property
    def https_base(self):
        return f"https://{self.advertised_host}:{self.https_port}"

    def start(self):
        if self._http is not None or self._https is not None:
            raise RuntimeError("fixture servers have already been started")
        self._http = _FixtureHTTPServer((self.bind_host, 0), _FixtureHandler)
        try:
            self._https = _FixtureHTTPServer((self.bind_host, 0), _FixtureHandler)
            self._http.fixture_https_port = self._https.server_address[1]
            self._https.fixture_https_port = self._https.server_address[1]
            self._http.fixture_advertised_host = self.advertised_host
            self._https.fixture_advertised_host = self.advertised_host
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.load_cert_chain(str(self.certificate), str(self._private_key))
            self._https.socket = context.wrap_socket(self._https.socket, server_side=True)
            self._http_thread = Thread(target=self._http.serve_forever, daemon=True)
            self._https_thread = Thread(target=self._https.serve_forever, daemon=True)
            self._http_thread.start()
            self._https_thread.start()
        except BaseException:
            self.close()
            raise
        return self

    def close(self):
        servers = (self._http, self._https)
        threads = (self._http_thread, self._https_thread)
        for server, thread in zip(servers, threads):
            if server is not None:
                if thread is not None and thread.is_alive():
                    server.shutdown()
                server.server_close()
        for thread in threads:
            if thread is not None:
                thread.join(timeout=1)
        self._http = None
        self._https = None
        self._http_thread = None
        self._https_thread = None

    def __enter__(self):
        return self.start()

    def __exit__(self, _exception_type, _exception, _traceback):
        self.close()

    def _require_started(self):
        if self._http is None or self._https is None:
            raise RuntimeError("fixture servers are not running")
