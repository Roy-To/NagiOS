# ui_unsupported — PROVISIONAL fixture (NDP-A019)

One self-made, three-element UI layout file (`panel_layout.xml`, root `PanelLayout`)
in a format that is none of Qt Designer, GTK Builder or XAML. It is not copied from any
third-party project and contains no build script.

`tools/nagi-developer-platform/validation/report.py --fixture ui_unsupported` routes it
to `ui_analysis` only through `UI_FIXTURE_PATHS`. The existing analyzer answers
`unsupported_format`, so the report shows `ui_analysis` as `unsupported`, `ui_mapping` as
`not_run` (`ui_analysis_did_not_pass`), and the independent checks (generation) still run.

Why XML and not QML-like text: the analyzer selects the format from the parsed XML
root, not from the file extension. A non-XML text file would be `invalid_xml` (a
`failed` check), not `unsupported_format`. See `validation/README.md` (PROVISIONAL).

Limits: static host reading only; nothing is rendered or run, and no new format is
implemented. Not Nagi target acceptance.
