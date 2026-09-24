# Nagi OS 0.1 Developer Preview
## Codex Implementation Specification

**Status:** Implementation baseline  
**Target:** Nagi OS 0.1 Developer Preview  
**Primary execution target:** QEMU x86-64 Reference VM  
**Default Standard LLM:** IBM Granite 4.2 3B  
**Browser engine:** Servo only  
**Kernel:** Nagi Kernel, not Linux-based  
**License strategy:** Open source OS; third-party/model licenses remain separate

---

# 0. Purpose of this document

This document is the implementation specification for **Nagi OS 0.1 Developer Preview**.

Codex should treat this file as the primary technical direction for the repository.

The goal is not to create a toy kernel or a Linux distribution with a custom shell. The goal is to create an independently booting operating system with:

- its own kernel;
- its own user-space service architecture;
- capability-based authority;
- a native GUI desktop;
- persistent storage;
- local networking;
- a Servo-based browser named **Albert**;
- local/offline AI;
- semantic search;
- voice interaction;
- transactions, undo, history, Wayback and recovery;
- an SDK and package system.

Nagi is intended to be usable by ordinary GUI/keyboard/mouse interaction even if the AI subsystem is disabled.

The AI is a first-class interaction mode, but **never a privileged kernel component**.

---

# 1. Non-negotiable product principles

## 1.1 Core philosophy

Nagi follows these principles:

> **Safe by default. Powerful by choice.**

> **Restrict software, not the owner.**

> **The human says what they want; Nagi understands intent.**

> **AI is powerful but untrusted.**

The owner of the machine must ultimately be able to:

- install unsigned software;
- enable Developer Mode;
- install experimental drivers;
- inspect low-level diagnostics;
- change security settings;
- enable kernel debugging;
- use custom model files.

These actions may require trusted authorization, but Nagi must not permanently lock the owner out of their own machine.

## 1.2 Offline-first

Core operation must not require:

- a cloud account;
- a cloud LLM;
- a cloud API key;
- remote telemetry;
- remote storage.

Offline operation must support:

- desktop;
- files;
- notes;
- calculator;
- terminal;
- semantic search;
- local AI;
- voice;
- Wayback/history.

Albert naturally requires a network connection for external web content.

## 1.3 Kernel determinism

Probabilistic AI logic must never live in kernel space.

The kernel manages:

- execution;
- memory;
- IPC;
- capabilities;
- processes;
- threads;
- timers;
- low-level device primitives.

The kernel does **not** interpret natural language, decide user intent, or make probabilistic security decisions.

---

# 2. Nagi 0.1 official target

Nagi 0.1 officially targets only the **Nagi Virtual Reference Machine**.

## 2.1 Reference VM

- QEMU
- x86-64
- UEFI / OVMF
- q35 machine
- 4 vCPU
- 8 GB RAM
- approximately 64 GB virtual disk
- VirtIO Block
- VirtIO Network
- VirtIO GPU
- VirtIO Sound
- VirtIO RNG
- fixed keyboard/mouse input devices

Physical PC support is explicitly **not** required for Nagi 0.1.

Future directions may include:

1. a selected x86-64 mini PC;
2. ARM64;
3. Raspberry Pi-class hardware.

Do not distort the 0.1 architecture to prematurely support those targets.

---

# 3. High-level system architecture

```text
UEFI
  |
Nagi Loader
  |
Nagi Kernel
  |
nagi-init
  |
Supervisor
  |
+-------------------- System Services ----------------------+
| FS | Net | Display | Input | Audio | Security | Package  |
| History | Search | Session | Font | AI | Model Manager   |
+-----------------------------------------------------------+
  |
Nagi SDK / Nagi PAL / relibc compatibility
  |
+--------------------- Applications -------------------------+
| Files | Settings | Notes | Calculator | Terminal | Albert |
+-----------------------------------------------------------+
```

The preferred direction is:

> **Kernel manages execution, memory, communication and authority. Everything else is a service.**

## 3.1 Unified Nagi environment and device/application model

Nagi is one logical user-owned environment, not separate Desktop, Mobile, or
Tablet OS editions. The environment contains identity, applications, logical
application state, user objects, workspaces, permissions, transaction/history
state, AI context, and a Device Registry. The official 0.1 implementation has
one practical Node: the QEMU x86-64 reference VM. Future Nodes are capability-
bearing execution and presentation endpoints, not new product identities.

The normative relationship is:

```text
Nagi Environment
├── Application Package (AppId)
│   └── Application Session (AppSessionId)
│       ├── logical state and ObjectId/WorkspaceId/TransactionId references
│       ├── Execution Instance(s) (ExecutionInstanceId, NodeId)
│       └── Presentation Surface(s) (SurfaceId, NodeId, capability context)
└── Device Registry (device@1)
    └── Node(s) (NodeId, capability descriptors)
```

These identities are distinct and must not be collapsed into PIDs, window IDs,
paths, inodes, or device-specific package IDs:

- `UserId` — user/environment identity;
- `NodeId` — stable Nagi-capable endpoint identity;
- `AppId` — one logical installed application identity;
- `AppSessionId` — continuable logical application session;
- `ExecutionInstanceId` — concrete process/runtime on a Node;
- `SurfaceId` — visual/interactive attachment to a Node and capability context;
- `WorkspaceId` — semantic grouping of objects and application sessions;
- `ObjectId` — stable user-visible object identity, separate from path/inode;
- `TransactionId` — meaningful action/history identity.

An Application Session is logically separate from its current process or
Execution Instance. A Presentation Surface is separate from a desktop Window;
a Window is one desktop presentation primitive. Nagi 0.1 may use a one-to-one
local mapping in practice, but future SDKs must be able to attach or detach
surfaces without changing the AppId or AppSessionId.

The user-space Device Registry, provisionally `device@1`, may expose a bounded
descriptor for the local Node when needed. Descriptors are capability-oriented
(display, input, audio, sensors, compute, storage, connectivity, and power),
so applications query capabilities and adaptive presentation context rather
than branching on `desktop`, `phone`, or `tablet` product categories.

Durable application data, session state, presentation-local state, and
execution-local state are separate. Cross-Node transport, pairing,
authentication, synchronization, conflict handling, and remote routing remain
explicit user-space services. Kernel Channels remain local IPC primitives and
must not become transparent network IPC. A remote Node does not gain authority
from environment membership; capabilities remain explicit, attenuated, and
locally policy-checked. This checkpoint adds no mobile hardware, cloud sync,
distributed execution, or fake multi-device behavior.

## 3.2 Common Language Architecture

Nagi uses English as its canonical internal language, while English (`en-US`)
and Japanese (`ja-JP`) are equally supported first-class user languages. This
rule applies across Nagi OS, Albert, Settings, Files, Terminal, the launcher,
lock screen, setup/OOBE, notifications, system dialogs, Store, and future
Nagi first-party applications.

English is used for source-code identifiers, APIs, IPC/RPC and message names,
schemas, configuration keys, localization keys, event names, internal command
names, developer diagnostics, log identifiers, and machine-readable error
identifiers. User-facing labels, explanations, notifications, and errors are
resolved through shared localization resources using stable English-based
keys; displayed text is never a key. Missing selected-locale entries fall
back to `en-US` and must not expose an empty UI or raw key to the user.

Japanese is not an experimental, community, partial, secondary, optional, or
post-hoc translation tier. Both languages are Nagi 0.1 quality commitments.
The architecture must allow future language packs without an OS-wide source
rewrite. UTF-8 is the default encoding for internal text, configuration,
localization resources, and logs.

System Language, Region/Locale, Input Language/Keyboard, and Albert/AI
Conversation Language are separate concepts and settings. In particular,
English presentation with Japan regional conventions, a Japanese IME with an
English UI, and Japanese AI conversation with an English system are valid
configurations. Terminal command syntax and machine-facing identifiers remain
English in 0.1, while user-facing help and explanations may be localized.

The detailed common-platform contract and implementation boundary are in
`docs/architecture/language-architecture.md`; ADR 0015 records the decision.

---

# 4. Kernel architecture

## 4.1 Kernel style

Nagi uses a **capability-based hybrid kernel**.

The long-term direction is microkernel-like, but 0.1 may keep a small number of pragmatic reference drivers in kernel space.

The kernel must remain small enough that high-level policy does not migrate into it.

## 4.2 Kernel-owned responsibilities

Kernel responsibilities:

- x86-64 CPU initialization;
- SMP startup;
- interrupt handling;
- APIC / IOAPIC / MSI/MSI-X;
- physical memory management;
- virtual memory;
- address spaces;
- process/thread scheduling;
- timers;
- waits;
- IPC;
- handles/capabilities;
- VMO management;
- device primitives;
- DMA buffers;
- minimal/reference drivers where required.

## 4.3 Explicitly not kernel-owned

Do not place the following in the kernel:

- filesystem semantics;
- VFS policy;
- TCP/IP;
- DNS;
- desktop shell;
- window manager;
- compositor policy;
- audio mixer;
- package management;
- AI;
- semantic search;
- browser logic;
- user permission policy.

Do not add high-level syscalls such as:

```text
file_open
socket_connect
window_create
play_audio
```

Those belong in user-space services.

---

# 5. Boot architecture

```text
UEFI
  |
Nagi Loader
  |
Kernel ELF
  |
ExitBootServices
  |
Kernel entry
  |
Memory / interrupts / SMP / scheduler
  |
nagi-init
  |
services
  |
login
  |
desktop
```

