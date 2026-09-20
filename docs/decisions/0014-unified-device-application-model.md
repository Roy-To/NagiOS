# ADR 0014: Unified Device and Application Model

- Status: Accepted
- Date: 2026-09-18
- Applies after: M13 PASS and before M14/M15 implementation

## Context

Nagi must not freeze its package, SDK, history, workspace, and AI-context
model around a desktop-only assumption. A future phone, tablet, laptop, or
other Nagi-capable endpoint must be another execution and presentation node in
the same user-owned environment, not a separate Nagi product or application
identity.

M0-M13 remain valid evidence for the single-node QEMU x86-64 Developer
Preview. This ADR changes the forward-facing model and terminology; it does
not require a mobile port, distributed execution, cloud sync, or a rewrite of
passing kernel, IPC, networking, or Window Server code.

## Decision

Nagi has one conceptual environment containing applications, logical
application state, user objects, workspaces, history, policy, AI context, and a
Device Registry. A device is a capability-bearing `Node`, identified by a
stable `NodeId`. Device-specific behavior is selected from advertised
capabilities and presentation context, not from separate desktop/mobile app
identities.

The stable concepts are distinct:

`UserId`, `NodeId`, `AppId`, `AppSessionId`, `ExecutionInstanceId`,
`SurfaceId`, `WorkspaceId`, `ObjectId`, and `TransactionId` must not be
collapsed into a PID, window ID, path, inode, or device-specific package ID.

An `AppId` identifies one logical application package. An `AppSessionId`
identifies a continuable logical session and its state. An execution instance
is the concrete process/runtime currently executing that session on a Node.
One session may eventually have zero or more execution instances.

A `PresentationSurface` is the visual/interactive attachment of a session to
a Node and capability context. A desktop `Window` is one presentation
primitive, not the universal application root. Nagi 0.1 may keep the current
one-session/one-local-execution/one-desktop-surface mapping in practice, but
the model must not make that mapping mandatory for later SDKs.

The Device Registry is a user-space service concept, provisionally
`device@1`, exposing a bounded `DeviceDescriptor` for the local reference
Node in 0.1. Its future fields include architecture, display, input, audio,
sensors, compute, storage, connectivity, and power capabilities. The wire
format follows normal Nagi IDL/versioning rules when the service is
implemented.

Future cross-node routing, pairing, authentication, synchronization, and
conflict handling remain explicit user-space services. Kernel channels remain
local IPC primitives and are not transparently turned into network IPC. A
remote Node receives no authority merely by joining the same environment;
capabilities remain explicit, attenuated, and locally policy-checked.

## Consequences

- Durable application data, session state, presentation-local state, and
  execution-local state are separate categories.
- M15 history/transaction records may include `NodeId`, `AppId`,
  `AppSessionId`, optional `SurfaceId`/`WorkspaceId`, and `ObjectId` values;
  history is not keyed only by PID, window, or path.
- M16 packages and SDKs use one `AppId`, logical sessions, adaptive
  presentation contexts, and surface attachment rather than desktop/mobile
  package identities.
- M19 workspaces reference logical application sessions and objects and are
  not owned by one Node; layout is presentation state.
- M22/M23 AI activity and context identify logical app/session/object/workspace
  state independently of the Node where a request originated.
- The Window Server remains the valid Nagi 0.1 desktop presentation path and
  is not deleted or rewritten by this checkpoint.
- 0.1 remains one local QEMU x86-64 Node. No ARM, mobile hardware, cloud
  sync, remote transport, migration, or fake multi-device behavior is added.

## Alternatives rejected

1. Separate Nagi Desktop and Nagi Mobile products with synchronization.
2. One codebase with separate desktop/mobile application identities.
3. One identical UI for every screen and input type.
4. Transparent networked kernel IPC.
