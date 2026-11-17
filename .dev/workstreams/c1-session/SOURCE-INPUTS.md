# C1 Session source inputs

This directory preserves source-only inputs from the fixed C1 runtime/shared and nonblocking-live snapshots, plus their declared local path-dependency closure. The snapshots are kept as reviewable inputs; they have not been integrated into the workspace implementation by this preservation commit.

- Runtime/shared snapshot: `ce0ec0bcea61aa3c0c958413e6c38e1ac1a7de9c`.
- Nonblocking-live snapshot: `2239981947d5398a62bbf461dca94a28968d43ee`; its SDK and root source trees match the runtime/shared snapshot.
- Dependency closure snapshots: identity input `108df4c1709875e6c3bb5329c18ebb45310840c6` and service-contract input `f53f11f3c94bb6fd39563e9141f12ca5bde991c7`.

The source bundle excluded Git history, build outputs, credentials and account settings. Test-only fixture credential values were replaced with `<REDACTED_TEST_CREDENTIAL>` in two test files. Those altered test files were not executed, and no test result from their source checkouts is claimed here. This preservation does not establish Host or Linux Target acceptance.

The `nagi-capability/src/model.rs` and `nagi-abi/Cargo.toml` copies each had one extra blank line at EOF; those whitespace-only endings were normalized for a clean diff check.
