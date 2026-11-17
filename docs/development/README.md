# Development

## Co-developer guides

- [AI-assisted co-developer onboarding (Japanese)](co-developer-onboarding-ja.md)
- [Allocation proposal and execution backlog (Japanese)](CO_DEVELOPER_ALLOCATION_PROPOSAL.md)
- [Claude / Codex co-developer starter kit (Japanese)](co-developer-starter-kit/00_START_HERE.md): founder handoff, setup, shared rules, twelve task briefs, implementation/review/resume prompts, and minimal templates. This kit does not itself activate workstreams or grant account permissions.

Use the repository-level `nagi` command for build, test, image, run, and host
diagnostic workflows. Tool versions are pinned in `rust-toolchain.toml` and
`nagi.toml`.

For Nagi 0.2 preparation, `./nagi dev status|resume|verify` reads the
workstream-local state and registry under `.dev/`; `./nagi dev diagnose`
inventories retained logs and hashes explicitly named artifacts. These
commands launch the standalone bootstrap workspace, so read-only development
state checks work in a clean checkout before `./nagi fetch` materializes the
ignored pinned third-party source directories required by the root Cargo
workspace. The 0.2 workflow and release-line gate are defined in
[`docs/0.2/DEVELOPMENT_ARCHITECTURE.md`](../0.2/DEVELOPMENT_ARCHITECTURE.md).

## Conversation history

- [Entire private history operation and verification (Japanese)](entire-private-history.md): dedicated private checkpoint storage, repository-scoped Actions disablement, upload boundaries, tested agents, and remaining validation.
