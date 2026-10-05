"""Regression coverage for the M17 linker diagnostic inventory parser."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT / "tools" / "nagi-target-link-diagnostics.py"
SPEC = importlib.util.spec_from_file_location("nagi_target_link_diagnostics", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
diagnostics = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(diagnostics)


class TargetLinkDiagnosticsRegressionTests(unittest.TestCase):
    def test_inventory_keeps_all_unique_errors_and_discards_repeated_annotations(self) -> None:
        log = "\n".join(
            [
                "rust-lld: error: undefined symbol: first_symbol",
                "rust-lld: error: undefined symbol: second_symbol",
                "rust-lld: error: undefined symbol: third_symbol",
                "rust-lld: error: undefined symbol: fourth_symbol",
                "rust-lld: error: undefined symbol: second_symbol",
                "::error title=M17 target undefined symbols (1/1)::first_symbol | second_symbol",
                "##[error]rust-lld: error: undefined symbol: annotation_payload",
            ]
        )
        with tempfile.TemporaryDirectory() as temporary_directory:
            log_path = Path(temporary_directory) / "target-link.log"
            log_path.write_text(log, encoding="utf-8")

            self.assertEqual(
                diagnostics.read_symbols(log_path),
                ["first_symbol", "second_symbol", "third_symbol", "fourth_symbol"],
            )


if __name__ == "__main__":
    unittest.main()