## 5.1 Nagi Loader

Implement the loader in Rust using the UEFI ecosystem.

It must:

- load the kernel ELF;
- gather the UEFI memory map;
- gather framebuffer information;
- gather ACPI information;
- construct BootInfo;
- call ExitBootServices;
- jump to the Nagi kernel entry point.

No GRUB dependency is required for the primary path.

---

# 6. Binary and ABI model

## 6.1 Binary format

Use:

- ELF64
- custom target: `x86_64-unknown-nagi`
- static linking preferred in 0.1

Dynamic/shared library support may be added later.

## 6.2 Native ABI

Nagi exposes:

- a native Nagi ABI;
- a stable C ABI;
- a Rust SDK.

The kernel syscall ABI and service API versions are separate.

## 6.3 Syscall ABI

x86-64:

```text
instruction: SYSCALL / SYSRET

RAX = syscall number
RDI = arg1
RSI = arg2
RDX = arg3
R10 = arg4
R8  = arg5
R9  = arg6
RAX = return/result
```

Published syscall numbers are never reused.

---

# 7. Kernel object model

Initial kernel object types:

- Process
- Thread
- AddressSpace
- VMO
- Channel
- Event
- Timer
- Device
- Interrupt
- DMA Buffer

Objects are never exposed as raw kernel pointers.

Processes use handles.

---

# 8. Handles and capabilities

A handle is the process-local representation of authority over a kernel/service object.

Possible rights include:

- READ
- WRITE
- MAP
- TRANSFER
- CONTROL

Use a stale-handle-resistant format, preferably:

```text
64-bit handle
  |
  +-- table slot
  +-- generation
```

## 8.1 Rights attenuation

If a sender owns:

```text
READ | WRITE | TRANSFER
```

it may transfer:

```text
READ
```

to another process.

The receiver must never be able to strengthen that handle back to WRITE.

This is a release-blocking security property.

---

# 9. IPC

## 9.1 Core primitive

The primary IPC primitive is a bidirectional **Channel**.

```text
Process A
  |
Endpoint A
  ||
Endpoint B
  |
Process B
```

## 9.2 Message form

Conceptually:

```text
Message
  Header
  Inline Payload
  Handles[]
```

Header fields should include enough information for:

- protocol identification;
- version;
- request/message ID;
- opcode;
- flags.

## 9.3 Data plane vs control plane

Use:

```text
small/control data -> Channel messages
large data          -> VMO/shared memory
```

Never repeatedly copy large rendering, audio or model buffers through ordinary message payloads.

## 9.4 Async-first

Service calls must be designed as asynchronous operations.

SDKs may expose synchronous wrappers for convenience.

## 9.5 Waiting

Kernel primitives:

- `wait`
- `wait_many`

must support waiting on:

- Channel readable;
- Event signaled;
- Timer fired;
- Process exit;
- service/socket readiness abstractions.

---

# 10. Service registry

Applications should depend on API names and versions, not PIDs or executable names.

Examples:

```text
filesystem@1
network.socket@1
audio@1
workspace@1
```

Service implementation changes must not require every client to learn a new PID or binary name.

Separate:

- system service registry;
- per-user/session registry.

---

# 11. Nagi IDL

Create a small Nagi Interface Definition Language.

Suggested extension:

```text
.nidl
```

Example:

```text
service audio@1 {
    fn volume_get() -> Volume;
    fn volume_set(volume: f32) -> Status;
}
```

Generate:

- Rust bindings;
- C headers;
- protocol IDs;
- request/response encoding.

Do not create an unnecessarily complex IDL compiler.

The wire format should be:

- binary;
- little-endian;
- fixed primitive widths;
- explicit lengths;
- versionable.

Do not use JSON as the core system IPC wire format.

---

# 12. Memory architecture

## 12.1 Process model

Each process owns:

- independent AddressSpace;
- independent Handle Table;
- security/session context.

Threads live inside processes.

## 12.2 VMO

The VMO is a central memory abstraction.

Support:

- Anonymous VMO
- Shared VMO
- File-backed VMO
- Device/DMA-related VMO where appropriate

## 12.3 mmap

0.1 must provide:

- map
- unmap
- protection changes

through native VMO mapping APIs, and expose POSIX `mmap`/`munmap`/`mprotect` through the compatibility layer.

This is required for Servo, Mesa and local model runtimes.

## 12.4 Demand paging

Support page-fault-driven loading for file-backed mappings.

Large GGUF files must not require an unconditional copy of the entire file into a separate memory buffer.

## 12.5 Page size

Use 4 KiB pages initially.

Huge page support is optional/later.

---

# 13. SMP and scheduler

Nagi 0.1 is not a single-core OS.

The official reference VM has four vCPUs.

Support:

- AP startup;
- per-CPU state;
- preemptive scheduling;
- multi-threading;
- TLS;
- sleeping/waking;
- timers;
- synchronization.

## 13.1 Scheduling classes

Initial classes may be:

```text
Realtime-ish
Interactive
Normal
Background
Idle
```

Suggested priorities:

```text
Audio            Realtime-ish
Input            Interactive
Window Server    Interactive
App UI           Interactive
Network          Normal
Servo workers    Normal
LLM              Normal/Background
Embedding        Background
Indexing         Idle
```

Do not allow ordinary applications to self-promote to the highest priority class.

## 13.2 Initial scheduler algorithm

0.1 does not require a Linux-CFS-class scheduler.

Use a simple and debuggable model such as:

- priority queues;
- round robin inside a priority;
- preemption;
- basic load balancing;
- preferably per-CPU run queues.

Correctness and predictability matter more than sophisticated heuristics.

---

# 14. Synchronization

Support at minimum:

- Mutex
- RWLock
- Semaphore
- Condvar
- Event
- futex-like wait/wake-on-address primitive
- TLS

The futex-like primitive is important for efficient Rust/C/POSIX synchronization.

---

# 15. OOM and resource pressure

No swap is required for 0.1.

Memory pressure handling must occur in a deliberate order.

Suggested order:

1. drop expendable caches;
2. pause/trim semantic indexing;
3. unload embedding model;
4. unload STT/TTS model;
5. shrink LLM context/KV cache where possible;
6. unload active LLM if necessary;
7. suspend background applications;
8. discard inactive Albert tab processes/state if supported;
9. terminate background apps as a last resort.

The OS should protect:

- desktop responsiveness;
- Files;
- foreground app;
- audio.

AI may be unloaded to preserve core OS usability.

---

# 16. Driver model

0.1 targets only the QEMU reference hardware.

Implement:

- PCI/PCIe discovery;
- ACPI basics;
- APIC;
- MSI/MSI-X;
- VirtIO Core;
- VirtIO Block;
- VirtIO Network;
- VirtIO GPU;
- VirtIO Sound;
- VirtIO RNG;
- keyboard/mouse support;
- timer/RTC.

Create a reusable VirtIO core handling:

- feature negotiation;
- virtqueues;
- descriptors;
- transport;
- interrupts;
- DMA buffers.

High-level services must not depend directly on VirtIO details.

Expose abstractions such as:

- BlockDevice
- NetworkDevice
- DisplayDevice
- AudioDevice
- InputDevice

Future hardware support should be inserted below these abstractions.

---

# 17. Storage layout

Logical GPT structure:

1. ESP / FAT32
2. System A
3. System B
4. User Data
5. Recovery
6. logical or physical Model Store separate from A/B duplication

System slots should be read-only or immutable-ish.

User data must survive system rollback.

Suggested logical mount points:

```text
/system
/users
/apps
/appdata
/temp
/recovery
/model-store
```

The POSIX layer may map conventional paths such as `/home`, `/tmp`, `/etc`.

---

# 18. Filesystem and semantic object model

Use:

- VFS
- ext2 initially
- FAT32 for ESP
- NagiFS later

## 18.1 Stable Object ID

Every user-visible object/file should have a stable Object ID separate from:

- path;
- inode.

Rename/move should retain identity whenever feasible.

`ObjectId` belongs to the Nagi environment and remains stable independently
of the Node that currently stores or presents the object. Nagi 0.1 may keep
the physical implementation local to the reference VM; it must not make a
device-local path the public application or AI identity.

## 18.2 Semantic metadata

Store separately:

- title;
- summary;
- tags;
- embedding;
- language;
- semantic type;
- relations;
- relation source: user/app/AI.

AI-inferred relations must never be silently presented as hard facts.

## 18.3 Semantic layer failure

Failure of semantic indexing must never make ordinary files unavailable.

Physical file access remains authoritative.

---

# 19. Workspace

A Workspace is a semantic grouping of:

- files;
- logical application sessions (`AppSessionId`);
- notes;
- browser pages/tabs;
- context;
- relations.

A file can belong to multiple workspaces.

Workspace identity (`WorkspaceId`) is device-independent. A Node-specific
window layout or surface attachment is presentation state associated with a
workspace, not the workspace's logical content.

The system should be able to restore basic work context, such as:

```text
continue yesterday's Nagi work
```

Workspace support is additive; it must not replace the physical filesystem.

---

# 20. History, transactions and Wayback

Nagi must support four recovery layers:

1. Immediate Undo
2. Local History
3. Wayback Backup
4. System Recovery

## 20.1 Transaction Ledger

Meaningful file/app/AI/system actions should have a transaction ID.

