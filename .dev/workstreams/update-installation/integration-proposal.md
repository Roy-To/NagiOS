# UPD-01 integration proposal

This workstream is intentionally unregistered because `.dev/workstreams.json`,
`.dev/schemas/**`, root `Cargo.toml`/`Cargo.lock`, and `.github/workflows/**`
are Integration Owner paths. `registration-proposal.json` contains the
requested DF-01 row. The dedicated state stays in
`.dev/workstreams/update-installation/state.json` until that row is accepted.

After review, the Integration Owner should:

1. Add the `update-installation` registry row using the proposal's exact owner,
   branch, dependencies, path allowlist, activation gate, and merge boundary.
2. Add `tools/nagi-installer` to root workspace membership and reconcile its
   package-local lock against the root lock without changing the standalone
   workstream commit.
3. Add focused format, warnings-denied Clippy, build/check, and offline test
   commands to Ubuntu and Windows host jobs. Keep the POSIX atomic rename and
   Windows `MoveFileExW` code paths covered by their native test hosts when
   available.
4. Preserve the public `AppSdkManifestAdapter` convergence point and do not
   replace it with a duplicate manifest format.
5. Keep M16 guest VFS/package replacement and all runtime adoption out of this
   foundation until the release-line activation gate is met.

No `.dev/workstreams.json`, schema, root Cargo, CI, guest package service, or
0.1 status file is changed by UPD-01.
