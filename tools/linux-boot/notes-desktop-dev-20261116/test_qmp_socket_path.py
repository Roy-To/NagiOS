#!/usr/bin/env python3
"""QMP socket path regressions for long integration worktree paths."""

from __future__ import annotations

import os
import sys
import tempfile
import unittest
from pathlib import Path


LANE = Path(__file__).resolve().parent
ROOT = LANE.parents[2]
sys.path.insert(0, str(LANE))
import run_persistence_acceptance as acceptance


class QmpSocketPathTests(unittest.TestCase):
    def test_socket_path_is_short_and_worktree_relative_for_deep_output(self) -> None:
        with tempfile.TemporaryDirectory(prefix=".qmp-path-test-", dir=ROOT) as temporary:
            output = Path(temporary) / ("nested-" + "x" * 80)
            output.mkdir()

            qmp = acceptance.unique_qmp_socket(output, "save")

            self.assertFalse(qmp.is_absolute())
            self.assertLess(len(os.fsencode(qmp)), 100)
            self.assertEqual(qmp.parent, Path("."))
            self.assertFalse((ROOT / qmp).exists())

    def test_socket_names_are_unique_across_acceptance_phases(self) -> None:
        with tempfile.TemporaryDirectory(prefix=".qmp-path-test-", dir=ROOT) as temporary:
            output = Path(temporary)
            save = acceptance.unique_qmp_socket(output, "save")
            read = acceptance.unique_qmp_socket(output, "read")
            another_save = acceptance.unique_qmp_socket(output, "save")

            self.assertNotEqual(save, read)
            self.assertNotEqual(save, another_save)
            self.assertLess(len(os.fsencode(save)), 100)
            self.assertLess(len(os.fsencode(read)), 100)


if __name__ == "__main__":
    unittest.main()
