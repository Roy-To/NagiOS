# Linux service broker candidate source preservation

Status: source preservation only. These files are not declared as Rust modules and are not wired into normal init.

`stage25/` preserves the selected Stage 25 broker, peer, NIPC, process-instance, and runtime-fixture source. `task24-c3-source/` preserves the two supplied C3 lifecycle sources and their three overlays. The C3 overlay has not been integrated or verified in this lane.

The supplied candidate reports a Stage 25 Linux guest 11 run and a real clone3/pidfd fixture success. Those earlier results were not rerun here and are not acceptance evidence for this commit.

The candidate Cargo manifests and locks were excluded because they contain local absolute dependency paths. Generated target/cache trees, ELF/cpio/disk/log outputs, and the private review document were also excluded.

No build, type check, or test was run for this snapshot. The Notes codec/client is not included in this preservation commit. No broker or Notes provider is enabled by these files; the existing init fail-closed `Unavailable` path remains in place.