Where relevant, a history record should also be able to carry `UserId`,
`NodeId`, `AppId`, `AppSessionId`, optional `SurfaceId` and `WorkspaceId`,
the affected `ObjectId` values, timestamp, action, and result. These fields
are context, not a requirement that every event be cross-device.

Examples:

- AI moved 10 files;
- package installed;
- settings changed;
- file restored.

## 20.2 Version Store

Because ext2 has no native snapshots, 0.1 must implement history above the filesystem.

Store prior versions where required.

## 20.3 Restore itself is reversible

Before restoring an older file/system state, preserve the current state.

The user must be able to undo the restore.

## 20.4 Event-based Wayback

Support time/event concepts such as:

- before AI file organization;
- before Albert update;
- before driver/package installation;
- yesterday;
- previous version.

---

# 21. A/B system and recovery

System updates should use:

```text
System A
System B
```

Flow:

```text
running A
 -> write update to B
 -> verify
 -> boot B
 -> mark success after login/desktop readiness
 -> on repeated boot failure, return to A
```

0.1 does not require a full internet update service, but the slot infrastructure and rollback path must work.

## 21.1 Recovery Environment

Recovery must boot independently and provide:

- boot slot selection;
- boot logs;
- filesystem check;
- important file/history restore;
- advanced terminal;
- basic repair operations.

---

# 22. User/session/account model

Roles:

- Owner
- Standard User
- Guest

The system must support a fully local account.

Cloud identity is not required.

Owner is not an always-root process.

Strong operations should use action-specific temporary authorization.

Authentication and authorization are separate concepts.

AI may never elevate itself.

---

# 23. Permission model

Prefer:

```text
Allow
Ask
Deny
Allow once
```

Distinguish foreground and background access where meaningful.

Selected-file actions count as user consent; do not show redundant prompts for the exact file the user just selected.

Important permissions include:

- file access;
- network.connect;
- network.listen;
- clipboard;
- microphone.capture;
- screenshot;
- background execution;
- debug/process inspection.

Trusted permission and credential dialogs are OS-owned.

---

# 24. Developer Mode

Owner may explicitly enable Developer Mode.

Developer Mode can expose:

- unsigned package install;
- custom repositories;
- kernel logs;
- raw diagnostics;
- IPC tracing;
- service restart;
- test/debug drivers;
- expanded debugging APIs.

Developer Mode does **not** mean "disable all security".

Owner override must occur through a trusted human path.

The AI may never invoke Owner override on its own.

---

# 25. Desktop and shell architecture

```text
Application Session
  |
Desktop Presentation Surface
  |
Window API (desktop surface primitive)
  |
Window Server
  |
Compositor
  |
Display Service
  |
VirtIO GPU
```

The Window Server remains the Nagi 0.1 desktop presentation path. A Window is
not the universal application abstraction: one `AppSessionId` may eventually
attach multiple `SurfaceId` values, while the current implementation may use
one local desktop surface.

`nagi-shell` is a separate process responsible for:

- desktop;
- dock/launcher;
- notifications;
- Nagi Bar;
- workspace switcher.

---

# 26. GUI toolkit

Support:

- Presentation Surface / Presentation Context;
- Window
- Button
- Text
- TextField
- List
- ScrollView
- Menu
- Toolbar
- Dialog

Layouts:

- Row
- Column
- Stack
- Grid

Design for logical pixels and DPI scaling from the start.

Support:

- UTF-8;
- CJK;
- text shaping;
- cursor/selection;
- Japanese IME.

Accessibility metadata should be present early, even if 0.1 accessibility UI is limited.

---

# 27. Graphics architecture

## 27.1 Native Nagi apps

```text
Application Session
  |
Presentation Surface
  |
Nagi UI Toolkit
  |
CPU 2D Renderer
  |
Surface VMO
  |
Window Server / Compositor
  |
VirtIO GPU scanout
```

## 27.2 Surface

Conceptually:

```text
Surface
  width
  height
  stride
  pixel format
  VMO
  damage regions
```

Default:

- RGBA8888
- premultiplied alpha

Use double buffering or a small buffer pool.

## 27.3 Compositor

0.1 uses a software compositor.

Support:

- surface positioning;
- clipping;
- alpha blending;
- resizing;
- opacity;
- damage regions;
- simple shadows;
- trusted overlays.

Do not require blur, HDR or advanced GPU effects.

## 27.4 Security overlays

Permission dialogs, lock UI and trusted system overlays must render above normal apps.

---

# 28. Albert browser

The standard Nagi browser is named **Albert**.

Albert uses **Servo as its only planned browser engine**.

Do not introduce Chromium or NetSurf as a fallback in the 0.1 design.

Albert is the default browser but is not a required networking subsystem.

Removing Albert must not break:

- package networking;
- curl/git-like tools;
- third-party browsers;
- Nagi networking.

Use the normal default-handler system for `http`/`https`.

---

# 29. Albert architecture

```text
Albert UI / Browser Core
  |
servo-nagi platform adapter
  |
Servo / WebRender
  |
Nagi rendering + OS adapters
  |
Nagi services
```

Albert UI chrome:

- tabs;
- address bar;
- back;
- forward;
- reload;
- downloads;
- bookmarks;

should use Nagi UI.

The web content area is a Servo/Web surface.

---

# 30. Servo integration rules

Albert is the Servo embedder.

Do not make `servoshell` the final browser.

Use Servo's embedder boundaries for:

- EventLoopWaker;
- RenderingContext;
- WebView;
- WebViewDelegate;
- permissions;
- file picker;
- clipboard;
- IME;
- navigation;
- dialogs.

Do not fake X11 or Wayland.

Create an explicit Nagi target path:

```text
target_os = "nagi"
```

---

# 31. Servo rendering plan

Bootstrap in stages.

Phase A:

```text
Servo
 -> software rendering
 -> image/buffer
 -> Nagi Surface
```

Correctness first.

Phase B:

```text
Servo
 -> NagiServoRenderingContext
 -> Mesa Softpipe
 -> Surface VMO
 -> Nagi compositor
```

Use Mesa Softpipe for 0.1.

LLVMpipe, VirGL and hardware acceleration are later optimizations.

---

# 32. Browser feature target

Albert 0.1 must provide:

- actual internet access;
- DNS;
- HTTPS;
- HTML/CSS/JS to Servo-supported extent;
- tabs;
- history;
- bookmarks;
- downloads;
- file uploads;
- find in page;
- session restore;
- basic developer diagnostics.

Not required:

- Chrome compatibility;
- YouTube perfection;
- DRM;
- extensions;
- password manager;
- WebGPU;
- advanced video stack.

---

# 33. Browser security integration

File upload:

```text
Web page
 -> Servo request
 -> Albert
 -> Nagi File Picker
 -> selected read-only handle
```

Downloads go through Nagi File Service.

Web microphone requests:

```text
Site
 -> Servo
 -> Albert
 -> Nagi Permission Broker
 -> Nagi Audio
```

Website JavaScript must never directly invoke:

- Nagi AI;
- Tool APIs;
- arbitrary file operations;
- system settings.

Web content is **UNTRUSTED input**.

---

# 34. Browser context and AI

Public Nagi Browser Context APIs may expose, subject to user policy:

- URL;
- title;
- selected text;
- visible text;
- tab metadata;
- history entry;
- downloads.

Albert is not privileged by secret APIs.

Future third-party browsers should be able to use the same public interfaces.

Nagi AI must receive browser context through:

```text
Servo
 -> Albert
 -> Browser Context API
 -> Nagi AI
```

Servo must never directly call the Nagi AI runtime.

---

# 35. Network architecture

```text
App
 -> Nagi Network API
 -> nagi-net
 -> smoltcp
 -> VirtIO Net
```

0.1:

- Ethernet framing;
- ARP;
- IPv4;
- ICMP;
- UDP;
- TCP;
- DHCP;
- DNS/cache;
- Nagi socket API;
- connect/listen permission split;
- simple firewall;
- manual proxy;
- system trust store.

IPv6 may be later.

---

# 36. TLS and trust

Do not implement custom cryptography.

Servo should keep its own proven web TLS/network behavior where practical while relying on Nagi socket/runtime support.

Native apps may use Rustls or equivalent trusted libraries.

Nagi owns the trust store:

- system roots;
- enterprise roots;
- user roots.

Changing trust roots requires appropriate authorization.

---

# 37. Audio architecture

```text
Apps / Albert / AI
  |
Nagi Audio API
  |
nagi-audio
  |
Audio Device API
  |
VirtIO Sound
```

Apps do not directly touch the audio device.

Support:

- playback;
- capture;
- mixing;
- volume;
- mute;
- per-app/session volume;
- audio focus.

Raw microphone capture and speech transcription are separate authorities.

An app may receive transcribed text without receiving raw PCM.

---

# 38. Voice architecture

STT:

```text
Mic
 -> nagi-audio
 -> speech service
 -> VAD
 -> whisper.cpp
 -> text
```

0.1 uses push-to-talk.

Wake word may be experimental later.

Microphone activity must always show a system-owned indicator naming the consumer.

Voice authentication is not an identity mechanism.

---

# 39. Local AI architecture

```text
User / App / constrained Context
              |
              v
       Context Resolver / Router
          /         |          \
         v          v           v
   Deterministic  Decision     Generative /
   Fast Path      capability   Reasoning
         \         |           /
          +--------+----------+
                   v
          Action / NagiPlan candidate
                   v
       Deterministic Validator / Policy
          / Permission / Executor
                   v
          Transaction / Undo / Ledger
```

