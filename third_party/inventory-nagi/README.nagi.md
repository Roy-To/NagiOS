# Nagi inventory source

This directory vendors the exact `inventory` 0.3.24 crates.io source archive
and applies the Nagi target constructor patch recorded in
`../inventory-nagi-patches/0001-nagi-target-init-array.patch`.

- Archive: <https://static.crates.io/crates/inventory/inventory-0.3.24.crate>
- SHA-256: `a4f0c30c76f2f4ccee3fe55a2435f691ca00c0e4bd87abe4f4a851b1d4dac39b`
- License: MIT OR Apache-2.0 (upstream license files are retained)
- Patch: include `target_os = "nagi"` in inventory's ELF `.init_array`
  constructor target list.

To reproduce, extract the pinned archive into a clean directory, apply the
patch from that directory, then compare the result with this vendored tree.
Do not edit Cargo's registry cache; the workspace resolves this crate through
the root `[patch.crates-io]` entry.
