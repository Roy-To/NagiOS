# PORT-MULTIARCH-01 handoff — provisional, unregistered

Base: `379d19eadb6f8792c27b9bc3f147e1ea48eac4a2`.
Verified published candidate: `6cbfb452ee83a7c0011036b6d0ed836625f1f7ae`. The next commit records evidence/state only;
its HEAD intentionally differs from the tested source SHA (no circular claim).
Push destination: `RT-NISH/NagiOS:codex/port-multiarch-foundation-01`.

A audit PASS; B proposal/host foundation PASS; C independent spike PASS.
D/E BLOCKED. Registration remains an unapplied proposal. M30 remains PARTIAL;
no runtime gate or owner status has been promoted. The original worktree is
untouched. All changes are confined to `docs/porting/multiarch/**` and two new
`experiments/` packages. No production kernel/loader/user/ABI/target/root Cargo,
CI, accepted ADR, shared registry or implementation_status file changed.

Verified: 99 host tests, 16 BootInfo tests, 5 ABI tests, 3 runner negative tests;
focused format/Clippy, ARM64 no_std contract compile; exact source/authority
identity; unchanged registry validates. Imports allow only named pre-existing
style lints, so this is not full production Clippy acceptance. dev status/resume
refuse the unregistered branch as expected; their original exit was not captured
separately and no numerical exit claim is made.

QEMU 10.0.13 TCG: virt/GICv2, cortex-a53, one CPU, 128 MiB, no display/network.
Actual EL1 UART, SVC/vector/ERET and generic counter progress; exit 0. Actual EL2
negative boot prints failure and exits 1. Counter progress does not prove timer
IRQ or GIC. MMU disabled; Nagi services/EL0/SMP/physical hardware NOT_RUN.
Independent ELF SHA-256: `47a0f88035087945ac27f763d7ff43ba2a13afc4da6677ebbef626fa35c5cc3b`; separate output-directory rebuild
has the identical digest. Full logs, commands and digests: [validation evidence](validation-evidence.json).

Top blockers: x86 entry/fault tables, MMU/address/TLS and syscall register ABI,
four-CPU APIC/ACPI startup and low-memory bounds, ELF machine 62, PCI/DMA/cache
transport, BootInfo v5 required firmware/display and signed A/B callbacks,
reference surface and future power/input. See [matrix](portability-matrix.md)
and [proposal](architecture-proposal.md).

Next: Integration Owner reviews [registration](registration-proposal.json),
proposal and migration acceptance. Production step 1 requires M30 PASS and
bounded os-core authorization naming exact base/paths, contract and x86 guest
parity/rollback. No merge or further production work is authorized. The
independent checkpoint has no remaining owned Acceptance item.

Publication note: Git transport returned 403 for host identity RyomaTOZAWA.
Repository-owner RT-NISH's connected Git Data API published all 76 files with
matching Git blob hashes and exact local tree aa7d52418f2332ab3b902c13a934d01ab70c2012.
The published candidate above was fetched and reverified (host/QEMU/registry).
The final records commit is its child; local original commits remain recoverable
under refs/port-multiarch/local-verified. Raw log whitespace is intentionally
retained with an evidence-only Git attribute; no source/style acceptance is waived.
