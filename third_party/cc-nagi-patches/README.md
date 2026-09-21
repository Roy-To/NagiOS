# Nagi `cc` patch boundary

This directory contains the reproducible Nagi-owned patch set for the pinned
`cc` 1.4.6 registry source. The Nagi target is an independent OS target, so
`cc-rs` must not infer or emit a host C++ standard-library link. C++ objects
remain compiled by the target toolchain; ownership of the target C++ runtime
is reserved for the Nagi runtime/toolchain integration.

The patch is intentionally target-specific (`target.os == "nagi"`) and does
not alter behavior for host, Windows, Apple, BSD, Android, WASI, or other
targets.
