#!/usr/bin/env python3
"""Deterministic remote-web acceptance fixture for the Nagi guest."""

from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import argparse

FIXTURE_ROOT = Path(__file__).resolve().parent


class FixtureHandler(SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", "/index.html")
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


def serve(bind="0.0.0.0", port=18081):
    ThreadingHTTPServer((bind, port), make_handler()).serve_forever()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--bind", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=18081)
    arguments = parser.parse_args()
    serve(arguments.bind, arguments.port)
