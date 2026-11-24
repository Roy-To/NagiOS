import unittest

import task24_recovery_serials as recovery
import task24_ws10_serials as ws10


class DesktopLogoutSerialTests(unittest.TestCase):
    note = "1234567890abcdef"
    saved = f"{recovery.SAVED} object={note} revision=1"
    reopened = f"{recovery.REOPENED} object={note} revision=1"
    previous_logout = "NAGI_DEV_WS10_PREVIOUS_SESSION_ENDED reason=UserLogout"

    def setUp(self):
        r = recovery

        def receipt(marker, boot):
            return r._receipt_line(
                marker, generation=boot, launch=boot * 2, pid=boot * 100, c1=boot * 3,
                revision=boot * 4, terminal=boot * 5,
            )

        def terminal(boot):
            return [
                r.REAPED_NORMAL, r.IDENTITY_EXIT_REQUESTED,
                receipt(r.C3_IDENTITY_NORMAL_EXIT_REAPED, boot), receipt(r.CURRENT_TERMINAL, boot),
            ]

        prefix = [r.SESSION, r.HANDSHAKE, r.READY, r.DESKTOP_READY, r.DESKTOP_LAUNCH]
        self.first_close = [r.INPUT, self.saved, r.CLOSED, r.DESKTOP_CLOSED]
        again = [r.DESKTOP_LAUNCH, r.DESKTOP_REISSUED, self.reopened, r.INPUT, r.CLOSED, r.DESKTOP_CLOSED]
        logout = [r.DESKTOP_LOGOUT, r.SESSION_LOGOUT]
        self.save_prefix = [r.MOUNT, *prefix, *self.first_close]
        self.save_suffix = [*logout, *terminal(1)]
        self.save = self.save_prefix + self.save_suffix
        self.l7_save = self.save_prefix + again + self.save_suffix
        self.previous_terminal = receipt(r.PREVIOUS_TERMINAL, 1)
        self.read = [
            r.MOUNT, self.previous_terminal, self.previous_logout, r.LOGIN_WAIT, *prefix,
            self.reopened, r.INPUT, r.CLOSED, r.DESKTOP_CLOSED, *logout, *terminal(2),
        ]

    def assert_logout_rejected(self, save, read):
        with self.assertRaises(ValueError):
            recovery.validate_desktop_logout_save_read(recovery._serial(save), recovery._serial(read))

    def test_logout_keeps_the_single_notes_close_and_relogin(self):
        self.assertEqual(
            recovery.validate_desktop_logout_save_read(recovery._serial(self.save), recovery._serial(self.read)),
            {"object": self.note, "revision": 1, "ready_count": 1},
        )
        ws10.validate("logout", self.save, self.read)
        with self.assertRaises(ValueError):
            recovery.validate_save_read(recovery._serial(self.save), recovery._serial(self.read))

    def test_normal_l7_requires_reacquiring_the_same_note_before_logout(self):
        self.assertEqual(
            recovery.validate_save_read(recovery._serial(self.l7_save), recovery._serial(self.read))["object"],
            self.note,
        )
        self.assert_logout_rejected(self.l7_save, self.read)
        wrong_reacquire = [line.replace("revision=1", "revision=2") if line == self.reopened else line
                           for line in self.l7_save]
        with self.assertRaises(ValueError):
            recovery.validate_save_read(recovery._serial(wrong_reacquire), recovery._serial(self.read))

    def test_logout_rejects_missing_reordered_or_changed_evidence(self):
        r = recovery
        for marker in (r.DESKTOP_LOGOUT, r.SESSION_LOGOUT, r.DESKTOP_CLOSED):
            with self.subTest(missing_save=marker):
                self.assert_logout_rejected([line for line in self.save if line != marker], self.read)
        for marker in (self.previous_logout, self.previous_terminal, self.reopened, r.LOGIN_WAIT):
            with self.subTest(missing_read=marker):
                self.assert_logout_rejected(self.save, [line for line in self.read if line != marker])
        self.assert_logout_rejected(
            self.save_prefix + [r.SESSION_LOGOUT, r.DESKTOP_LOGOUT] + self.save_suffix[2:], self.read,
        )
        self.assert_logout_rejected(self.save, [*self.read[:2], r.LOGIN_WAIT, self.previous_logout, *self.read[4:]])
        self.assert_logout_rejected(
            self.save, [line.replace("terminal_operation_id=5", "terminal_operation_id=9")
                        if line == self.previous_terminal else line for line in self.read],
        )
        self.assert_logout_rejected(
            self.save, [line.replace("revision=1", "revision=2") if line == self.reopened else line
                        for line in self.read],
        )
