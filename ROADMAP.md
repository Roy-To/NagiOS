# Nagi OS 0.1 Roadmap

This roadmap summarizes the ordered acceptance work in the
[Nagi OS 0.1 implementation specification](docs/Nagi_OS_0.1_Codex_Implementation_Spec.md).
It describes target scope, not a claim that a feature is present or a release
schedule. The current state and evidence are maintained in
[`docs/implementation_status.md`](docs/implementation_status.md) and the
owning `docs/workstreams/` files.

## Milestone sequence

| Range | Intended outcome |
| --- | --- |
| M0–M4 | Reproducible host tooling, UEFI/kernel boot, memory, scheduling, handles, and IPC foundations. |
| M5–M13 | User processes and supervision, persistent storage, shell, display/input, desktop, permissions, networking, and POSIX user-space compatibility. |
| M14–M18 | Audio, History/Wayback foundation, package and SDK path, then Servo-based Albert browser. |
| M19–M22 | Stable-object search and Workspace, local Granite runtime, validated AI actions, and transaction/undo safety. |
| M23–M26 | Nagi Bar and constrained app/browser context, semantic search, voice, and typed model/provider routing. |
| M27–M28 | A/B boot and Recovery Environment, followed by reference-machine integration and stress acceptance. |
| M29–M30 | Developer Preview documentation and polish, then reproducible release artifact assembly and verification. |

Milestones remain ordered from M0 through M30. A later task may proceed on
work independent of an earlier blocker, but acceptance that depends on the
blocked capability stays open. A `PARTIAL` implementation or a host-only test
does not satisfy guest acceptance.

## M19–M30 focus

The current continuation focuses on these gates:

- **M19–M22:** connect stable Object IDs and search to real producers and
  authenticated services; run Granite in Nagi; connect plans to authorized
  actions; and tie file changes to the production transaction, Activity
  Ledger, and Undo path.
- **M23–M26:** provide trusted current-app, selected-object, Workspace, and
  Albert context; add real embedding and speech providers; and verify model
  routing with the required local artifacts and fallback behavior.
- **M27–M28:** prove failed-slot rollback and bootable recovery, then exercise
  the reference workload while measuring memory, handles, audio, and service
  behavior.
- **M29:** finish onboarding, accurate diagnostics, screenshots, sample-app
  guidance, package metadata, notices, and clean-build polish without
  describing unfinished functionality as available.
- **M30:** create `Nagi-OS-0.1-devpreview.qcow2`, `SHA256SUMS`, source revision
  and build manifest, license notices, architecture/SDK/contribution/roadmap
  documentation, and reproducibility evidence.

Release readiness is determined by the release blockers and acceptance rules
in the primary specification, not by this summary or by artifact checksums
alone. See the current [M30 workstream](docs/workstreams/NagiOS_M30_Release_Workstream.md)
for release assembly evidence and remaining gates. No release date is promised
by this roadmap.

## Related guides

- [Developer onboarding and diagnostics](docs/developer-preview/README.md)
- [SDK surface and sample workflow](sdk/README.md)
- [Contribution workflow](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Third-party notices](THIRD_PARTY_NOTICES.md)
