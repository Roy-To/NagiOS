# app_windows — PROVISIONAL app fixture (NDP-A018)

A small, self-made, Windows-flavoured "project" for the migration report. It is a
**pair of existing evaluation inputs**, not a copy and not a new third-party source:

| Part | File | Read by |
|---|---|---|
| API (Windows C) | `../windows/read_prefix.c` with the handwritten snapshot `../../api/windows/read_prefix.report-v1.json` | `windows_diagnosis` |
| UI (WPF XAML) | `../ui/wpf_task_panel.xaml` | `ui_analysis`, `ui_mapping` |

`tools/nagi-developer-platform/validation/report.py --fixture app_windows` routes both
parts into one report (API gaps, UI row and per-axis gaps, scaffold files, Stage 1/2/3
status). The pairing lives in `report.py` (`APP_FIXTURE`, `UI_FIXTURE_PATHS`,
`api_case_paths`); this directory only documents it, so the two inputs have a single
copy and the existing drift test keeps covering the C snapshot.

Limits: static host reading only. The C file and the XAML are never compiled, loaded or
run. There is no approved Stage 2 target for this fixture (`stage_2` is `unsupported`),
and Stage 3 (Nagi target) stays `not_run`. The XAML `x:Class`, `Click` handler and
bindings are not resolved. See `validation/README.md` (PROVISIONAL) for assumptions.
