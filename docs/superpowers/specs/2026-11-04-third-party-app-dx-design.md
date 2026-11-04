# Third-Party App DX: Host-Only Design

## Goal

Let an external developer build the existing Rust `Hello Nagi` sample out of tree using the public Nagi SDK, and validate its app manifest with the existing host package tool.

## Current constraints

- Nagi OS 0.1 M30 is still `PARTIAL`; the 0.2 activation checkpoint is not open.
- The existing `app-sdk-contract` `PASS` is host-side contract acceptance only. It does not mean the target runtime or guest installation passed.
- Work is limited to host-side sample build, manifest validation, documentation, and evidence. Do not integrate runtime behavior, install or launch a guest app, add authority behavior, or claim full M21 acceptance.
- The SDK is not published to crates.io (`publish = false`). The sample will use a fixed public Git revision of `Roy-To/NagiOS` and depend directly only on `nagi-sdk`.
- Keep changes within the registered `third-party-app-dx` workstream paths. Shared registry and root integration files are owned by the Integration Owner.

## Design

Reuse `samples/hello-nagi` as the external sample instead of creating another app. Keep it as a standalone Cargo workspace. Change its direct SDK dependency from a relative path to the public repository at the audited main revision, so the sample can build without a NagiOS checkout. Add a third-party app manifest that is valid under the existing manifest schema and deliberately declares no capabilities or publisher identity.

Extend the existing `nagi-pkg` contract tests to exercise the third-party manifest through the production host validator. The test must assert successful validation and key third-party properties. Do not add a second validator or alter runtime/package installation behavior.

Document English and Japanese quick starts with the pinned revision, build, dependency-source inspection, and manifest-validation commands. State clearly that manifest validation does not install or launch an app and that runtime acceptance remains gated.

## Acceptance evidence

1. The sample builds out of tree from the public Git SDK dependency.
2. Its direct dependency list uses only `nagi-sdk`; `cargo tree` identifies the pinned public repository revision.
3. The manifest passes `nagi-pkg manifest validate` and a focused contract regression test.
4. Documentation describes both languages and accurately distinguishes host validation from target runtime acceptance.
5. Workstream state remains host-only and `PARTIAL`; neither M30 nor M21 is marked complete.
