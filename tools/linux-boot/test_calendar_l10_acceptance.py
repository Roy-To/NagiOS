import unittest

from calendar_l10_acceptance import CalendarAcceptanceError, validate


def boot(count, body):
    return "\n".join([
        "NAGI_DEV_DESKTOP_READY",
        "NAGI_DEV_DESKTOP_APP_LAUNCH app=calendar",
        f"NAGI_DEV_CALENDAR_READY events={count} date=2026-11-23",
        *body,
        "NAGI_DEV_DESKTOP_APP_CLOSED app=calendar exit=ClosedByUser",
        "NAGI_DEV_DESKTOP_LOGOUT_REQUESTED",
    ])


def save(op, event):
    return f"NAGI_DEV_CALENDAR_SAVED op={op} id=event_{event} bytes=104 sha256=1234567890ab"


DAYS = [
    "NAGI_DEV_CALENDAR_DAY date=2026-11-23 rows=2 ids=event_1,event_3",
    "NAGI_DEV_CALENDAR_DAY date=2026-11-24 rows=2 ids=event_2,event_3",
]
BEFORE = boot(0, [
    save("create", 1), save("create", 2), save("create", 3),
    save("update", 2), save("create", 4), save("delete", 4), *DAYS,
])
AFTER = boot(3, DAYS)


class CalendarAcceptanceTests(unittest.TestCase):
    def test_two_boots_preserve_the_expected_ids(self):
        validate(BEFORE, AFTER)

    def test_rejects_ready_before_launch(self):
        self.assert_rejected(AFTER.replace("NAGI_DEV_DESKTOP_APP_LAUNCH app=calendar\n", ""))

    def test_rejects_deleted_event_after_reboot(self):
        self.assert_rejected(AFTER.replace("event_2,event_3", "event_2,event_4"))

    def test_rejects_a_write_during_reboot_readback(self):
        self.assert_rejected(boot(3, [save("update", 2), *DAYS]))

    def test_rejects_uncertain_close(self):
        self.assert_rejected(AFTER.replace("exit=ClosedByUser", "exit=StateUnknown"))

    def assert_rejected(self, after):
        with self.assertRaises(CalendarAcceptanceError):
            validate(BEFORE, after)
