"""Host-only guards for Task24 Data provisioning and save/read serial checks."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


TOOLS = Path(__file__).resolve().parents[1]


def load_tool(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


data_runner = load_tool(
    "run_data_owner_persistence",
    TOOLS / "run_data_owner_persistence.py",
)
serials = load_tool("task24_recovery_serials", TOOLS / "task24_recovery_serials.py")


def make_save_read_pair() -> tuple[str, str]:
    note_id = "0000000000000001"
    save_c3 = serials._receipt_line(
        serials.C3_IDENTITY_NORMAL_EXIT_REAPED,
        generation=1,
        launch=11,
        pid=44,
        c1=7,
        revision=9,
        terminal=12,
    )
    save_c1 = serials._receipt_line(
        serials.CURRENT_TERMINAL,
        generation=1,
        launch=11,
        pid=44,
        c1=7,
        revision=9,
        terminal=12,
    )
    save = serials._serial(
        [
            serials.MOUNT,
            serials.SESSION,
            serials.HANDSHAKE,
            serials.READY,
            serials.INPUT,
            f"{serials.SAVED} object={note_id} revision=2",
            serials.CLOSED,
            serials.REAPED_NORMAL,
            serials.IDENTITY_EXIT_REQUESTED,
            save_c3,
            save_c1,
        ]
    )

    read_c3 = serials._receipt_line(
        serials.C3_IDENTITY_NORMAL_EXIT_REAPED,
        generation=2,
        launch=21,
        pid=55,
        c1=8,
        revision=15,
        terminal=22,
    )
    read_c1 = serials._receipt_line(
        serials.CURRENT_TERMINAL,
        generation=2,
        launch=21,
        pid=55,
        c1=8,
        revision=15,
        terminal=22,
    )
    previous = serials._receipt_line(
        serials.PREVIOUS_TERMINAL,
        generation=1,
        launch=11,
        pid=44,
        c1=7,
        revision=9,
        terminal=12,
    )
    read = serials._serial(
        [
            serials.MOUNT,
            previous,
            serials.LOGIN_WAIT,
            serials.SESSION,
            serials.HANDSHAKE,
            serials.READY,
            f"{serials.REOPENED} object={note_id} revision=2",
            serials.INPUT,
            serials.CLOSED,
            serials.REAPED_NORMAL,
            serials.IDENTITY_EXIT_REQUESTED,
            read_c3,
            read_c1,
        ]
    )
    return save, read


class Task24RecoveryPreflightTests(unittest.TestCase):
    def test_provision_marker_requires_one_exact_success_line(self) -> None:
        marker = (
            "NAGI_DEV_IDENTITY_DIRECTORIES_PROVISIONED "
            "accounts=accounts snapshots=snapshots "
            "c3_journal=system-service-journal first_provision=true "
            "owner_ended=true"
        )
        self.assertEqual(data_runner.provisioned_marker_line(marker + "\n"), marker)
        self.assertIsNone(data_runner.provisioned_marker_line(marker + "\n" + marker + "\n"))
        self.assertIsNone(
            data_runner.provisioned_marker_line(marker.replace("first_provision=true", "first_provision=false") + "\n")
        )

    def test_save_read_accepts_the_same_normal_exit_receipt(self) -> None:
        save, read = make_save_read_pair()
        self.assertEqual(
            serials.validate_save_read(save, read),
            {"object": "0000000000000001", "revision": 2, "ready_count": 1},
        )

    def test_read_must_consume_the_exact_save_boot_receipt_once(self) -> None:
        save, read = make_save_read_pair()
        previous = serials._receipt_line(
            serials.PREVIOUS_TERMINAL,
            generation=1,
            launch=11,
            pid=44,
            c1=7,
            revision=9,
            terminal=12,
        )
        with self.assertRaises(ValueError):
            serials.validate_save_read(save, read + previous + "\n")

        mismatched = previous.replace("c1_operation_id=7", "c1_operation_id=70")
        with self.assertRaises(ValueError):
            serials.validate_save_read(save, read.replace(previous, mismatched, 1))

    def test_read_rejects_a_different_saved_notes_object_or_revision(self) -> None:
        save, read = make_save_read_pair()
        reopened = f"{serials.REOPENED} object=0000000000000001 revision=2"
        mismatched_readbacks = {
            "different_object": read.replace(
                reopened,
                f"{serials.REOPENED} object=0000000000000002 revision=2",
                1,
            ),
            "different_revision": read.replace(
                reopened,
                f"{serials.REOPENED} object=0000000000000001 revision=3",
                1,
            ),
        }
        for name, mismatched in mismatched_readbacks.items():
            with self.subTest(name=name), self.assertRaisesRegex(
                ValueError, "saved and reopened Notes ObjectId/revision differ"
            ):
                serials.validate_save_read(save, mismatched)

    def test_save_must_have_a_normal_child_exit(self) -> None:
        save, read = make_save_read_pair()
        abnormal_save = save.replace(
            serials.REAPED_NORMAL,
            serials.REAPED + " status=ServiceExitStatus { exit_code: Some(1), signal: None }",
            1,
        )
        with self.assertRaises(ValueError):
            serials.validate_save_read(abnormal_save, read)

    def test_save_rejects_missing_malformed_duplicate_or_early_note_identity(self) -> None:
        save, _ = make_save_read_pair()
        saved_line = f"{serials.SAVED} object=0000000000000001 revision=2"
        broken_saves = {
            "missing": serials._serial(
                [line for line in save.splitlines() if line != saved_line]
            ),
            "malformed_object_id": save.replace(
                saved_line,
                f"{serials.SAVED} object=not-an-object-id revision=2",
            ),
            "zero_revision": save.replace(
                saved_line,
                f"{serials.SAVED} object=0000000000000001 revision=0",
            ),
            "duplicate": save.replace(
                saved_line, f"{saved_line}\n{saved_line}"
            ),
            "before_input_ready": save.replace(
                f"{serials.INPUT}\n{saved_line}",
                f"{saved_line}\n{serials.INPUT}",
            ),
        }
        for name, broken in broken_saves.items():
            with self.subTest(name=name), self.assertRaises(ValueError):
                serials._validate_phase(broken, "save", serials.SAVED)

    def test_save_and_read_note_markers_reject_body_or_extra_fields(self) -> None:
        save, read = make_save_read_pair()
        saved_marker = f"{serials.SAVED} object=0000000000000001 revision=2"
        reopened_marker = f"{serials.REOPENED} object=0000000000000001 revision=2"

        with self.assertRaisesRegex(ValueError, "well-formed NAGI_DEV_NOTES_SAVED"):
            serials._validate_phase(
                save.replace(saved_marker, f"{saved_marker} body=synthetic-secret"),
                "save",
                serials.SAVED,
            )
        with self.assertRaisesRegex(ValueError, "well-formed NAGI_DEV_NOTES_REOPENED"):
            serials._validate_phase(
                read.replace(reopened_marker, f"{reopened_marker} extra=synthetic-secret"),
                "read",
                serials.REOPENED,
            )


if __name__ == "__main__":
    unittest.main()
