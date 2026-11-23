import json
import os
import socket
import tempfile
import time
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest.mock import Mock, call, patch

import task24_recovery_serials as recovery
import task24_rfb_automatic as automatic


class AutomaticRfbTests(unittest.TestCase):
    def setUp(self):
        stack = self.enterContext(ExitStack())
        run = Path(stack.enter_context(tempfile.TemporaryDirectory())).resolve()
        self.fixture_path = run / "development-fixture.json"
        self.scope = {
            "run_directory": run,
            "run_id": run.name,
            "preflight_manifest_sha256": "a" * 64,
        }
        self.fixture_input = {
            "schema": automatic.FIXTURE_SCHEMA,
            "run_id": run.name,
            "preflight_manifest_sha256": "a" * 64,
            "rfb_port": 5968,
            "owner_name": "fixtureowner",
            "owner_password": "fixture-secret-one",
            "wrong_password": "fixture-secret-two",
            "note_text": "fixture body",
        }
        self.write_fixture()
        self.fixture = automatic.load_development_fixture(self.fixture_path, **self.scope)
        self.supervisor_log = run / "qemu-supervisor.log"
        self.supervisor_log.write_text("qemu_child_started pid=12345\n", encoding="utf-8")

    def write_fixture(self):
        self.fixture_path.write_text(json.dumps(self.fixture_input), encoding="utf-8")
        self.fixture_path.chmod(0o600)

    def test_fixture_is_private_bound_and_kept_out_of_repr(self):
        for key in ("owner_name", "owner_password", "wrong_password", "note_text"):
            self.assertNotIn(self.fixture_input[key], repr(self.fixture))
        self.fixture_input["preflight_manifest_sha256"] = "b" * 64
        self.write_fixture()
        with self.assertRaisesRegex(automatic.AutomationError, "not bound"):
            automatic.load_development_fixture(self.fixture_path, **self.scope)
        self.fixture_path.chmod(0o644)
        with self.assertRaisesRegex(automatic.AutomationError, "private single-link file"):
            automatic.load_development_fixture(self.fixture_path, **self.scope)

    def test_fixture_owner_name_matches_guest_registration_rules(self):
        for name in ("", "1fixture", "fixture-owner", "Aowner", "a" * 33):
            with self.subTest(name=name):
                self.fixture_input["owner_name"] = name
                self.write_fixture()
                with self.assertRaisesRegex(automatic.AutomationError, "invalid for guest registration"):
                    automatic.load_development_fixture(self.fixture_path, **self.scope)
        for name in ("a", "a0", "a" * 32):
            with self.subTest(name=name):
                self.fixture_input["owner_name"] = name
                self.write_fixture()
                self.assertEqual(
                    automatic.load_development_fixture(self.fixture_path, **self.scope).owner_name,
                    name,
                )

    def test_save_waits_for_registration_unlock_note_and_save_in_order(self):
        driver, keyboard, connect = self.make_driver("save")
        saved = f"{recovery.SAVED} object=0000000000000001 revision=1"
        markers = [automatic.REGISTRATION_WAIT, recovery.LOGIN_WAIT, recovery.INPUT, saved]
        self.feed(driver, markers[0], terminated=False)
        connect.assert_not_called()
        self.feed(driver, *markers[:1])
        self.assertNotIn(call.type_fixture_text(self.fixture.note_text), keyboard.mock_calls)
        self.feed(driver, *markers[:2])
        self.assertNotIn(call.type_fixture_text(self.fixture.note_text), keyboard.mock_calls)
        self.feed(driver, *markers, terminated=False)
        self.assertNotIn(call.tap(0xFF1B), keyboard.mock_calls)
        self.feed(driver, *markers)
        self.feed(driver, *markers)
        connect.assert_called_once()
        self.assertEqual(keyboard.mock_calls, [
            call.tap(0xFF0D),
            call.type_fixture_text(self.fixture.owner_name),
            call.tap(0xFF09),
            call.type_fixture_text(self.fixture.owner_password),
            call.tap(0xFF09),
            call.type_fixture_text(self.fixture.owner_password),
            call.tap(0xFF0D),
            call.type_fixture_text(self.fixture.owner_password),
            call.tap(0xFF0D),
            call.type_fixture_text(self.fixture.note_text),
            call.tap(0xFF0D),
            call.tap(0xFF1B),
        ])
        driver.verify_complete()

    def test_desktop_save_relaunches_before_logout(self):
        driver, keyboard, _ = self.make_driver("save")
        markers = [automatic.REGISTRATION_WAIT, recovery.LOGIN_WAIT]
        self.feed(driver, *markers)
        keyboard.reset_mock()
        for marker in recovery.DESKTOP_NOTES_FLOW["save"]:
            if marker in (recovery.SAVED, recovery.REOPENED):
                marker += " object=0000000000000001 revision=1"
            markers.append(marker)
            self.feed(driver, *markers)
            self.feed(driver, *markers)
        driver.verify_complete()
        self.assertEqual(keyboard.mock_calls, [
            call.tap(0xFF0D),  # First launch
            call.type_fixture_text(self.fixture.note_text),
            call.tap(0xFF0D),  # Save
            call.tap(0xFF1B),
            call.tap(0xFF0D),  # Relaunch
            call.tap(0xFF1B),
            call.tap(0xFF09),
            call.tap(0xFF0D),  # Logout
        ])

    def test_desktop_read_requires_reopen_before_accepting_input(self):
        driver, keyboard, _ = self.make_driver("read")
        markers = [recovery.LOGIN_WAIT]
        self.feed(driver, *markers)
        keyboard.reset_mock()
        markers.append(recovery.DESKTOP_READY)
        self.feed(driver, *markers)
        markers.append(recovery.DESKTOP_LAUNCH)
        self.feed(driver, *markers)
        with self.assertRaisesRegex(automatic.AutomationError, "out-of-order desktop marker"):
            self.feed(driver, *markers, recovery.INPUT)
        self.assertEqual(keyboard.mock_calls, [call.tap(0xFF0D)])
        for marker in recovery.DESKTOP_NOTES_FLOW["read"][2:]:
            if marker == recovery.REOPENED:
                marker += " object=0000000000000001 revision=1"
            markers.append(marker)
            self.feed(driver, *markers)
        driver.verify_complete()
        self.assertEqual(keyboard.mock_calls, [
            call.tap(0xFF0D), call.tap(0xFF1B), call.tap(0xFF09), call.tap(0xFF0D),
        ])

    def test_reject_sends_one_wrong_password_and_waits_for_return_to_login(self):
        driver, keyboard, _ = self.make_driver("reject-close")
        self.feed(driver, recovery.LOGIN_WAIT, recovery.LOGIN_REJECTED)
        with self.assertRaisesRegex(automatic.AutomationError, "did not complete"):
            driver.verify_complete()
        self.feed(driver, recovery.LOGIN_WAIT, recovery.LOGIN_REJECTED, recovery.LOGIN_WAIT)
        self.feed(driver, recovery.LOGIN_WAIT, recovery.LOGIN_REJECTED, recovery.LOGIN_WAIT)
        self.assertEqual(keyboard.mock_calls, [
            call.type_fixture_text(self.fixture.wrong_password),
            call.tap(0xFF0D),
            call.tap(0xFF1B),
        ])
        driver.verify_complete()

    def test_read_only_closes_after_reopen_and_reports_body_unverified(self):
        driver, keyboard, _ = self.make_driver("read")
        self.feed(driver, recovery.LOGIN_WAIT)
        with self.assertRaisesRegex(automatic.AutomationError, "without reopening"):
            self.feed(driver, recovery.LOGIN_WAIT, recovery.INPUT)
        self.assertNotIn(call.tap(0xFF1B), keyboard.mock_calls)
        reopened = f"{recovery.REOPENED} object=0000000000000001 revision=1"
        self.feed(driver, recovery.LOGIN_WAIT, reopened, recovery.INPUT)
        driver.verify_complete()
        self.assertEqual(keyboard.mock_calls, [
            call.type_fixture_text(self.fixture.owner_password),
            call.tap(0xFF0D),
            call.tap(0xFF1B),
        ])
        self.assertEqual(driver.summary()["post_action_display_content"], "NOT_VERIFIED")

    @unittest.skipUnless(Path("/usr/sbin/lsof").is_file(), "local lsof is unavailable")
    def test_lsof_verifies_actual_ipv4_listener(self):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
            server.bind(("127.0.0.1", 0))
            server.listen()
            port = server.getsockname()[1]
            self.assertTrue(automatic.owned_loopback_listener(os.getpid(), port))
            self.assertFalse(automatic.owned_loopback_listener(os.getppid(), port))

    def make_driver(self, phase):
        keyboard = Mock()
        self.enterContext(patch.object(automatic, "owned_loopback_listener", return_value=True))
        connect = self.enterContext(
            patch.object(automatic.RfbKeyboard, "connect_local", return_value=keyboard)
        )
        return automatic.AutomaticRfbInput(self.fixture, phase), keyboard, connect

    def feed(self, driver, *lines, terminated=True):
        serial = "\n".join(lines)
        if terminated:
            serial += "\n"
        driver.handle(serial, self.supervisor_log, time.monotonic() + 3)


if __name__ == "__main__":
    unittest.main()
