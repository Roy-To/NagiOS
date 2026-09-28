import hashlib
import http.client
import json
from pathlib import Path
import socket
import ssl
import tempfile
import urllib.error
import urllib.request
import unittest

from browser_acceptance.fixture_server import FixtureServer
from browser_acceptance.feature_gates import blocked_feature_graph_evidence, evaluate_m17_feature_graph
from browser_acceptance.report import ReportError, build_report, load_manifest
from browser_acceptance.storage_fixtures import (
    FIXTURE_FORMAT,
    STATE_FIXTURES,
    materialize_state_fixture,
)


class BrowserFixtureServerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = FixtureServer()
        cls.server.start()
        cls.trusted_tls = ssl.create_default_context(cafile=str(cls.server.certificate))

    @classmethod
    def tearDownClass(cls):
        cls.server.close()

    def test_http_page_is_deterministic_and_has_a_title(self):
        with urllib.request.urlopen(f"{self.server.http_base}/pages/one", timeout=2) as response:
            body = response.read()

        self.assertEqual(response.status, 200)
        self.assertIn(b"<title>M18-C Fixture One</title>", body)
        self.assertEqual(response.headers["Content-Type"], "text/html; charset=utf-8")

    def test_redirect_chain_finishes_on_expected_local_page(self):
        with urllib.request.urlopen(
            f"{self.server.http_base}/redirect/one", timeout=2
        ) as response:
            body = response.read()

        self.assertTrue(response.url.endswith("/pages/final"))
        self.assertIn(b"M18-C Fixture Final", body)

    def test_http_to_https_redirect_uses_the_local_trusted_certificate(self):
        opener = urllib.request.build_opener(
            urllib.request.HTTPSHandler(context=self.trusted_tls)
        )
        with opener.open(f"{self.server.http_base}/redirect/https", timeout=2) as response:
            body = response.read()

        self.assertTrue(response.url.startswith(self.server.https_base))
        self.assertIn(b"M18-C Fixture Secure", body)

    def test_redirect_loop_fails_with_a_bounded_http_error(self):
        with self.assertRaises(urllib.error.HTTPError) as raised:
            urllib.request.urlopen(f"{self.server.http_base}/redirect/loop-a", timeout=2)

        self.assertIn(raised.exception.code, (301, 302, 307, 308))
        raised.exception.close()

    def test_non_success_responses_are_observable(self):
        for status in (404, 503):
            with self.subTest(status=status):
                with self.assertRaises(urllib.error.HTTPError) as raised:
                    urllib.request.urlopen(
                        f"{self.server.http_base}/status/{status}", timeout=2
                    )
                self.assertEqual(raised.exception.code, status)
                raised.exception.close()

    def test_connection_refusal_uses_a_closed_loopback_port(self):
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]

        with self.assertRaises((urllib.error.URLError, OSError)):
            urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=1)

    def test_fixture_lifecycle_closes_ephemeral_listener_sockets(self):
        server = FixtureServer().start()
        http_port = server.http_port
        https_port = server.https_port
        server.close()

        for port in (http_port, https_port):
            with self.subTest(port=port), self.assertRaises(OSError):
                socket.create_connection(("127.0.0.1", port), timeout=0.1)

    def test_timeout_fixture_is_bounded_and_local(self):
        with self.assertRaises((TimeoutError, urllib.error.URLError, OSError)):
            urllib.request.urlopen(f"{self.server.http_base}/timeout", timeout=0.05)

    def test_malformed_response_is_rejected_by_the_http_client(self):
        with self.assertRaises((http.client.HTTPException, urllib.error.URLError)):
            urllib.request.urlopen(f"{self.server.http_base}/malformed", timeout=2)

    def test_https_succeeds_only_when_the_fixture_certificate_is_trusted(self):
        with urllib.request.urlopen(
            f"{self.server.https_base}/pages/secure", context=self.trusted_tls, timeout=2
        ) as response:
            self.assertEqual(response.status, 200)
            self.assertIn(b"M18-C Fixture Secure", response.read())

        with self.assertRaises(urllib.error.URLError):
            urllib.request.urlopen(f"{self.server.https_base}/pages/secure", timeout=2)

    def test_https_rejects_a_hostname_mismatch_even_with_the_fixture_ca(self):
        raw_socket = socket.create_connection(("127.0.0.1", self.server.https_port), timeout=2)
        try:
            with self.assertRaises(ssl.SSLCertVerificationError):
                self.trusted_tls.wrap_socket(raw_socket, server_hostname="wrong.invalid")
        finally:
            raw_socket.close()

    def test_qemu_gateway_hostname_is_covered_by_the_fixture_certificate(self):
        raw_socket = socket.create_connection(("127.0.0.1", self.server.https_port), timeout=2)
        try:
            tls_socket = self.trusted_tls.wrap_socket(raw_socket, server_hostname="10.0.2.2")
            tls_socket.close()
        finally:
            raw_socket.close()

    def test_qemu_fixture_redirect_advertises_the_guest_gateway(self):
        server = FixtureServer(advertised_host="10.0.2.2").start()
        connection = http.client.HTTPConnection("127.0.0.1", server.http_port, timeout=2)
        try:
            connection.request("GET", "/redirect/https")
            response = connection.getresponse()
            self.assertEqual(response.status, 302)
            location = response.getheader("Location")
            self.assertIsNotNone(location)
            self.assertTrue(location.startswith("https://10.0.2.2:"))
        finally:
            connection.close()
            server.close()

    def test_download_response_has_a_stable_name_and_body(self):
        with urllib.request.urlopen(f"{self.server.http_base}/download", timeout=2) as response:
            body = response.read()

        self.assertEqual(response.headers.get_filename(), "fixture-download.txt")
        self.assertEqual(body, self.server.download_body)

    def test_upload_response_reports_exact_length_and_digest(self):
        payload = b"M18-C deterministic upload\n" * 7
        request = urllib.request.Request(
            f"{self.server.http_base}/upload", data=payload, method="POST"
        )
        with urllib.request.urlopen(request, timeout=2) as response:
            result = json.loads(response.read())

        self.assertEqual(result, {
            "bytes_received": len(payload),
            "sha256": hashlib.sha256(payload).hexdigest(),
        })

    def test_upload_fixture_rejects_payloads_over_its_declared_bound(self):
        payload = b"x" * (self.server.max_upload_bytes + 1)
        request = urllib.request.Request(
            f"{self.server.http_base}/upload", data=payload, method="POST"
        )
        with self.assertRaises(urllib.error.HTTPError) as raised:
            urllib.request.urlopen(request, timeout=2)
        self.assertEqual(raised.exception.code, 413)
        raised.exception.close()


class BrowserAcceptanceReportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = load_manifest()

    def test_missing_product_evidence_is_reported_as_blocked_not_passed(self):
        report = build_report(self.manifest, branch="codex/m18c-browser-acceptance-reliability")

        self.assertEqual(report["status"], "PARTIAL")
        self.assertGreater(report["summary"]["SKIP-BLOCKED"], 0)
        self.assertEqual(report["summary"]["PASS"], 0)
        self.assertTrue(all(case["owner_workstreams"] for case in report["cases"]))

    def test_case_evidence_is_joined_by_id_and_requires_a_reason(self):
        case_id = self.manifest["cases"][0]["id"]
        evidence = {
            "schema_version": 1,
            "cases": [{"id": case_id, "status": "PASS", "evidence": "guest marker"}],
        }

        report = build_report(self.manifest, evidence=evidence)
        passed = next(case for case in report["cases"] if case["id"] == case_id)
        self.assertEqual(passed["status"], "PASS")
        self.assertEqual(passed["evidence"], "guest marker")
        self.assertEqual(report["status"], "PARTIAL")

    def test_unavailable_gate_evidence_is_blocked_with_its_reason(self):
        evidence = blocked_feature_graph_evidence("pinned dependency source is unavailable")
        report = build_report(self.manifest, evidence=evidence)
        gate = next(case for case in report["cases"] if case["id"] == "features.m17-excludes-m18")

        self.assertEqual(gate["status"], "SKIP-BLOCKED")
        self.assertIn("pinned dependency source is unavailable", gate["reason"])
        self.assertEqual(report["status"], "PARTIAL")

    def test_report_passes_only_when_every_case_has_pass_evidence(self):
        evidence = {
            "schema_version": 1,
            "cases": [
                {"id": case["id"], "status": "PASS", "evidence": "guest acceptance marker"}
                for case in self.manifest["cases"]
            ],
        }

        report = build_report(self.manifest, evidence=evidence)
        self.assertEqual(report["status"], "PASS")
        self.assertEqual(report["summary"]["PASS"], len(self.manifest["cases"]))
        self.assertEqual(report["summary"]["SKIP-BLOCKED"], 0)

    def test_fail_evidence_makes_the_report_fail(self):
        case_id = self.manifest["cases"][0]["id"]
        evidence = {
            "schema_version": 1,
            "cases": [{"id": case_id, "status": "FAIL", "evidence": "missing URL event"}],
        }

        report = build_report(self.manifest, evidence=evidence)
        self.assertEqual(report["status"], "FAIL")

    def test_unimplemented_cases_are_not_reported_as_blocked_or_passed(self):
        manifest = json.loads(json.dumps(self.manifest))
        manifest["cases"][0].pop("blocker")

        report = build_report(manifest)
        first_case = next(
            case for case in report["cases"] if case["id"] == manifest["cases"][0]["id"]
        )
        self.assertEqual(first_case["status"], "NOT-IMPLEMENTED")
        self.assertEqual(report["summary"]["PASS"], 0)

    def test_duplicate_unknown_or_evidence_free_cases_are_rejected(self):
        invalid = (
            {"schema_version": 1, "cases": [
                {"id": "unknown.case", "status": "PASS", "evidence": "marker"}
            ]},
            {"schema_version": 1, "cases": [
                {"id": self.manifest["cases"][0]["id"], "status": "PASS", "evidence": "one"},
                {"id": self.manifest["cases"][0]["id"], "status": "PASS", "evidence": "two"},
            ]},
            {"schema_version": 1, "cases": [
                {"id": self.manifest["cases"][0]["id"], "status": "PASS", "evidence": " "}
            ]},
        )
        for evidence in invalid:
            with self.subTest(evidence=evidence), self.assertRaises(ReportError):
                build_report(self.manifest, evidence=evidence)

    def test_m17_feature_graph_passes_only_when_m18_features_are_absent(self):
        graph = '\n'.join((
            'nagi-init v0.1.0',
            'nagi-init feature "m17-servo"',
            'nagi-albert v0.1.0',
        ))
        evidence = evaluate_m17_feature_graph(graph)
        self.assertEqual(evidence["cases"][0]["status"], "PASS")

        leaking_graph = graph + '\nnagi-posix feature "browser-storage"'
        failed = evaluate_m17_feature_graph(leaking_graph)
        self.assertEqual(failed["cases"][0]["status"], "FAIL")

        albert_leak = graph + '\nnagi-albert feature "m18-acceptance"'
        failed = evaluate_m17_feature_graph(albert_leak)
        self.assertEqual(failed["cases"][0]["status"], "FAIL")

    def test_m17_feature_gate_requires_expected_graph_nodes(self):
        evidence = evaluate_m17_feature_graph('nagi-init v0.1.0')
        self.assertEqual(evidence["cases"][0]["status"], "FAIL")


