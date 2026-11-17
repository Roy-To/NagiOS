# Linux service broker candidate source preservation

Status: source preservation only. Nothing in this candidate is wired into the maintained Linux init path.

`stage25/` preserves the selected Stage 25 broker, peer, NIPC, process-instance, and runtime-fixture source. The Task41 owner candidate includes the C3 cleanup-request state and method. `task24-c3-source/` separately preserves the two supplied C3 lifecycle sources and their three overlays. This lane has not verified that source against the current Task24 contract or integrated it into maintained init; the preserved candidate's original sibling-module layout is also not reconstructed here.

The supplied candidate reports a Stage 25 Linux guest 11 run and a real clone3/pidfd fixture success. Those earlier results were not rerun here and are not acceptance evidence for this commit.

The candidate Cargo manifests and locks were excluded because they contain local absolute dependency paths. Generated target/cache trees, ELF/cpio/disk/log outputs, and the private review document were also excluded.

No build, type check, or test was run for this snapshot. The Notes codec/client is not included in this preservation commit. No broker or Notes provider is enabled by these files; the existing init fail-closed `Unavailable` path remains in place.
