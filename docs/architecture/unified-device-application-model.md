# Unified Device and Application Model

This is the architecture reference for the post-M13 model. The primary
implementation specification and ADR 0014 are authoritative; this document
keeps the relationships and terminology easy to find for SDK, service, and
milestone work.

## One environment, many possible Nodes

```text
Nagi Environment
├── identity, policy, applications, objects, workspaces, history, AI context
├── Application Package (AppId)
│   └── Application Session (AppSessionId)
│       ├── logical state and object/workspace/transaction references
│       ├── Execution Instance(s) (ExecutionInstanceId, NodeId)
│       └── Presentation Surface(s) (SurfaceId, NodeId, capabilities)
└── Device Registry (device@1)
    └── Node(s) (NodeId, capability descriptors)
```

Nagi 0.1 has one practical Node: the QEMU x86-64 reference machine. A Node
is an execution, display, input, sensor, storage, or compute endpoint with
capabilities; it is not a separate Nagi OS edition.

## Identity model

| Identity | Meaning | Must not be substituted by |
|---|---|---|
| `UserId` | user/environment identity | process or device ID |
| `NodeId` | stable Nagi-capable endpoint | architecture or window ID |
| `AppId` | logical installed application identity | desktop/mobile package IDs |
| `AppSessionId` | continuable logical application session | PID or Window ID |
| `ExecutionInstanceId` | concrete runtime on a Node | AppSessionId |
| `SurfaceId` | visual/interactive session attachment | Window ID as app identity |
| `WorkspaceId` | semantic grouping of objects/sessions | node-local layout ID |
| `ObjectId` | stable user-visible object identity | path or inode |
| `TransactionId` | meaningful action/history identity | timestamp or PID |

An application session is logically separate from the process executing it. A
Presentation Surface is logically separate from a desktop Window. A Window is
one desktop surface primitive used by the Nagi 0.1 Window Server.

## Presentation and state

Presentation context is derived from capabilities such as logical dimensions,
DPI, touch, pointer, keyboard, pen, orientation, and safe regions. Use
adaptive classes such as `Compact`, `Medium`, and `Expanded`; do not branch
the application identity on a fixed device category.

Application state is classified as:

- durable data: documents, projects, and persistent settings;
- session state: open `ObjectId`s, navigation, tasks, and workspace relation;
- presentation-local state: window/layout/scroll/hover state for a surface;
- execution-local state: caches, temporary buffers, and runtime/GPU state.

Presentation-local and execution-local state do not silently become canonical
logical application state.

## Device Registry and authority

The future user-space `device@1` service may describe the local Node in 0.1.
Its descriptor is bounded and capability-oriented: display, input, audio,
sensors, compute, storage, connectivity, and power. It must not become a
large ontology before a consumer requires it.

Remote Nodes do not inherit authority from environment membership. Handles and
capabilities remain explicit and attenuated, authorization remains local to
the trusted Node, and AI cannot use a future link service to bypass local
policy. Cross-node transport, pairing, encryption, synchronization, and
conflict handling are later user-space work, not kernel IPC changes.

## Forward milestone rules

- M15 history records can carry Node/App/Session/Surface/Workspace context.
- M16 defines one package/AppId, session and surface APIs, adaptive
  presentation, and multi-architecture payloads without multiple app IDs.
- M19 makes Workspace references device-independent and object/session based.
- M22/M23 keep AI activity/context tied to logical app/session/object/workspace
  identities, while recording the originating Node where relevant.
