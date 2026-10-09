Title: Slides Core: bounded independent presentation host foundation

Implement a UI-independent standalone Slides model from first-party spec §59.2–59.7.
Canonical slide/object IDs stay stable across editing and reorder; duplicate takes
fresh IDs and remaps internal links. Expected-revision batches validate fully
before committing, including geometry, references, byte/count bounds and tombstones.
Native v1 preserves Unicode, metadata and revisions and rejects malformed/unknown
versions. Source revision detection is an injected read-only boundary.

Validation: package-scoped fmt, locked offline acceptance tests, warnings-denied
Clippy, DF-01 state JSON schema and owned-path audit. See exact code SHA/log in
`.dev/workstreams/slides-core-01/state.json`.

Host-only. Shared registry/CI/root Cargo/ABI and runtime are unchanged.
Registration proposal requires Integration Owner review. PDF/PPTX, rendering,
guest/AI/Activity/Wayback/provider execution remain deferred. Do not mark ready
or merge until registration/publication review. Target M30 gate is retained.