The three lanes are distinct, but share one side-effect boundary:

- **Tier 0 — Deterministic Fast Path:** exact commands, simple parsers,
  known intents, and deterministic state transitions that do not require
  probabilistic inference.
- **Tier 1 — Decision capability:** bounded boolean, choice, score, ranking,
  classification, routing, candidate-pruning, and future batch decisions.
  `System 1` is descriptive terminology for this lane; the durable Nagi
  contract is the capability, not the label or model family.
- **Tier 2 — Generative / Reasoning Path:** language understanding, complex
  intent interpretation, reasoning, planning, summarization, explanation, and
  generation.

Generative/reasoning responsibilities include:

- language understanding;
- intent recognition;
- planning;
- summarization.

Decision capabilities may select or prune bounded candidates before the
Generative/Reasoning path. They do not grant authority or replace
deterministic code responsibilities:

- permissions;
- risk;
- capability validation;
- execution;
- logging;
- transaction creation;
- undo/rollback.

The LLM never directly emits arbitrary shell commands for execution.

The provider-neutral Decision contract is defined by the following typed
concepts: `DecisionProvider`, `DecisionRequest`, `DecisionResult`,
`DecisionCapability`, `DecisionKind`, `DecisionBatchRequest`, and
`DecisionBatchResult`. A request carries a stable request ID, bounded inputs
or candidates, constrained context, requested capability, sensitivity/privacy
classification, optional latency budget, local-only/cloud-allowed policy,
caller identity, and applicable logical identity context. A result carries a
bounded selected value, optional score/distribution and confidence, provider /
model metadata, and fallback/escalation metadata. It carries no validation
state and grants no authority; providers that do not support confidence are
valid without an invented confidence value.

The contract must allow bounded batch decisions. A preferred compatible
DecisionProvider may be followed by a local compatible provider,
`LlmDecisionAdapter`, a suitable Generative/Reasoning path, or deterministic /
manual fallback. The selected fallback must remain within sensitivity and
policy limits, and all side effects still pass deterministic
Validator/Policy/Permission/Executor checks.

---

# 40. Nagi AI runtime isolation

`nagi-ai-runtime` performs local GenerativeProvider inference only for the
Nagi 0.1 Generative LLM path. Provider adapters and future DecisionProvider
implementations remain user-space and untrusted.

It should not hold:

- arbitrary Files access;
- Network;
- kernel/security privileges;
- microphone capture;
- window control.

It may receive:

- model file read handles;
- constrained context;
- inference parameters.

AI runtime and provider output flows to Planner/Validator/Executor, not
directly to system services. A DecisionProvider result is a bounded candidate,
not a permission, capability, or authority grant.

---

# 41. Default model decision

This is fixed for Nagi 0.1.

```text
Default Standard:
IBM Granite 4.2 3B

Alternative Standard:
Qwen3 4B

Lite:
Google Gemma 3 1B
```

Granite is the default unless a concrete technical blocker appears, such as:

- unusable Japanese intent performance;
- unusable structured planning;
- severe llama.cpp incompatibility;
- unacceptable resource consumption.

A minor benchmark advantage by another model is not sufficient reason to change the default.

---

# 42. Model runtime

Use:

```text
GenerativeProvider
 -> nagi-ai-runtime
 -> llama.cpp
 -> GGUF
```

For Nagi 0.1 this is the Generative LLM runtime for Granite, Qwen, and Gemma.
Do not create separate inference runtimes for each Generative LLM.

Use llama.cpp as a library, not an internal HTTP server.

This does not make llama.cpp or GGUF the universal runtime contract for every
AI provider. DecisionProvider, EmbeddingProvider, SpeechToTextProvider, and
TextToSpeechProvider may use other runtimes. When a dedicated DecisionProvider
is unavailable, `LlmDecisionAdapter` may reuse this local GenerativeProvider
path to satisfy the typed Decision API; that is a fallback adapter, not a
DecisionProvider runtime requirement.

---

# 43. Model Manager

Create `nagi-model-manager`.

Responsibilities:

- model discovery;
- load/unload;
- RAM budgeting;
- capability- and role-based model/provider selection;
- compatibility checks;
- model metadata;
- license metadata;
- hash verification;
- cache behavior.

The registry is not limited to LLMs. Capability examples include
`text.generate`, `structured.generate`, `reasoning`, `decision.boolean`,
`decision.choice`, `decision.score`, `decision.ranking`,
`decision.classification`, `decision.routing`, `decision.candidate_pruning`,
`decision.batch`, `embedding`, `speech.stt`, and `speech.tts`. Applications
must request a capability or role rather than hardcode a vendor/model name.

The Model Router considers capability, role, availability, provider health,
local/offline state, privacy and sensitivity policy, latency, memory pressure,
loaded state, user preference, and fallback availability.

Store models outside A/B system duplication.

Example:

```text
/model-store/
  llm/
  embedding/
  stt/
  tts/
```

The model store classification must remain extensible to a future `decision/`
category or equivalent provider-neutral metadata. Nagi 0.1 does not require a
physical directory migration solely to establish that boundary.

---

# 44. Model package metadata

Track:

- Name
- Developer
- Version
- Architecture
- Parameter count
- Quantization
- File size
- RAM recommendation
- Languages
- License
- Source
- SHA-256
- modification state
- provider type;
- supported capabilities;
- supported decision kinds;
- batch support;
- confidence support;
- local/cloud classification;
- runtime/backend;
- preferred roles.

Gemma's license/notice remains separate from the Nagi OS license.

Jev-specific metadata is not part of the common contract. A future provider
is represented through provider-neutral identity, capability, role, locality,
runtime, health, and policy metadata.

---

# 45. Model memory policy

Do not keep all three Generative LLMs loaded.

On the 8 GB reference VM, typically keep one Generative LLM loaded at a
time. A future specialized DecisionProvider may have a separate memory and
runtime profile; it must still yield to core OS usability and policy.

Boot must not block on loading the LLM.

Use lazy loading after the desktop becomes usable.

Granite is the Default Standard target. A dedicated System 1 model is not a
Nagi 0.1 loading requirement; `LlmDecisionAdapter` may use the already
selected local GenerativeProvider when a bounded decision is needed.

---

# 46. AI Fast Path

Simple deterministic commands must bypass probabilistic providers when
possible. This is Tier 0 of the three-lane AI architecture.

Examples:

```text
volume 30%
open calculator
mute
show battery/status
```

Fast Path:

```text
intent parser
 -> deterministic action
```

No model load required.

If a decision or generative provider is unavailable, the Fast Path remains
available for commands that can be resolved deterministically.

---

# 47. Structured planning

Planning mode must produce schema-constrained output.

Example:

```json
{
  "plan_version": 1,
  "intent": "file_move",
  "steps": [
    {
      "action": "file.move",
      "object_id": "...",
      "destination_object_id": "..."
    }
  ]
}
```

Use grammar/JSON-schema constrained generation where practical.

Do not execute partial streaming JSON.

Validate the complete plan first.

Prefer Object IDs to model-invented filesystem paths.

The Planner may receive a small, bounded action set selected by the Action
Registry and a DecisionProvider or `LlmDecisionAdapter`. A decision result is
not itself a plan and must not bypass complete plan validation.

---

# 48. Tool selection

Do not expose every system action to the model on every request.

Use a Tool/Action Registry and pass only relevant action schemas.

The preferred selection path is:

```text
Intent / constrained Context
 -> Action Registry
 -> deterministic filtering
 -> DecisionProvider or LlmDecisionAdapter
 -> small relevant action set
 -> Generative Planner
```

If no DecisionProvider is available, deterministic filtering and the local
adapter or existing reasoning path remain valid. The complete registry must
not be exposed merely to compensate for provider unavailability.

Example:

```text
volume request
 -> system.volume.get
 -> system.volume.set
```

Small models benefit from smaller tool spaces.

---

# 49. AI autonomy

Modes:

- Conservative
- Balanced — default
- Autonomous

Even Autonomous must never cross:

- owner authorization;
- system security boundaries;
- credentials/identity;
- irreversible external side effects;

without deterministic policy.

Reversible internal actions should prefer execution + Undo rather than excessive confirmation.

Decision confidence, score, or probability may influence routing, escalation,
candidate pruning, ranking, review requests, or an existing confirmation
strategy. It may never grant or strengthen capabilities, permissions,
Owner override, or authority for external, destructive, credential, upload,
share, or privilege-changing operations.

---

# 50. AI Activity Ledger

Record:

- user intent;
- resolved context;
- selected model;
- plan;
- actions;
- results;
- denials;
- transaction ID.

Decision activity may additionally record the request category, provider,
model, selected bounded result, confidence/score summary when useful,
fallback, escalation, and resulting Action/Plan. Do not persist hidden
chain-of-thought, credentials, API keys, unrestricted sensitive context, or
unlimited raw batch distributions.

Do **not** persist hidden chain-of-thought.

AI Activity Ledger, Security Activity and technical System Logs are separate stores.

---

# 51. AI memory model

Separate:

1. Conversation Context — short-lived;
2. Semantic History — retrieval/search;
3. Persistent Preferences/Memory — explicit and controlled.

The LLM must not create unrestricted persistent memories on its own.

---

# 52. Embedding and semantic search

Initial embedding candidate:

```text
multilingual-e5-small
```

Use a pinned source revision and convert/build in a reproducible way.

