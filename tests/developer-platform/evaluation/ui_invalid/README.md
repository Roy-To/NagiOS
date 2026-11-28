# ui_invalid — PROVISIONAL fixture (NDP-A021)

One self-made, four-line UI file (`panel_layout.qml`) written as QML-like text, which is
not XML. Provenance: written for this task from scratch; it is not copied from any
third-party project, is not derived from Qt's QML documentation samples, and contains no
build script. Permitted use: only as static input of the migration report fixture
`ui_invalid` in this repository (reading its bytes on the host); it must not be loaded by
Qt/QML, compiled, rendered or executed.

`tools/nagi-developer-platform/validation/report.py --fixture ui_invalid` routes it to
`ui_analysis` only through `UI_FIXTURE_PATHS`. The existing analyzer parses the file as XML
and answers `invalid_xml`, so the report shows `ui_analysis` as `failed` (`file_failed`;
the file entry carries `invalid_xml`), `ui_mapping` as `not_run`
(`ui_analysis_did_not_pass`), and the independent checks (generation) still run. The CLI
exits 1 because a check failed.

Compare with [`ui_unsupported/`](../ui_unsupported/README.md): well-formed XML in an
unknown format is `unsupported` (exit 0); text that is not XML at all is `failed` (exit 1).
The analyzer chooses the format from the parsed XML root, not from the file extension.

Limits: static host reading only; nothing is rendered or run, and no QML support is added.
Not Nagi target acceptance.
