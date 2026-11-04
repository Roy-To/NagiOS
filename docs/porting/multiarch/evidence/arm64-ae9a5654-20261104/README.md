# ARM64 runtime follow-up, 2026-11-04

This run uses source `ae9a56544d2c66cbbf6e641b80cd144ed52314a4` and tree
`f21070e7e24e9a4aad6a0ba6b562076b1a575e35`, descending from public main
`0362427747e769b00558053e4c7f4c756d303802`. The source candidate was published
unchanged to PR #57 with its existing ancestry. This is a later execution
checkpoint than the FE52 static/host records, which remain unchanged snapshots.

Two fresh offline builds using pinned nightly-2025-08-01 produced the same
AArch64 ELF SHA-256 `47a0f88035087945ac27f763d7ff43ba2a13afc4da6677ebbef626fa35c5cc3b`.
QEMU 11.1.1 TCG ran the first fresh ELF with virt/GICv2, cortex-a53, one CPU and
128 MiB RAM. EL1 serial markers, SVC/vector/ERET and counter progression passed
(exit 0); the real EL2 negative run rejected the exception level (exit 1,
negative condition MET). This is an independent freestanding experiment.

The entry point is not fully passing: its unpinned preflight selected an older
Homebrew compiler (exit 101). Explicitly selecting the pinned tools allowed the
preflight and generator to run, but verify.sh then failed with exit 2 because
it omits the receipt verifier's mandatory source commit/tree arguments.
The separate receipt verifier with those full SHAs exited 0. Both failures are
retained in summary.json and private raw logs. The existing script still needs
a bounded fix to pin its preflight tools and forward both SHA arguments.

For independent reproduction, check out the exact source and run through the
SSD guard. In the spike directory, explicitly select the pinned toolchain:

```sh
toolchain_bin=$(dirname "$(rustup which rustc --toolchain nightly-2025-08-01)")
export PATH="$toolchain_bin:$PATH"
export RUSTUP_TOOLCHAIN=nightly-2025-08-01
export RUSTC="$toolchain_bin/rustc"
export RUSTDOC="$toolchain_bin/rustdoc"
export RUSTFMT="$toolchain_bin/rustfmt"
cargo fmt -- --check
cargo clippy --release --locked --offline -- -D warnings
python3 -m unittest test_runner test_attestation -v
# Choose a new, absolute SSD directory outside the checkout; it must not exist.
output="$NAGI_STORAGE_ROOT/private-evidence/NEW-UNIQUE-ARM64-RUN"
python3 attest.py --source-commit ae9a56544d2c66cbbf6e641b80cd144ed52314a4 \
  --source-tree f21070e7e24e9a4aad6a0ba6b562076b1a575e35 \
  --main 0362427747e769b00558053e4c7f4c756d303802 --output "$output"
python3 verify_attestation.py --attestation "$output/attestation.json" \
  --source-commit ae9a56544d2c66cbbf6e641b80cd144ed52314a4 \
  --source-tree f21070e7e24e9a4aad6a0ba6b562076b1a575e35 \
  --main 0362427747e769b00558053e4c7f4c756d303802
```

The complete unsigned receipt, exact invocations, source archive, both ELFs and
raw build/runner logs remain on the approved SSD at the location recorded in
[summary.json](summary.json). Public serial logs are unmodified copies.
Hashes bind artifacts but do not prove independent execution. Phase C remains
PENDING independent rebuild/review. No physical ARM target was identified in
the available local inventory, and no physical boot, media write, network
probe, Nagi ARM64 production runtime, or hardware driver acceptance occurred.
D/E, shared registration and ABI/BootInfo changes retain their M30/owner gates.