Supported semantic content in 0.1:

- txt;
- md;
- basic HTML/text;
- extractable PDF text;
- Albert page metadata/text.

Hybrid search combines:

- embedding similarity;
- filename;
- metadata;
- time;
- workspace relation;
- object relation.

A future provider-neutral flow may be:

```text
lexical / metadata / embedding retrieval
 -> permission-filtered candidate set
 -> Decision ranking or candidate pruning
 -> reasoning/summarization
```

Decision capability is not a prerequisite for semantic search. Exact and
metadata search must continue to work when semantic AI is disabled, and an
EmbeddingProvider remains replaceable.

Results should explain why they matched.

---

# 53. STT

Use:

```text
whisper.cpp
Whisper small multilingual
```

as the 0.1 baseline.

Do not keep STT permanently resident under memory pressure.

Push-to-talk flow:

```text
Super+V
 -> capture PCM
 -> load/activate Whisper
 -> transcribe
 -> text
 -> intent handling
```

Known app/workspace vocabulary may be used for constrained correction, but do not let an LLM rewrite the user's spoken meaning freely.

---

# 54. TTS

The Speech API is fixed; the concrete TTS engine is replaceable.

Evaluate at implementation time:

- Piper;
- MeloTTS;
- another local engine if licensing/portability is superior.

Selection criteria:

- Japanese quality;
- CPU performance;
- RAM;
- porting difficulty;
- binary dependencies;
- engine license;
- voice/model license.

Do not hardwire the rest of Nagi to one TTS implementation.

---

# 55. Nagi personality

Nagi's default personality is:

- calm;
- competent;
- restrained;
- occasionally lightly witty;
- not overly emotional;
- does not interrupt unnecessarily.

Expression setting:

- Minimal
- Balanced
- Expressive

Default:

```text
Balanced / Calm Companion
```

Critical/safety responses override personality and become concise and neutral.

Do not maintain a fake "emotional mood state" as a core OS variable.

---

# 56. Package model

Applications are packages, currently using the provisional extension:

```text
.xapp
```

Package contents may include:

- manifest;
- executable;
- resources;
- action/service schemas;
- licenses;
- signature.

App bundles should be immutable/read-only-ish.

User data is stored separately as:

- persistent;
- cache;
- temp.

---

# 57. App authority vs actions

Terminology:

- **Capability / Permission** = what an app is authorized to access.
- **Action / Service API** = what functionality an app provides.

Example Albert authorities:

- network.connect;
- clipboard.read;
- selected-file access.

Albert-provided actions:

- web.navigate;
- web.current_page;
- web.selected_text.

Do not call both concepts "capabilities".

---

# 58. Package manager

Provide:

```text
nagi-pkg / Package Service
```

Support:

- install;
- remove;
- update/replace;
- list;
- info.

Store is optional and never mandatory.

Side-loading and GitHub-style distribution are normal.

Unsigned packages must be installable through Developer Mode + warning/authorization.

Signatures matter for:

- identity;
- update continuity;
- trust continuity.

Use atomic app updates and rollback where practical.

---

# 59. SDK

Provide:

- Rust first-class SDK;
- C ABI;
- Nagi UI;
- file handle APIs;
- IPC;
- network;
- audio;
- clipboard;
- notifications;
- Action registration;
- build/package/install/run/debug tools.

Sample apps:

- hello window;
- text editor;
- file picker;
- network client;
- audio player;
- action provider;
- background service.

Nagi system GUI apps should dogfood public APIs whenever possible.

---

# 60. POSIX compatibility

Nagi is **not** architecturally a POSIX kernel.

POSIX is a user-space compatibility layer.

Preferred first libc strategy:

```text
relibc
 -> Nagi backend
 -> libnagi / Nagi PAL
```

Do not copy Redox's entire kernel/service design merely because relibc originates there.

If relibc proves fundamentally unsuitable, reevaluate musl or another libc, but relibc remains the initial choice.

---

# 61. POSIX file descriptors

Native Nagi uses handles.

POSIX code expects integer file descriptors.

Implement an fd table inside the compatibility runtime.

Example:

```text
fd 3 -> FileHandle
fd 4 -> SocketHandle
fd 5 -> Pipe/stream object
```

POSIX APIs do not bypass capability checks.

---

# 62. POSIX 0.1 priority

Tier A / required:

- C standard runtime;
- malloc/free;
- stdio;
- file I/O;
- directories/stat;
- pthread;
- TLS;
- mutex/condvar;
- mmap/mprotect;
- sockets;
- DNS;
- poll/select;
- clock/time;
- sleep;
- environment;
- basic signals;
- process spawn.

Tier B / when required:

- symlink;
- advanced signals;
- file locking;
- epoll compatibility;
- process groups;
- tty details.

Not required for 0.1:

- full `fork()` semantics;
- System V IPC;
- Linux namespaces;
- cgroups;
- Linux ptrace compatibility;
- complete `/proc`;
- io_uring;
- Linux binary ABI.

---

# 63. Process creation

Native Nagi process creation is spawn-based.

Do not make Unix `fork()` foundational.

Use:

```text
process.create(...)
spawn(...)
```

as the native model.

Expose `posix_spawn()` efficiently.

General `fork()` compatibility may be absent or limited in 0.1.

---

# 64. Rust std PAL

Servo requires substantial Rust standard library support.

Implement:

```text
std::thread
std::sync
std::fs
std::net
std::time
std::env
std::process
```

through:

```text
Rust std
 -> Nagi PAL/runtime
 -> Nagi services/kernel ABI
```

Do not add filesystem/network syscalls to the kernel merely to simplify `std`.

---

# 65. Terminal and shell

Provide a standard GUI Terminal application.

Native shell working name:

```text
nsh
```

It does not need full Bash compatibility.

Basic commands:

```text
cd
ls
pwd
cp
mv
rm
mkdir
cat
echo
```

Nagi-native administration:

```text
nagi ps
nagi log
nagi mem
nagi cpu
nagi service
nagi pkg
nagi model
nagi history
nagi wayback
nagi permission
nagi handle
nagi trace
nagi net
nagi audio
nagi workspace
```

Important commands should support:

```text
--json
```

for machine-readable output.

---

# 66. Logging and diagnostics

All components should use structured logging.

Fields include:

- timestamp;
- severity;
- component;
- event ID;
- message;
- structured fields;
- process/thread;
- session;
- transaction ID;
- trace ID when applicable.

Severity:

- TRACE
- DEBUG
- INFO
- WARN
- ERROR
- FATAL

Kernel uses a ring buffer and serial output from early boot.

---

# 67. Boot and crash diagnostics

Track boot stages:

```text
BOOT.LOADER.START
BOOT.KERNEL.ENTER
BOOT.SMP.READY
BOOT.FS.READY
BOOT.LOGIN.READY
BOOT.DESKTOP.READY
```

Every boot receives a Boot ID.

Kernel panic output should include, when possible:

- reason;
- CPU;
- RIP;
- registers;
- current process/thread;
- backtrace.

User-process crashes should produce:

- process/package ID;
- version;
- build ID;
- thread;
- exception;
- fault address;
- registers;
- backtrace;
- mappings;
- timestamp;
- Boot ID.

---

# 68. Service supervision

Service health states:

- Starting
- Ready
- Healthy
- Degraded
- Failed
- Restarting
- CrashLoop

Crash looping must not cause infinite restart storms.

Network, audio, search, AI and similar services should be restartable without killing the kernel.

---

# 69. Diagnostics privacy

`nagi diagnose bundle` may include:

- system info;
- boot logs;
- service status;
- crash records;
- hardware info;
- build versions.

It must not silently include:

- document contents;
- AI conversations;
- browser page contents;
- passwords;
- cookies.

Uploads are not automatic.

0.1 diagnostic bundles remain local unless the user explicitly exports them.

---

# 70. Repository structure

Nagi 0.1 uses a monorepo.

Recommended structure:

```text
nagi-os/
  boot/
  kernel/
  services/
  drivers/
  sdk/
  interfaces/
  compat/
  apps/
  shell/
  ports/
  third_party/
  models/
  tests/
  recovery/
  image/
  tools/
  docs/
  ci/
  rust-toolchain.toml
  Cargo.toml
  Cargo.lock
  nagi.toml
  nagi
  nagi.ps1
  AGENTS.md
```

Keep dependency direction clear:

```text
Apps
 -> SDK / services
 -> libnagi
 -> kernel ABI
```

Kernel must never depend on apps/services.

---

# 71. Build system

Use existing build tools where appropriate:

- Cargo for Nagi Rust and Servo;
- Meson/Ninja for Mesa;
- CMake for llama.cpp/whisper.cpp;
- LLVM/Clang/LLD for C/C++ and linking.

Do not force everything into Cargo.

Create a Rust-based host orchestrator:

```text
tools/nagi-cli
```

Primary commands:

```text
./nagi doctor
./nagi fetch
./nagi build
./nagi image
./nagi run
./nagi test
./nagi clean
./nagi fmt
./nagi lint
```

PowerShell should provide a thin launcher on Windows.

---

# 72. Toolchain pinning

Pin:

- Rust nightly by date;
- LLVM/Clang expectations;
- QEMU expectations;
- OVMF expectations;
- external dependency revisions.

Do not depend on "latest nightly".

The release build must be reconstructible from the pinned manifest.

---

# 73. External sources

Do not vendor multi-gigabyte upstream sources directly into the main Git history.

