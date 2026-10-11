# Multi-architecture contract experiment (host only)

Standalone, unregistered PORT-MULTIARCH-01 foundation. Not a kernel backend,
canonical ABI or production workspace member. It grants no authority and
introduces no duplicate rights/ID/IPC implementation.

Prerequisites: repository-pinned nightly-2025-08-01 with rustfmt/Clippy, host
linker, Python 3. On the verified Debian 13 x86-64 host:

```sh
./experiments/nagi-multiarch-contracts/verify.sh
python3 experiments/nagi-multiarch-contracts/audit.py \
  --output docs/porting/multiarch/evidence/source-inventory.json --check
```

The local Cargo workspace has only path dependencies on unchanged nagi-abi and
nagi-bootinfo. Test-only `#[path]` modules read production source directly:
handles, IPC, user IPC, VMO, memory, scheduler, process exit, user/loader ELF,
ACPI and CPU tables. Existing tests exercise the actual mechanisms; privileged
CPU functions are not invoked by this host harness. BootInfo/ABI unit tests
are compiled separately with rustc from their actual library source. There is
no root Cargo resolution, production module extraction or guest execution.

`src/lib.rs` contains proposed semantic validation only: bounded unique CPU
IDs, versioned future firmware/headless policy, mapping request bounds/W^X and
explicit ELF machine selection. It implements neither firmware parsing nor
MMU isolation; CPU exception/syscall parity needs later target tests.

Imported modules have dead-code allowances because this focused harness does
not consume every API. IPC additionally allows baseline-only
`unnecessary_mut_passed`/`drop_non_drop`; loader ELF allows `derivable_impls`;
scheduler allows `new_without_default`. These are style lints in unchanged
production code, scoped on test module imports, not assertion or security
exceptions. Owned proposal code and the ARM spike retain warning-denied lint.
Do not present the harness lint result as full production-kernel Clippy PASS.

The audit inventories tracked sources and exact hashes, retaining matched
text and line numbers, including tests/comments. It is not a semantic proof
of exhaustive portability. The manually reviewed matrix and unknowns are in
`docs/porting/multiarch/`. A new baseline requires owner review and regeneration,
not relaxing a stale inventory check.
