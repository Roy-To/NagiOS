# Nagi mozjs_sys patches

These ordered patches adapt the pinned `mozjs_sys 153.0.0-2` source build to
Nagi's freestanding target toolchain. The Mozilla configure triplet is only a
build-system identifier; the Rust target, compiler wrapper, relibc headers,
and final guest link remain Nagi-owned. The configure-only triplet uses the
recognized `x86_64-unknown-none` bare-metal form; it does not relabel the
Nagi Rust target as Linux or use a host runtime.

The generated source is materialized from `third_party/sources.lock` and is
never edited in the Cargo cache. Each patch is checked and applied before the
source fingerprint is recorded.
