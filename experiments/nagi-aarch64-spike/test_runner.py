import unittest
from run import MARKERS, accepted


class AcceptanceTests(unittest.TestCase):
    def test_complete_ordered_guest_and_zero_exit(self):
        self.assertTrue(accepted(0, b"\n".join(MARKERS), False))

    def test_missing_duplicate_and_reordered_markers(self):
        self.assertFalse(accepted(0, b"\n".join(MARKERS[:-1]), False))
        self.assertFalse(accepted(0, b"\n".join(MARKERS + [MARKERS[-1]]), False))
        self.assertFalse(accepted(0, b"\n".join(reversed(MARKERS)), False))

    def test_crash_failure_and_timeout_even_with_markers(self):
        output = b"\n".join(MARKERS)
        self.assertFalse(accepted(1, output, False))
        self.assertFalse(accepted(-11, output, False))
        self.assertFalse(accepted(None, output, True))
        self.assertFalse(accepted(0, output + b"\nSPIKE FAIL", False))


if __name__ == "__main__":
    unittest.main()
