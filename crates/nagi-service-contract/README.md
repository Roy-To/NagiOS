# Nagi service contract foundation

`nagi-service-contract` is a host-side, transport-neutral contract and a
reference harness for system-service calls. It defines typed service and
operation IDs, exact contract-version resolution, bounded binary payload
boundaries, request/response identity, descriptors, provider/client traits,
authorization hooks, cooperative cancellation, and an in-process reference
transport.

This crate does not replace `user/libnagi`'s target service registry and
supervisor, kernel Channels, or the App SDK's NIPC v1 envelope codec. It does
not add a target transport, a new wire codec, permission storage, policy
implementation, or product service behavior. Its isolated Cargo workspace is
intentional; the root Cargo workspace and lockfile remain integration-owned.

Caller identity is not part of `RequestEnvelope`: each transport must derive
the principal from a trusted execution boundary. The in-process adapter takes
an explicitly configured principal for host tests. Request and response
payloads are each bounded to 1 MiB; oversized provider responses return a
structured error.

See [`docs/0.2/system-service-ipc-contract.md`](../../docs/0.2/system-service-ipc-contract.md)
for the complete contract and integration boundaries. Run the executable echo
example with:

```sh
cargo run --manifest-path crates/nagi-service-contract/Cargo.toml --example in_process_echo --locked
```
