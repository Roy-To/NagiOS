# app_linux — PROVISIONAL app fixture (NDP-A020)

A small, self-made, Linux-flavoured "project" for the migration report; the Linux
counterpart of `../app_windows`. It is a **pair of existing evaluation inputs**, not a copy
and not a new third-party source:

| Part | File | Read by |
|---|---|---|
| API (Linux C) | `../linux/watch_changes.c` with the handwritten snapshot `../../api/linux/watch_changes.report-v1.json` | `linux_diagnosis` |
| UI (GTK Builder XML) | `../ui/gtk3_task_panel.ui` | `ui_analysis`, `ui_mapping` |

`tools/nagi-developer-platform/validation/report.py --fixture app_linux` routes both parts
into one report (API gaps, UI row and per-axis gaps, scaffold files, Stage 1/2/3 status).
The pairing lives in `report.py` (`APP_LINUX_FIXTURE`, `UI_FIXTURE_PATHS`,
`APP_API_FIXTURES`, `api_case_paths`); this directory only documents it, so the two inputs
have a single copy and the existing drift test keeps covering the C snapshot. The API
diagnosis is the same `watch_changes` report-v1 snapshot the `linux` fixture uses; it is
not implemented a second time.

Limits (PROVISIONAL): self-made fixture, host-side static reading only. The C file and
the `.ui` file are never compiled, loaded or run, and no real scanner is connected: the
API evidence is the handwritten snapshot, not scanner output. The GTK Builder signal
handlers and bindings are not resolved against the C source. There is no approved Stage 2
target for this fixture (`stage_2` is `unsupported`), and Stage 3 (Nagi target) stays
`not_run`. A host result here is not Nagi target acceptance. The name and the routing may be
replaced by real project inputs later. See `validation/README.md` (PROVISIONAL).
