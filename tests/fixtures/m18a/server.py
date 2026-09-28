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
        if self.path == "/tls-probe":
            body = b"untrusted TLS fixture reached"
            self.send_response(200)
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
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
    untrusted_https_port=18444,
    tls_cert=FIXTURE_ROOT / "tls" / "server.pem",
    tls_key=FIXTURE_ROOT / "tls" / "server-key.pem",
    untrusted_tls_cert=FIXTURE_ROOT / "tls" / "untrusted-server.pem",
    untrusted_tls_key=FIXTURE_ROOT / "tls" / "untrusted-server-key.pem",
):
    http_server = ThreadingHTTPServer((bind, port), make_handler())
    https_server = ThreadingHTTPServer((bind, https_port), make_handler())
    tls_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls_context.load_cert_chain(certfile=tls_cert, keyfile=tls_key)
    https_server.socket = tls_context.wrap_socket(https_server.socket, server_side=True)
    untrusted_https_server = ThreadingHTTPServer((bind, untrusted_https_port), make_handler())
    untrusted_tls_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    untrusted_tls_context.load_cert_chain(
        certfile=untrusted_tls_cert, keyfile=untrusted_tls_key
    )
    untrusted_https_server.socket = untrusted_tls_context.wrap_socket(
        untrusted_https_server.socket, server_side=True
    )
    try:
        https_thread = Thread(target=https_server.serve_forever, daemon=True)
        untrusted_https_thread = Thread(
            target=untrusted_https_server.serve_forever, daemon=True
        )
        https_thread.start()
        untrusted_https_thread.start()
        http_server.serve_forever()
    finally:
        http_server.shutdown()
        https_server.shutdown()
        untrusted_https_server.shutdown()
        http_server.server_close()
        https_server.server_close()
        untrusted_https_server.server_close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--bind", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=18081)
    parser.add_argument("--https-port", type=int, default=18443)
    parser.add_argument("--untrusted-https-port", type=int, default=18444)
    parser.add_argument("--tls-cert", type=Path, default=FIXTURE_ROOT / "tls" / "server.pem")
    parser.add_argument("--tls-key", type=Path, default=FIXTURE_ROOT / "tls" / "server-key.pem")
    parser.add_argument(
        "--untrusted-tls-cert",
        type=Path,
        default=FIXTURE_ROOT / "tls" / "untrusted-server.pem",
    )
    parser.add_argument(
        "--untrusted-tls-key",
        type=Path,
        default=FIXTURE_ROOT / "tls" / "untrusted-server-key.pem",
    )
    arguments = parser.parse_args()
    serve(
        arguments.bind,
        arguments.port,
        arguments.https_port,
        arguments.untrusted_https_port,
        arguments.tls_cert,
        arguments.tls_key,
        arguments.untrusted_tls_cert,
        arguments.untrusted_tls_key,
    )
