# Development

Use the repository-level `nagi` command for build, test, image, run, and host
diagnostic workflows. Tool versions are pinned in `rust-toolchain.toml` and
`nagi.toml`.

For Nagi 0.2 preparation, `./nagi dev status|resume|verify` reads the
workstream-local state and registry under `.dev/`; `./nagi dev diagnose`
inventories retained logs and hashes explicitly named artifacts. The 0.2
workflow and release-line gate are defined in
[`docs/0.2/DEVELOPMENT_ARCHITECTURE.md`](../0.2/DEVELOPMENT_ARCHITECTURE.md).
