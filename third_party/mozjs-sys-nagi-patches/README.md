# Nagi mozjs_sys patches

These ordered patches adapt the pinned `mozjs_sys 153.0.0-2` source build to
Nagi's freestanding target toolchain. The Mozilla configure triplet is only a
build-system identifier; the Rust target, compiler wrapper, relibc headers,
and final guest link remain Nagi-owned. The configure-only triplet is the
Nagi-owned `x86_64-unknown-nagi` form; the ordered adapter patch teaches the
pinned Mozilla configure layer to recognize it without relabeling Nagi as
Linux/WASI or using a host runtime.

The generated source is materialized from `third_party/sources.lock` and is
never edited in the Cargo cache. Each patch is checked and applied before the
source fingerprint is recorded.
