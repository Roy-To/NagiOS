#!/usr/bin/env python3
"""Host checks for the separate Landlock control-fixture candidate.

The filesystem test exercises this host's ordinary DAC/filesystem path only.
The Linux source overlay adds a same-TEST_UID, no-Landlock control subprocess;
that target test is type-checked separately and is not claimed as executed here.
"""

from __future__ import annotations

import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
V1 = HERE / "legacy-source"
V2_PATCH = HERE / "source-overlay-v2.patch"
V3 = HERE
V3_PATCH = V3 / "control-fixture-overlay.patch"
PROBE = V3 / "landlock_raw_io_probe.S"


def macro_errno_values(source: str, macro_name: str) -> set[int]:
    match = re.search(
        rf"\.macro {macro_name} bit\n(.*?)\.endm",
        source,
        flags=re.DOTALL,
    )
    if not match:
        raise AssertionError(f"missing macro {macro_name}")
    return {int(value) for value in re.findall(r"cmp \$(-?\d+), %rax", match.group(1))}


class ControlFixtureTests(unittest.TestCase):
    def test_v3_probe_requires_control_evidence_before_errno_acceptance(self) -> None:
        source = PROBE.read_text()
        self.assertEqual(macro_errno_values(source, "EXPECT_EACCES"), {-13})
        self.assertEqual(macro_errno_values(source, "EXPECT_RENAME_DENIED"), {-13, -18})
        self.assertIn("same-credential no-Landlock controls succeed", source)
        self.assertIn("share st_dev", source)

    def test_overlay_applies_after_fixed_v2_without_editing_its_inputs(self) -> None:
        with tempfile.TemporaryDirectory(prefix="nagi-landlock-control-overlay-") as temp:
            temp_path = Path(temp)
            shutil.copy2(V1 / "linux_init_process_owner_candidate.rs", temp_path)
            shutil.copy2(V1 / "linux_x86_64_process.rs", temp_path)
            for patch_path in (V2_PATCH, V3_PATCH):
                with patch_path.open("rb") as patch_file:
                    subprocess.run(
                        ["patch", "--batch", "-p1"],
                        cwd=temp_path,
                        stdin=patch_file,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.STDOUT,
                        check=True,
                    )

            owner = (temp_path / "linux_x86_64_process.rs").read_text()
            self.assertIn("fn no_landlock_same_uid_control_child()", owner)
            self.assertIn("NAGI_WS04_CONTROL_PROFILE", owner)
            self.assertIn("NR_SETRESUID, TEST_UID as usize", owner)
            self.assertIn("assert_same_device(&foreign_rename_source, &allowed_rename_dest)", owner)
            self.assertIn("and_does_not_inherit_held_fd", owner)
            self.assertIn("does not observe", owner)
            self.assertIn("transient FD_CLOEXEC flag", owner)

    def test_current_host_same_credential_control_and_device_check(self) -> None:
        # This verifies the fixture mechanics for this host UID. The target
        # overlay separately runs the same operations as TEST_UID/TEST_GID.
        with tempfile.TemporaryDirectory(prefix="nagi-landlock-control-host-") as temp:
            base = Path(temp)
            app = base / "app"
            other = base / "other"
            profile = base / "profile"
            app.mkdir(mode=0o777)
            other.mkdir(mode=0o777)
            profile.mkdir(mode=0o777)
            os.chmod(app, 0o777)
            os.chmod(other, 0o777)
            os.chmod(profile, 0o777)
            profile_file = profile / "secret"
            profile_file.write_bytes(b"profile")
            os.chmod(profile_file, 0o666)
            profile_file.open("rb").close()
            with profile_file.open("ab") as stream:
                stream.write(b"+")
            source_in = other / "move-to-app"
            dest_in = app / "moved-from-other"
            source_out = app / "move-to-other"
            dest_out = other / "moved-from-app"
            source_in.write_bytes(b"in")
            source_out.write_bytes(b"out")
            os.chmod(source_in, 0o666)
            os.chmod(source_out, 0o666)

            self.assertEqual(os.stat(source_in.parent).st_dev, os.stat(dest_in.parent).st_dev)
            self.assertEqual(os.stat(source_out.parent).st_dev, os.stat(dest_out.parent).st_dev)
            source_in.open("rb").close()
            with source_in.open("ab") as stream:
                stream.write(b"+")
            created = other / "created-by-control"
            created.write_bytes(b"control")
            os.rename(source_in, dest_in)
            os.rename(source_out, dest_out)
            self.assertTrue(dest_in.is_file())
            self.assertTrue(dest_out.is_file())
            self.assertTrue(created.is_file())


if __name__ == "__main__":
    unittest.main()