Maintain:

```text
third_party/sources.lock
```

with exact source repositories, commits and hashes.

`./nagi fetch` downloads sources to a cache and applies Nagi-owned patches.

Example:

```text
ports/servo/patches/
ports/mesa/patches/
ports/relibc/patches/
```

Do not modify cached upstream trees and then forget to capture those changes as patches.

---

# 74. Model distribution

Do not commit multi-gigabyte models to the Git repository.

Maintain model manifests containing:

- exact source revision;
- filename;
- hash;
- license metadata;
- provider family and supported capability/role metadata where applicable;
- local/cloud classification and runtime/backend metadata.

Commands may include:

```text
./nagi models fetch lite
./nagi models fetch full
```

Full Developer Preview can include:

- Granite 4.2 3B;
- Qwen3 4B;
- Gemma 3 1B;
- embedding model;
- Whisper;
- chosen TTS model.

Jev or another specialized DecisionProvider may be added later, but it is not
a Nagi 0.1 model-distribution or release requirement. Model distribution must
not turn a provider example into a mandatory dependency.

---

# 75. CI tiers

The runnable wrapper mapping for milestone acceptance is maintained once in
`tests/acceptance/registry.tsv`. It maps Acceptance IDs to milestones,
subsystems, host/target scope, wrapper scripts, and timeouts; the success
criteria below remain authoritative.

## Pull Request

Run:

- formatting;
- lint;
- host unit tests;
- diagnostic-parser regression tests;
- focused M0 host acceptance;
- target build and the real M17 first-web-pixel acceptance.

Record filtered cases as `NOT RUN` and environmental prerequisites as
`BLOCKED`. A host acceptance result cannot satisfy a target milestone.

## Main branch

Run:

- desktop build;
- integration tests;
- GUI tests;
- security tests;
- Albert local-page tests.

## Nightly

Run:

- full build;
- actual Servo networking tests;
- real Granite tests;
- Wayback tests;
- 8 GB stress test.

## Release

Run all acceptance tests in a clean environment.

---

# 76. CI networking

Do not make routine CI depend on public internet availability.

Run host-side local HTTP/HTTPS test servers and connect the Nagi VM to them.

Use a test CA where required.

External-site smoke tests may exist in nightly/release jobs, but must not be the only test.

---

# 77. AI CI

Separate deterministic and probabilistic testing.

## Deterministic tests

Use:

- mock model;
- known structured plans;
- fixed executor results.

Test:

- Planner;
- Validator;
- Executor;
- Transaction;
- Undo;
- mock DecisionProvider and fixed DecisionResult;
- DecisionProvider fallback, escalation, invalid bounded responses, batch
  limits, and no authority escalation.

## Real-model tests

Use Granite in nightly/release.

Hard failures should test things such as:

- schema validity;
- correct action category;
- absence of unsafe actions;
- successful object resolution.

When a dedicated DecisionProvider is available, test its bounded result
validity and safe routing separately. The Nagi 0.1 path may use
`LlmDecisionAdapter` with a local GenerativeProvider; Jev availability is
never a CI prerequisite.

Do not hard-test exact prose wording.

---

# 78. Standard build profiles

Useful profiles:

```text
Minimal
Desktop
Browser
AI
Full
```

CI should not download every AI model on every pull request.

---

# 79. Image builder

Build disk images programmatically.

Do not require manual partitioning.

`./nagi image` should produce the required:

- ESP;
- system slots;
- data;
- recovery;
- model store layout.

Primary Developer Preview artifact:

```text
Nagi-OS-0.1-devpreview.qcow2
```

---

# 80. Release artifacts

Release should include:

- qcow2 image;
- SHA256SUMS;
- release notes;
- source revision;
- build manifest;
- third-party notices;
- architecture docs;
- SDK docs;
- contribution guide;
- roadmap.

Build manifest should include:

- Nagi version;
- Git commit;
- kernel build ID;
- Servo commit;
- Mesa commit;
- llama.cpp commit;
- Granite model hash;
- toolchain versions.

---

# 81. Official development host

Primary CI host for 0.1:

```text
Ubuntu x86-64
```

This does not make Nagi Linux-based. Linux is merely a build host.

Windows developers may use WSL2 initially.

Native Windows host support may expand later.

---

# 82. Codex architecture rules

Create `AGENTS.md` in the repository root and, where useful, directory-specific `AGENTS.md`.

Codex must follow:

1. Do not add Linux kernel dependencies.
2. Do not replace Nagi components with host-side helpers merely to pass a test.
3. Do not bypass capability/security checks.
4. Do not make AI privileged.
5. Do not add high-level file/network/window syscalls to the kernel.
6. Do not fake successful networking/rendering/AI results.
7. Do not modify upstream third-party source without recording the patch.
8. Do not skip acceptance tests by deleting or weakening them.
9. Do not silently change architecture decisions.
10. When a design conflict is unavoidable, document it before implementing the deviation.

---

# 83. Explicitly prohibited shortcuts

The following are prohibited unless the implementation is clearly marked as a test-only harness and never included in the release image.

## 83.1 Host escape

Do not let Nagi call the host Linux/Windows filesystem to simulate a Nagi filesystem.

Do not let Nagi call host networking directly to simulate `nagi-net`.

Do not let Albert render on the host and copy screenshots back into Nagi.

Do not run Granite on the host and return results as if Nagi ran the model.

## 83.2 Security bypass

Do not:

- grant all apps universal file access;
- make Owner equivalent to permanent kernel root;
- give Nagi AI unrestricted handles;
- skip permission broker checks.

## 83.3 Fake services

Do not implement:

```text
HTTPS test -> return hardcoded HTML
```

or:

```text
AI test -> return canned JSON
```

outside dedicated mock tests.

Mocks are allowed only where the test explicitly says it is testing the orchestration layer rather than the real component.

## 83.4 Compatibility contamination

Do not add Linux syscalls or POSIX semantics to the kernel merely because a port is difficult.

Fix the PAL/relibc/port layer instead.

---

# 84. Implementation phases

Nagi 0.1 is implemented in the following high-level phases.

```text
Phase 0  Repository / Toolchain / CI
Phase 1  Boot / Kernel / SMP / IPC
Phase 2  User Space / FS / CLI / Drivers
Phase 3  GUI / Security / Network / Audio
Phase 4  POSIX / SDK / Packages / History
Phase 5  Servo / Albert
Phase 6  Semantic / Granite AI / Voice
Phase 7  Recovery / Integration / Stress / Release
```

Detailed milestones follow.

---

# 85. Detailed milestone order

## M0 — Repository / Toolchain

Deliver:

- monorepo;
- Cargo workspace;
- `nagi-cli`;
- `AGENTS.md`;
- docs/test skeleton;
- `./nagi doctor/build/run/test` skeleton;
- basic CI.

Acceptance:

```text
./nagi doctor
```

successfully validates host dependencies.

---

## M1 — UEFI -> Kernel

Deliver:

- Nagi Loader;
- ELF loading;
- BootInfo;
- UEFI memory map;
- framebuffer info;
- ACPI pointer;
- ExitBootServices;
- kernel entry.

Acceptance:

QEMU serial prints:

```text
Nagi Kernel started
```

---

## M2 — Memory / Exceptions / Interrupts

Deliver:

- physical page allocator;
- page tables;
- kernel heap;
- IDT;
- exceptions;
- APIC/timer.

Acceptance:

- page allocation/free;
- expected page fault handling;
- timer interrupts;
- invalid access diagnostic.

---

## M3 — SMP / Scheduler / Threads

Deliver:

- AP startup;
- per-CPU data;
- thread model;
- context switching;
- preemptive scheduler;
- timer preemption;
- wait/wake basics.

Acceptance:

all four reference CPUs report online and run test workloads.

---

## M4 — Handles / VMO / IPC

Deliver:

- handle table;
- rights;
- generation counters;
- VMO;
- AddressSpace;
- Channel;
- Event;
- Timer;
- wait/wait_many.

Acceptance:

- process A -> process B channel message;
- transferred READ-only handle cannot be strengthened to WRITE.

---

## M5 — First User Process

Deliver:

- user/kernel separation;
- syscall entry;
- user ELF loader;
- user address space;
- stack/TLS basics;
- initial `libnagi`;
- `nagi-init`.

Acceptance:

user process prints:

```text
Hello from user space
```

---

## M6 — Init / Supervisor / Registry

Deliver:

- supervisor;
- manifests;
- dependency order;
- restart policy;
- service health;
- service registry.

Acceptance:

a client finds and calls a test `echo@1` service through the registry.

---

## M7 — Block / FS / Persistent Storage

Deliver:

- VirtIO core;
- VirtIO Block;
- VFS;
- ext2;
- file handles;
- directory operations;
- persistent data;
- file-backed VMO/mmap.

Acceptance:

create -> write -> shutdown -> reboot -> read same file.

---

## M8 — CLI Foundation

Deliver:

- initial `nsh`;
- serial/console CLI;
- basic file commands;
- process/memory/log commands.

Acceptance:

developers can inspect files and processes inside Nagi.

---

## M9 — Display / Input / First Window

Deliver:

- VirtIO GPU;
- framebuffer/scanout;
- Surface VMO;
- Window Server;
- input service;
- focus;
- mouse/keyboard.

Acceptance:

a real Nagi window can be moved by mouse in QEMU.

---

## M10 — Nagi UI / Desktop

Deliver:

