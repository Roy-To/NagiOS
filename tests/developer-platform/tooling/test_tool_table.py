"""Check the NDP-08 source references without running any documented tools."""

import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
AUDIT_PATH = REPO_ROOT / "docs/development/developer-platform/contract-audit.md"
SECTION_HEADING = "## NDP-08: 正規toolの利用表"
EVIDENCE_COLUMN = "根拠（repo 相対パスと識別子・flag）"
SOURCE_REFERENCE = re.compile(r"`([^`]+)` \((`[^`]+`(?:, `[^`]+`)*)\)")


def table_cells(line):
    line = line.strip()
    if not (line.startswith("|") and line.endswith("|")):
        raise ValueError(f"Invalid tool table row: {line!r}")
    return [cell.strip() for cell in line[1:-1].split("|")]


def tool_references(document):
    _, heading, rest = document.partition(f"\n{SECTION_HEADING}\n")
    if not heading:
        raise ValueError(f"Missing section: {SECTION_HEADING}")
    section = rest.split("\n## ", 1)[0]
    tables = [
        block.splitlines()
        for block in section.split("\n\n")
        if block.lstrip().startswith("|")
    ]
    if len(tables) != 1 or len(tables[0]) < 3:
        raise ValueError("Expected one NDP-08 tool table with at least one data row")

    header, separator, *rows = tables[0]
    columns = table_cells(header)
    if columns[-1] != EVIDENCE_COLUMN or table_cells(separator) != ["---"] * len(columns):
        raise ValueError("Expected a tool table header with an evidence column and separator")

    for row in rows:
        cells = table_cells(row)
        if len(cells) != len(columns) or not all(cells):
            raise ValueError(f"Tool table row has missing or extra cells: {row}")
        entry = cells[0]
        for reference in cells[-1].split("<br>"):
            match = SOURCE_REFERENCE.fullmatch(reference)
            if match is None:
                raise ValueError(f"{entry}: malformed source evidence: {reference!r}")
            path, identifiers = match.groups()
            for identifier in re.findall(r"`([^`]+)`", identifiers):
                yield entry, path, identifier


def check_tool_table(document):
    for entry, path, identifier in tool_references(document):
        relative = Path(path)
        source = (REPO_ROOT / relative).resolve()
        if relative.is_absolute() or ".." in relative.parts or not source.is_relative_to(REPO_ROOT):
            raise ValueError(f"{entry}: expected a repo-relative source path: {path}")
        if not source.is_file():
            raise ValueError(f"{entry}: source file does not exist: {path}")

        # A renamed symbol or flag can leave the old name as a substring.
        token = rf"(?<![\w-]){re.escape(identifier)}(?![\w-])"
        if not re.search(token, source.read_text(encoding="utf-8")):
            raise ValueError(f"{entry}: {identifier!r} was not found in {path}")


class ToolTableTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.document = AUDIT_PATH.read_text(encoding="utf-8")

    def test_all_rows_have_current_source_evidence(self):
        check_tool_table(self.document)

    def test_unknown_identifier_is_rejected(self):
        # Change the last row, using a substring of the original Rust identifier.
        bad_document = self.document.replace("`build_signed`", "`build_signe`", 1)
        with self.assertRaisesRegex(ValueError, r"'build_signe' was not found in .*main\.rs"):
            check_tool_table(bad_document)

    def test_unknown_flag_is_rejected(self):
        bad_document = self.document.replace(
            "`execute_doctor`, `--allow-missing`", "`execute_doctor`, `--allow`", 1
        )
        with self.assertRaisesRegex(ValueError, r"'--allow' was not found in .*commands\.rs"):
            check_tool_table(bad_document)

    def test_missing_file_is_rejected(self):
        bad_document = self.document.replace(
            "`tools/nagi-cli/src/commands.rs` (`execute_doctor`",
            "`tools/nagi-cli/src/missing.rs` (`execute_doctor`",
            1,
        )
        with self.assertRaisesRegex(ValueError, r"source file does not exist: .*missing\.rs"):
            check_tool_table(bad_document)

    def test_file_without_identifier_is_rejected(self):
        bad_document = self.document.replace("`nagi` (`--locked`)<br>", "`nagi`<br>", 1)
        with self.assertRaisesRegex(ValueError, r"malformed source evidence: '`nagi`'"):
            check_tool_table(bad_document)
