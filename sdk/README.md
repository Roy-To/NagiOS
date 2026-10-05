# Nagi SDK

The checked-in SDK is an early Nagi 0.1 Developer Preview surface, not a
complete application framework. Its current Rust crate is `nagi-sdk` in
[`sdk/rust`](rust/); the C header and implementation are in
[`sdk/c`](c/). The current sample is [`samples/hello-nagi`](../samples/hello-nagi/).

## Current API surface

The Rust crate is `no_std`. It re-exports the shared application and logical
identity types (`AppId`, `AppSessionId`, `NodeId`, `SurfaceId`, and related
IDs), plus `Application` and presentation helpers. The C surface currently
provides `nagi_application_open` and `nagi_application_surface` for the same
application/session and presentation concepts.

The generated Rust and C bindings come from
[`idl/application.nidl`](../idl/application.nidl). Files marked generated,
including `sdk/rust/src/generated.rs` and the C header/source, should be
updated through the IDL generator rather than edited by hand.

Example from the sample:

```rust
use nagi_sdk::{
    AppSessionId, Application, PresentationContext, PresentationSurface, HELLO_APP_ID,
};

pub fn compact_surface() -> PresentationSurface {
    let app = Application::new(HELLO_APP_ID, AppSessionId(1));
    app.presentation(PresentationContext::compact(320, 200))
}
```

The `Application` helper constructs logical identity and presentation data; it
does not itself start a process or open a Window Server connection.
The C functions validate their pointers and nonzero dimensions, then fill the
presentation IDs; they do not make an IPC call or grant authority.

## Build and package the sample

From the repository root, the current CI-verified sample artifact flow is:

```sh
cargo build --manifest-path samples/hello-nagi/Cargo.toml --offline --locked
cargo run --manifest-path samples/hello-nagi/Cargo.toml --bin hello-nagi-package \
  --offline --locked -- out/artifacts/hello-nagi.napp
cargo run --manifest-path tools/nagi-pkg/Cargo.toml --offline --locked -- \
  build-hello out/artifacts/hello-nagi.napp out/artifacts/hello-nagi.xapp
```

The package tool also exposes `build-signed-hello` and `info`; its signing key
is a fixed host-side test key and is not a production signing workflow. The
`./nagi m16` acceptance command stages the sample package in a QEMU image and
checks guest package install, list/info, launch, atomic update, and removal.
That single sample acceptance is not a general installer or a claim that all
SDK capabilities in the primary specification exist.

## Not yet provided by this SDK surface

The current public files do not define general file-handle, IPC, network,
audio, clipboard, notification, or Action-registration APIs, nor a complete
UI toolkit or application lifecycle client. Do not infer those interfaces
from the broader SDK plan in the implementation specification. Check the
current milestone evidence in [`docs/implementation_status.md`](../docs/implementation_status.md)
and the relevant workstream before relying on any additional API.

Application identity and presentation follow the
[unified device/application model](../docs/architecture/unified-device-application-model.md).
Packages and service calls remain subject to Nagi's capability and permission
boundaries; an SDK helper does not grant authority. See
[ADR 0011](../docs/decisions/0011-m11-user-space-permission-broker.md),
[ADR 0016](../docs/decisions/0016-package-signature-and-staging.md), and the
[primary implementation specification](../docs/Nagi_OS_0.1_Codex_Implementation_Spec.md).

The project has not selected an overall Nagi OS license. Review the root
[README](../README.md) and [third-party notices](../THIRD_PARTY_NOTICES.md)
before redistributing artifacts; the sample package's test metadata does not
set a license for the repository.