- UI toolkit basics;
- Font Service;
- text;
- Japanese display/input path;
- widgets/layout;
- `nagi-shell`.

Initial apps:

1. Calculator
2. Notes
3. Files
4. Settings
5. GUI Terminal

Acceptance:

multiple GUI apps run simultaneously.

---

## M11 — Login / Permissions / Security

Deliver:

- Owner/Standard/Guest;
- local login;
- lock screen;
- Permission Broker;
- trusted dialogs;
- Developer Mode.

Acceptance:

malicious test app is denied unauthorized file/microphone access.

---

## M12 — Networking

Deliver:

- VirtIO Net;
- `nagi-net`;
- smoltcp;
- ARP;
- DHCP;
- IPv4;
- ICMP;
- UDP/TCP;
- DNS;
- Nagi socket API.

Acceptance:

Nagi opens a TCP connection and performs a real HTTP request through its own stack.

Do not start Servo before networking works independently.

---

## M13 — Rust std / POSIX

Deliver:

- Nagi PAL;
- Rust std platform support;
- relibc Nagi backend;
- pthread/TLS/files/mmap/time/socket/DNS/poll/spawn.

Acceptance:

- Rust std test app;
- C/POSIX test app;
- one small existing OSS library.

---

## M14 — Audio

Deliver:

- VirtIO Sound;
- `nagi-audio`;
- playback;
- capture;
- mixer;
- volume;
- mute;
- sessions.

Acceptance:

play PCM/WAV and capture microphone/audio input in the reference VM.

---

## M15 — History / Transaction

Deliver:

- Transaction Ledger;
- file version storage;
- trash;
- undo;
- restore;
- History Service.

History records must be able to associate logical `AppId`/`AppSessionId`,
originating `NodeId`, optional `SurfaceId`/`WorkspaceId`, and affected
`ObjectId` values. History is an environment activity model, not a log keyed
only by PID, Window, or path.

Acceptance:

create/edit/move/delete/restore/undo tests pass.

---

## M16 — Package / SDK

Deliver:

- `.xapp`;
- Package Service;
- Rust SDK;
- C SDK;
- Nagi IDL pipeline;
- package CLI.

The package and SDK model must use one stable `AppId` with logical
`AppSessionId`, `PresentationContext`, and `PresentationSurface` concepts.
Adaptive presentations and future architecture-specific payloads must not
create separate desktop/mobile application identities. Window remains a
desktop surface primitive rather than the universal application root.

Acceptance:

an out-of-tree Hello Nagi app builds with SDK only, packages, installs and launches.

---

## M17 — Servo Bootstrap

Deliver in order:

1. Servo compile;
2. Servo initialize;
3. `about:blank`;
4. local HTML;
5. CSS;
6. JavaScript;
7. mouse/keyboard/scroll.

Use software rendering first.

Acceptance milestone:

> **First Web Pixel on Nagi**

---

## M18 — Albert Browser

Deliver:

- Albert UI;
- tabs;
- address bar;
- navigation;
- history;
- bookmarks;
- downloads;
- file upload;
- clipboard;
- IME;
- permissions;
- session restore;
- HTTP;
- HTTPS.

Acceptance:

Albert is usable as a basic browser and renders several real HTTPS websites.

---

## M19 — Semantic Layer / Search

Deliver:

- Object IDs;
- semantic metadata DB;
- relations;
- Search Service;
- Workspace.

Workspace references logical application sessions and `ObjectId` values and
is independent of the Node. Node-local surface/layout state is presentation
state, not the workspace's logical identity.

Start with filename/metadata/time search, then embedding.

Acceptance:

files/pages/workspaces can be found and grouped using stable Object IDs.

---

## M20 — AI Runtime / Granite

Deliver:

- GenerativeProvider foundation and local `llama.cpp` / GGUF runtime;
- Granite 4.2 3B model package;
- capability/role-based Model Manager and Router foundation;
- inference API;
- lazy load.

The M20 runtime contract is for GenerativeProvider. It must leave a typed
DecisionProvider boundary for future specialized providers without fixing
DecisionProvider to llama.cpp/GGUF. A dedicated local System 1 model, Jev,
and any cloud DecisionProvider are optional and are not M20 PASS conditions.

Acceptance:

Granite responds locally inside Nagi.

---

## M21 — Planner / Validator / Executor

Deliver:

- Router;
- Context Resolver;
- Planner;
- NagiPlan@1;
- Validator;
- Executor;
- initial Action Registry.

The planning boundary also defines the provider-neutral DecisionProvider
contract, `LlmDecisionAdapter`, Decision routing, confidence-based
escalation, bounded Action candidate preselection, and deterministic fallback.
The native DecisionProvider may be absent in 0.1; Granite, Qwen, or Gemma
through `LlmDecisionAdapter` may satisfy the typed Decision API where
appropriate.

Start with few actions:

- app.launch;
- file.search;
- file.copy;
- file.move;
- system.volume.set.

Acceptance:

schema-valid plans execute only after deterministic validation; provider
unavailability follows a safe fallback; and confidence/score never increases
authority or bypasses Validator/Policy/Permission checks.

---

## M22 — AI Safety / Undo Integration

Deliver:

- AI -> transaction integration;
- Activity Ledger;
- risk/confirmation logic;
- undo integration;
- one security boundary for Decision and Generative output.

Both lanes must follow:

```text
Decision or Generative output
 -> Deterministic Validator / Policy / Permission
 -> Executor
 -> Transaction / Activity Ledger / Undo / Wayback
```

Decision confidence, score, or provider identity must never short-circuit
this boundary or grant authority.

AI activity records should preserve logical app/session/object/workspace
context and the relevant `NodeId` without allowing a Node or remote route to
strengthen capabilities.

Acceptance:

AI moves three files and `undo` restores them.

---

## M23 — Nagi Bar / Context / Albert AI

Deliver:

- unified Nagi Bar;
- current app context;
- selected object context;
- Workspace context;
- Albert Browser Context.

Context is expressed in terms of logical `AppId`, `AppSessionId`, `ObjectId`,
and `WorkspaceId`; the originating `NodeId` is context, not a replacement for
those identities. Albert remains one application identity with a desktop
presentation in 0.1.

Acceptance:

"Summarize this page" uses untrusted Albert context through the public Browser Context API.

Decision and Generative providers receive only constrained Context Broker
input. Browser context remains untrusted, and provider routing does not grant
access to app memory or system authority.

---

## M24 — Embedding / Semantic AI

Deliver:

- multilingual embedding model;
- chunking;
- semantic index;
- hybrid search.

The architecture may use lexical/embedding retrieval followed by Decision
ranking or candidate pruning and then reasoning/summarization, but a
DecisionProvider is not a prerequisite for semantic search. Permission
filtering and replaceable provider boundaries remain mandatory.

Acceptance:

natural-language queries such as:

```text
the Servo article I looked at yesterday
```

return plausible local results with explanations.

---

## M25 — Voice

Deliver:

- push-to-talk;
- whisper.cpp;
- Japanese STT;
- Speech Service;
- chosen local TTS;
- mic privacy indicator.

Acceptance:

spoken Japanese can launch Albert or perform a basic Nagi command.

---

## M26 — Qwen / Gemma / Automatic

Deliver:

- Qwen3 4B;
- Gemma 3 1B;
- capability/role-based model and provider routing;
- Generative versus Decision route selection;
- local/offline and provider availability fallback;
- model switching and automatic routing;
- custom-model entry point.

Granite remains the default standard model.

Models and apps must not be coupled to vendor/model names when a capability
or role can be requested. Jev and any specialized DecisionProvider are
optional provider examples, not M26 deliverables or acceptance prerequisites.

---

## M27 — A/B / Recovery

Deliver:

- slot switching;
- boot success marking;
- failed-boot rollback;
- Recovery Environment.

Acceptance:

intentionally broken inactive slot fails and system returns to the working slot.

---

## M28 — Integration / Stress

Reference load:

```text
4 vCPU
8 GB RAM

Desktop
Files
Notes
Albert 3-5 tabs
Granite loaded
Audio playback
Semantic Search
```

Acceptance:

- no kernel OOM;
- desktop remains usable;
- no sustained audio underrun;
- Granite does not monopolize every CPU;
- no major handle/memory leaks.

---

## M29 — Developer Preview Polish

No major new features.

Focus on:

- bug fixing;
- documentation;
- onboarding;
- screenshots;
- sample apps;
- diagnostic quality;
- package metadata;
- license notices;
- clean build.

---

## M30 — Release

Produce:

```text
Nagi-OS-0.1-devpreview.qcow2
SHA256SUMS
source revision
build manifest
licenses/notices
architecture docs
SDK docs
contribution guide
roadmap
```

---

# 86. Milestone completion rule

Do not proceed past a milestone merely because "most code exists".

Every milestone should end with:

1. build;
2. tests;
3. QEMU boot where applicable;
4. acceptance test;
5. log review;
6. documentation update;
7. clean/commit-ready repository state.

Record:

```text
Milestone Status:
PASS
PARTIAL
BLOCKED
```

A `BLOCKED` milestone should describe the exact blocker.

---

# 87. Failure continuation rules for Codex

When a build/test fails:

```text
failure
 -> identify root cause
 -> fix
 -> rerun focused test
 -> rerun milestone acceptance
```

Do not respond to failure by:

- deleting tests;
- lowering assertions without justification;
- returning fake success;
- disabling the affected feature;
- bypassing security;
- routing work through the host OS.

