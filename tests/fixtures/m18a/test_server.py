#!/usr/bin/env python3
"""Host-side deterministic contract tests for the M18-A HTTP/TLS fixture."""

from http.server import ThreadingHTTPServer
import http.client
import socket
import ssl
from threading import Thread
from urllib.request import Request, urlopen
import unittest

from server import DOWNLOAD_BODY, FIXTURE_ROOT, FixtureHandler

TLS_ROOT = FIXTURE_ROOT / "tls"


class FixtureTLSServer(ThreadingHTTPServer):
    def __init__(self, cert_file=None, key_file=None):
        handler = lambda *args, **kwargs: FixtureHandler(
            *args, directory=str(FIXTURE_ROOT), **kwargs
        )
        super().__init__(("127.0.0.1", 0), handler)
        tls_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls_context.load_cert_chain(
            certfile=cert_file or TLS_ROOT / "server.pem",
            keyfile=key_file or TLS_ROOT / "server-key.pem",
        )
        self.socket = tls_context.wrap_socket(self.socket, server_side=True)


def tls_request(server, hostname, context, method="GET", path=None, body=None):
    raw_socket = socket.create_connection(("127.0.0.1", server.server_port), timeout=2)
    tls_socket = context.wrap_socket(raw_socket, server_hostname=hostname)
    path = path or ("/index.html" if method == "GET" else "/upload")
    headers = ["Host: m18a.test", "Connection: close"]
    if body is not None:
        headers.extend(
            [
                "Content-Type: application/octet-stream",
                f"Content-Length: {len(body)}",
            ]
        )
    request = f"{method} {path} HTTP/1.1\r\n" + "\r\n".join(headers) + "\r\n\r\n"
    tls_socket.sendall(request.encode("ascii") + (body or b""))
    response = http.client.HTTPResponse(tls_socket)
    response.begin()
    content = response.read()
    status = response.status
    headers = dict(response.getheaders())
    tls_socket.close()
    return status, headers, content


class FixtureServerTests(unittest.TestCase):
    def setUp(self):
        handler = lambda *args, **kwargs: FixtureHandler(
            *args, directory=str(FIXTURE_ROOT), **kwargs
        )
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self.thread = Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.base_url = f"http://127.0.0.1:{self.server.server_port}"

        self.https_server = FixtureTLSServer()
        self.https_thread = Thread(target=self.https_server.serve_forever, daemon=True)
        self.https_thread.start()
        self.untrusted_https_server = FixtureTLSServer(
            TLS_ROOT / "untrusted-server.pem", TLS_ROOT / "untrusted-server-key.pem"
        )
        self.untrusted_https_thread = Thread(
            target=self.untrusted_https_server.serve_forever, daemon=True
        )
        self.untrusted_https_thread.start()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        self.https_server.shutdown()
        self.https_server.server_close()
        self.https_thread.join(timeout=2)
        self.untrusted_https_server.shutdown()
        self.untrusted_https_server.server_close()
        self.untrusted_https_thread.join(timeout=2)

    @staticmethod
    def trusted_test_context():
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.verify_mode = ssl.CERT_REQUIRED
        context.check_hostname = True
        context.load_verify_locations(cafile=TLS_ROOT / "root.pem")
        return context

    def test_redirect_delivers_the_controlled_download(self):
        with urlopen(f"{self.base_url}/redirect", timeout=2) as response:
            content = response.read().decode("utf-8")
            self.assertEqual(response.url, f"{self.base_url}/index.html")
            self.assertEqual(response.status, 200)
        self.assertIn("Nagi M18A Controlled Remote Fixture", content)
        self.assertIn("Nagi M18A remote fixture", content)

    def test_guest_redirect_upgrades_http_to_https(self):
        connection = http.client.HTTPConnection(
            "127.0.0.1", self.server.server_port, timeout=2
        )
        connection.request("GET", "/redirect", headers={"Host": "10.0.2.2:18081"})
        response = connection.getresponse()
        self.assertEqual(response.status, 302)
        self.assertEqual(
            response.getheader("Location"), "https://10.0.2.2:18443/index.html"
        )
        connection.close()

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

    def test_download_endpoint_returns_attachment_bytes(self):
        with urlopen(f"{self.base_url}/download", timeout=2) as response:
            self.assertEqual(response.status, 200)
            self.assertEqual(
                response.headers.get("Content-Disposition"),
                'attachment; filename="m18a.bin"',
            )
            self.assertEqual(response.read(), DOWNLOAD_BODY)

    def test_https_fixture_accepts_test_ca_and_dns_hostname(self):
        status, _headers, content = tls_request(
            self.https_server, "m18a.test", self.trusted_test_context()
        )
        self.assertEqual(status, 200)
        self.assertIn(b"Nagi M18A Controlled Remote Fixture", content)

    def test_https_fixture_accepts_test_ca_and_ip_subject_alt_name(self):
        status, _headers, content = tls_request(
            self.https_server, "10.0.2.2", self.trusted_test_context()
        )
        self.assertEqual(status, 200)
        self.assertIn(b"Nagi M18A Controlled Remote Fixture", content)

    def test_https_fixture_rejects_untrusted_chain(self):
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.verify_mode = ssl.CERT_REQUIRED
        context.check_hostname = True
        with self.assertRaises(ssl.SSLCertVerificationError):
            tls_request(self.https_server, "m18a.test", context)

    def test_https_fixture_rejects_self_signed_server_even_with_matching_ip_san(self):
        with self.assertRaises(ssl.SSLCertVerificationError):
            tls_request(
                self.untrusted_https_server,
                "10.0.2.2",
                self.trusted_test_context(),
                path="/tls-probe",
            )

    def test_https_fixture_rejects_wrong_hostname(self):
        with self.assertRaises(ssl.SSLCertVerificationError):
            tls_request(
                self.https_server,
                "wrong.m18a.test",
                self.trusted_test_context(),
            )

    def test_https_upload_endpoint_echoes_the_received_body(self):
        status, _headers, content = tls_request(
            self.https_server,
            "m18a.test",
            self.trusted_test_context(),
            method="POST",
            body=b"nagi-m18a-upload",
        )
        self.assertEqual(status, 200)
        self.assertEqual(content, b"upload received: nagi-m18a-upload")

    def test_https_download_endpoint_returns_attachment_bytes(self):
        status, headers, content = tls_request(
            self.https_server,
            "m18a.test",
            self.trusted_test_context(),
            path="/download",
        )
        self.assertEqual(status, 200)
        self.assertEqual(headers["Content-Disposition"], 'attachment; filename="m18a.bin"')
        self.assertEqual(content, DOWNLOAD_BODY)


if __name__ == "__main__":
    unittest.main()