class BrowserStateFixtureTests(unittest.TestCase):
    def test_recovery_scenarios_are_explicit_and_deterministic(self):
        scenario_ids = {fixture.scenario_id for fixture in STATE_FIXTURES}
        self.assertEqual(len(scenario_ids), len(STATE_FIXTURES))
        self.assertTrue({
            "session.missing",
            "session.empty",
            "session.clean-multitab",
            "session.truncated",
            "session.corrupt",
            "session.incompatible-version",
            "session.partial-records",
            "reliability.interrupted-write",
            "reliability.repeated-restart",
        }.issubset(scenario_ids))

        with tempfile.TemporaryDirectory() as first_root, tempfile.TemporaryDirectory() as second_root:
            for fixture in STATE_FIXTURES:
                first = materialize_state_fixture(fixture.scenario_id, Path(first_root))
                second = materialize_state_fixture(fixture.scenario_id, Path(second_root))
                self.assertEqual(
                    {path.relative_to(first): path.read_bytes() for path in first.rglob("*") if path.is_file()},
                    {path.relative_to(second): path.read_bytes() for path in second.rglob("*") if path.is_file()},
                    fixture.scenario_id,
                )

    def test_clean_multitab_fixture_has_a_stable_fixture_envelope(self):
        with tempfile.TemporaryDirectory() as temp_root:
            directory = materialize_state_fixture("session.clean-multitab", Path(temp_root))
            record = json.loads((directory / "session.json").read_bytes())

        self.assertEqual(record["fixture_format"], FIXTURE_FORMAT)
        self.assertEqual(record["kind"], "session")
        self.assertEqual(len(record["payload"]["tabs"]), 2)
        self.assertEqual(record["payload"]["active_tab"], "tab-02")

    def test_corrupt_and_incompatible_fixtures_cover_distinct_inputs(self):
        with tempfile.TemporaryDirectory() as temp_root:
            empty = materialize_state_fixture("session.empty", Path(temp_root))
            truncated = materialize_state_fixture("session.truncated", Path(temp_root))
            corrupt = materialize_state_fixture("session.corrupt", Path(temp_root))
            incompatible = materialize_state_fixture("session.incompatible-version", Path(temp_root))
            self.assertEqual((empty / "session.json").read_bytes(), b"")
            for directory in (truncated, corrupt):
                with self.subTest(directory=directory.name), self.assertRaises((UnicodeDecodeError, json.JSONDecodeError)):
                    json.loads((directory / "session.json").read_bytes())
            incompatible_record = json.loads((incompatible / "session.json").read_bytes())
            self.assertEqual(incompatible_record["fixture_format"], "m18c-test-fixture-v99")

    def test_partial_and_interrupted_fixtures_preserve_good_and_incomplete_bytes(self):
        with tempfile.TemporaryDirectory() as temp_root:
            partial = materialize_state_fixture("session.partial-records", Path(temp_root))
            interrupted = materialize_state_fixture("reliability.interrupted-write", Path(temp_root))
            self.assertEqual(
                json.loads((partial / "history/entry-01.json").read_bytes())["kind"],
                "history-entry",
            )
            with self.assertRaises(json.JSONDecodeError):
                json.loads((partial / "history/entry-02.json").read_bytes())
            self.assertEqual(json.loads((partial / "bookmarks.json").read_bytes())["kind"], "bookmarks")
            self.assertEqual(
                json.loads((interrupted / "records/committed.json").read_bytes())["payload"]["checkpoint"],
                "before-interrupted-write",
            )
            with self.assertRaises(json.JSONDecodeError):
                json.loads((interrupted / "records/pending.json.tmp").read_bytes())

    def test_missing_fixture_materializes_no_storage_record(self):
        with tempfile.TemporaryDirectory() as temp_root:
            directory = materialize_state_fixture("session.missing", Path(temp_root))
            self.assertEqual(list(directory.iterdir()), [])
        with self.assertRaises(KeyError):
            materialize_state_fixture("session.unknown", Path(temp_root))


if __name__ == "__main__":
    unittest.main()