Temporary stubs are allowed only when clearly marked:

```text
STUB
TODO
NOT PRODUCTION
```

and only when the current milestone does not claim that functionality is complete.

---

# 88. Retry policy

For implementation work, Codex should attempt multiple rounds of diagnosis and repair before declaring a blocker.

Suggested default:

- up to 10 meaningful repair attempts for a milestone blocker;
- each attempt must change something based on evidence;
- do not repeat the same failed command blindly.

After repeated failure, preserve:

- logs;
- failing test;
- current hypothesis;
- changes attempted;
- next recommended experiment.

Do not hide a persistent failure to keep moving.

---

# 89. Parallel work rules

Some work may proceed in parallel after foundations exist.

Good parallel candidates:

- individual GUI apps;
- documentation;
- History UI;
- Package UI;
- audio UI;
- sample apps.

Critical path remains:

```text
Kernel
 -> IPC
 -> FS
 -> GUI
 -> Network
 -> PAL/POSIX
 -> Servo
 -> Albert
```

Do not parallelize heavily across an unstable critical path.

---

# 90. Nagi 0.1 Definition of Done

Nagi 0.1 is complete only when all of the following are true.

## 90.1 Boot and core

- Boots on official QEMU reference VM.
- Uses Nagi Loader + Nagi Kernel.
- Reaches login/desktop.
- 4 vCPUs are online.
- Process isolation works.
- VMO/IPC/capability security tests pass.

## 90.2 Persistent OS

- Persistent ext2/VFS works.
- Files app works.
- Object IDs survive normal rename/move where feasible.
- file-backed mmap works.

## 90.3 Desktop

- windowing/compositor/input works;
- Japanese text/input works;
- Files/Settings/Notes/Calculator/Terminal/Albert exist;
- multiple apps can run at once.

## 90.4 Security

- local Owner/Standard/Guest accounts;
- trusted permission dialogs;
- Developer Mode;
- file/network/microphone/etc. permissions;
- AI cannot self-elevate.

## 90.5 Networking

- DHCP/IPv4/DNS/TCP works;
- Albert reaches real HTTPS sites through Nagi networking;
- manual proxy is available.

## 90.6 Albert

- Servo runs natively on Nagi;
- HTML/CSS/JS render;
- tabs/history/bookmarks/downloads/upload/find/session restore work;
- real HTTPS websites load;
- Albert crash does not kill desktop/kernel.

## 90.7 Local AI

- Granite 4.2 3B is default standard model;
- Qwen3 4B and Gemma 3 1B are optional selectable bundled models;
- AI works offline;
- deterministic Fast Path, Decision capability, and Generative/Reasoning
  lanes remain distinct;
- provider routing uses typed capability/role contracts;
- Nagi 0.1 remains viable without Jev, a cloud DecisionProvider, or a
  dedicated local System 1 model;
- a local `LlmDecisionAdapter` may satisfy the DecisionProvider contract when
  no specialized provider is available;
- structured plans are validated;
- confidence/score never grants authority;
- model runtime has no arbitrary OS authority.

## 90.8 Semantic/voice

- semantic search works on supported local content;
- Workspace exists;
- push-to-talk Japanese STT works;
- local TTS works;
- microphone indicator works.

## 90.9 Recovery

- AI/file transactions can be undone;
- file version history works;
- restore is itself reversible;
- recovery environment boots;
- A/B rollback path works.

## 90.10 SDK/build

- `.xapp` installs;
- out-of-tree sample app builds;
- source -> image -> QEMU is reproducible through `./nagi`;
- CI can automatically detect desktop-ready state.

---

# 91. Explicitly out of scope for 0.1

Do not block 0.1 on:

## Physical hardware

- Raspberry Pi;
- x86 mini PC;
- laptops;
- broad desktop compatibility.

## Hardware support

- physical GPU acceleration;
- Wi-Fi;
- Bluetooth;
- USB general-purpose stack;
- NVMe;
- battery;
- suspend;
- webcam;
- touch;
- printers.

## Browser completeness

- full Chrome compatibility;
- YouTube perfection;
- Netflix/DRM;
- WebGPU;
- extensions;
- password manager.

## Compatibility

- Windows application compatibility;
- Linux binary compatibility;
- full fork;
- full Bash;
- formal POSIX certification.

## Cloud

- Nagi cloud account;
- cloud sync;
- cloud LLM;
- remote backup.

## AI

- GPU inference;
- NPU inference;
- huge models;
- unrestricted autonomous agents;
- requiring Jev or another specialized DecisionProvider;
- external/cloud DecisionProvider as a core dependency;
- probabilistic kernel policy or AI authority decisions.

---

# 92. Release blockers

The following prevent 0.1 release:

- reference VM cannot reliably boot;
- filesystem/data corruption;
- capability escape;
- frequent kernel panics;
- unusable desktop;
- unusable Files;
- fundamentally broken network;
- Albert cannot render basic real HTTPS pages;
- Granite cannot perform basic structured plans;
- Undo corrupts data;
- Recovery cannot boot;
- build cannot be reproduced from a clean supported host.

The absence of Jev, a dedicated local System 1 model, or a cloud Decision
Provider is not a release blocker. A compatible local adapter or deterministic
fallback must cover the applicable Nagi 0.1 path.

The following may remain as known Developer Preview issues:

- complex website layout bugs;
- Servo site incompatibilities;
- slow WebGL;
- minor UI glitches;
- incomplete animations;
- missing lower-priority POSIX calls;
- imperfect TTS voice quality.

---

# 93. Reference demo for Nagi 0.1

A successful 0.1 should be able to demonstrate:

1. boot Nagi in QEMU;
2. log in as Owner;
3. browse local files;
4. open a PDF/document;
5. open Albert;
6. browse a real HTTPS website;
7. ask:
   - "summarize this page";
8. Granite performs local summarization;
9. use push-to-talk:
   - "find the Servo article I looked at yesterday";
10. semantic search finds it;
11. ask:
   - "move these three files into the Nagi Development workspace";
12. structured plan executes;
13. ask:
   - "undo that";
14. transaction rolls back;
15. write a note;
16. reboot;
17. confirm files/workspace/history persist;
18. intentionally crash Albert;
19. desktop remains alive;
20. inspect crash log;
21. disconnect network;
22. continue using Files, Granite, semantic search, voice and Wayback offline.

This demonstration should prove that Nagi is an independent OS experience rather than merely a bootable kernel experiment.

---

# 94. Repository-facing documentation requirements

Maintain:

```text
README.md
AGENTS.md
docs/architecture/
docs/development/
docs/porting/
docs/decisions/
docs/testing/
```

Use ADRs for major decisions, including:

```text
ADR-0001 hybrid kernel
ADR-0002 channel IPC
ADR-0003 Servo browser
ADR-0004 Granite default LLM
ADR-0005 ext2 + semantic layer
ADR-0006 QEMU reference machine
```

---

# 95. Suggested root AGENTS.md content

Codex should eventually create and maintain a root `AGENTS.md` with rules equivalent to:

```text
- Read the current milestone and architecture docs before coding.
- Do not change architecture to make a single test easier.
- Do not add Linux runtime dependencies to Nagi.
- Do not use host OS facilities to fake guest functionality.
- Do not bypass capability checks.
- Do not give AI direct execution authority.
- Keep kernel responsibilities minimal.
- Keep POSIX compatibility in user space.
- Keep upstream third-party modifications as explicit patches.
- Run focused tests after each change.
- Run milestone acceptance before moving on.
- Update docs/decisions if architecture changes.
- Never delete a failing test solely to obtain green CI.
```

Directory-specific `AGENTS.md` may strengthen rules for:

- `kernel/`
- `services/ai/`
- `apps/albert/`
- `compat/`
- `ports/`

---

# 96. Final implementation priority

If tradeoffs are required, prioritize in this order:

1. correctness;
2. data integrity;
3. security boundaries;
4. diagnosability;
5. architectural cleanliness;
6. reproducibility;
7. user responsiveness;
8. performance optimization;
9. visual polish.

A slower correct browser is better than a fast host-assisted fake browser.

A smaller functioning AI plan is better than a powerful unrestricted agent.

A simple scheduler that is easy to verify is better than a sophisticated scheduler full of hidden failure modes.

---

# 97. Project identity

Project name:

# **Nagi OS**

Japanese reading:

# **凪**

Product direction:

> **A quiet, local-first AI operating system.**

The intended feel is calm, spacious and low-noise, without leaning on superficial Japanese visual clichés.

The system should feel like a coherent independent computer environment where:

- files are understandable;
- history is recoverable;
- AI is local and useful;
- software is constrained;
- the owner remains in control.

---

# 98. Codex final instruction

Implement Nagi OS incrementally according to the milestone order in this document.

Do not attempt to "finish the whole OS" in one uncontrolled pass.

For each milestone:

1. inspect current repository state;
2. identify the milestone acceptance criteria;
3. implement the smallest architecture-correct increment;
4. build;
5. test;
6. diagnose failures;
7. repair failures;
8. rerun the acceptance test;
9. update documentation/status;
10. leave the repository in a clean, resumable state.

When uncertain, prefer the architecture and principles in this document over expedient compatibility hacks.

The primary success condition is not "many files were generated".

The primary success condition is:

> **At every stage, Nagi remains a real, testable operating system whose architecture is moving toward the 0.1 Definition of Done without hidden dependencies on another operating system.**
