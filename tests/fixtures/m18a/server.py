#!/usr/bin/env python3
"""Deterministic remote-web acceptance fixture for the Nagi guest."""

from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread
import argparse
import ssl

FIXTURE_ROOT = Path(__file__).resolve().parent
DOWNLOAD_BODY = b"nagi-m18a-download"


class FixtureHandler(SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/download":
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Disposition", 'attachment; filename="m18a.bin"')
            self.send_header("Content-Length", str(len(DOWNLOAD_BODY)))
            self.end_headers()
            self.wfile.write(DOWNLOAD_BODY)
            return
        if self.path == "/redirect":
            self.send_response(302)
            if self.headers.get("Host", "").startswith("10.0.2.2:"):
                location = "https://10.0.2.2:18443/index.html"
            else:
                location = "/index.html"
            self.send_header("Location", location)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        super().do_GET()

    def do_POST(self):
        if self.path != "/upload":
            self.send_error(404)
            return
        content_length = int(self.headers.get("Content-Length", "0"))
        uploaded = self.rfile.read(content_length)
        response = b"upload received: " + uploaded
        self.send_response(200)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)

    def log_message(self, _format, *_args):
        pass


def make_handler():
    return lambda *args, **kwargs: FixtureHandler(
        *args, directory=str(FIXTURE_ROOT), **kwargs
    )


def serve(
    bind="0.0.0.0",
    port=18081,
    https_port=18443,
    tls_cert=FIXTURE_ROOT / "tls" / "server.pem",
    tls_key=FIXTURE_ROOT / "tls" / "server-key.pem",
):
    http_server = ThreadingHTTPServer((bind, port), make_handler())
    https_server = ThreadingHTTPServer((bind, https_port), make_handler())
    tls_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls_context.load_cert_chain(certfile=tls_cert, keyfile=tls_key)
    https_server.socket = tls_context.wrap_socket(https_server.socket, server_side=True)
    try:
        https_thread = Thread(target=https_server.serve_forever, daemon=True)
        https_thread.start()
        http_server.serve_forever()
    finally:
        http_server.shutdown()
        https_server.shutdown()
        http_server.server_close()
        https_server.server_close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--bind", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=18081)
    parser.add_argument("--https-port", type=int, default=18443)
    parser.add_argument("--tls-cert", type=Path, default=FIXTURE_ROOT / "tls" / "server.pem")
    parser.add_argument("--tls-key", type=Path, default=FIXTURE_ROOT / "tls" / "server-key.pem")
    arguments = parser.parse_args()
    serve(
        arguments.bind,
        arguments.port,
        arguments.https_port,
        arguments.tls_cert,
        arguments.tls_key,
    )
