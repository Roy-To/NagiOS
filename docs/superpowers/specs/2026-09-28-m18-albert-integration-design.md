# M18 Albert Browser Integration

## Problem and outcome

The M17 PASS baseline boots Servo on a local data page. M18 requires the guest
to work as a basic browser and render several real HTTPS websites. The active
M18-A and M18-B workstreams separately build transport and browser UI/state.
This main workstream owns the guest launch path and assembles their results into
a Servo-to-chrome-to-Nagi-Surface runtime and end-to-end acceptance.

## Current behavior

At fixed baseline SHA 94e9a027618182b10c0ac2315e94673543f22423, ./nagi m17
builds nagi-init with m17-servo and accepts one local first-web-pixel frame.
There is no ./nagi m18 command or multi-site HTTPS acceptance path.

## Scope

- Preserve the existing M17 command and acceptance unchanged.
- Add an M18-specific target build and QEMU acceptance path.
- Require three distinct HTTPS hostnames to complete with explicit verified
  TLS evidence, browser chrome presentation, and a nonzero Servo frame checksum.
- Feed each guest navigation through M18-B's typed BrowserState, record
  completion/redirect/title state, compose the chrome view over the real Servo
  RGBA frame, and present the composed frame through Nagi Surface.
- Route the granted Nagi input capability through Albert's chrome hit testing
  and action dispatcher. The end-to-end gate enters the first HTTPS address by
  clicking and typing into the rendered address bar; later hosts exercise the
  same typed navigation and Servo path.
- Keep transport and browser-state modules behind their typed interfaces while
  the main runner owns their end-to-end composition.
- Record integration state and run host checks before target acceptance.

Out of scope here: implementing raw sockets/TLS, browser chrome/state models,
changing the M17 acceptance, or marking M18 PASS before real QEMU evidence.

## Design

Use an M18 guest marker contract consumed by both a focused host validator and
the QEMU acceptance wrapper. QMP injects pointer and key events through the
VirtIO input device after the guest emits the browser-ready marker:

    Nagi M18 browser READY
    Nagi M18 browser input navigation PASS host=example.com
    Nagi M18 HTTPS TLS PASS host=<host> chain=verified hostname=verified
    Nagi M18 HTTPS page RENDERED host=<host> frame_checksum=0x<hex>
    Nagi M18 browser scenario complete pages=3

The validator accepts only the three configured HTTPS hosts
(example.com, example.org, and example.net), requires each to have verified TLS,
chrome presentation, and a nonzero checksum, rejects duplicate or out-of-order
evidence and guest failure markers, and checks that the summary follows all
page evidence. Guest code must update the typed browser state after Servo reports
completion, then emit evidence only after the trust verifier accepts the chain
and hostname, the real page frame is read back, browser chrome is composed, and
the capability-checked Nagi Surface presents the combined image.

Add a separate ./nagi m18 path using the same pinned Servo/Mesa and QEMU
infrastructure while retaining ./nagi m17 as the regression gate. M18 uses
its own image, persistent disk, variable store, and log names so M17 artifacts
and evidence remain intact.

## Trade-offs

A separate M18 runner duplicates some M17 setup but reduces risk to the already
passing M17 flow. Once the common build/run path is understood, share only
helpers whose behavior can be kept identical and covered by existing tests.
Reserved example domains keep the initial smoke list stable; M18-A still needs
independent deterministic fixture tests for DNS, HTTP, TLS errors, redirects,
timeouts, and reset behavior.

## Verification

- Unit-test the M18 guest-log validator with complete, missing, malformed,
  unverified, zero-frame, and failure evidence.
- Test m18 command parsing and its build/acceptance contract.
- Test bounded evdev address entry and chrome hit testing; QEMU must navigate
  the first HTTPS host through the address bar before accepting page evidence.
- Run focused host tests for the browser state, chrome renderer, and serial
  acceptance validator; check formatting, Clippy, and git diff --check.
- Build the Nagi target and run QEMU once transport/TLS and the guest launch path
  are wired on the supported Ubuntu target toolchain.
- Do not report M18 PASS until real QEMU evidence proves several HTTPS pages
  reached the guest Servo renderer and Nagi Surface.


## Current integration boundary (2026-09-28)

The main runner now consumes M18-B's BrowserState and chrome renderer, updates
per-tab navigation/history state from actual Servo completions, and presents a
composed real-page-plus-chrome frame. The guest routes Nagi pointer and key
events through a bounded adapter; the QEMU gate clicks the Albert address bar,
enters `example.com`, and requires the resulting HTTPS navigation to complete
before the remaining two-site sweep. Pointer and keyboard events below the
chrome are forwarded to Servo. The host validator requires the address-bar
navigation marker and per-host TLS, chrome-presentation, and rendered-frame
evidence.

The target runner now owns one Servo WebView per `BrowserState` tab, shows the
active tab's view, hides inactive views, and drops a view when its tab closes.
Tab-specific navigation and page input route through the matching view. The
three-site acceptance still navigates sequentially in the selected tab; new-tab
creation, switching, and closing have not yet been exercised in QEMU. Browser
session/history/bookmarks now save and restore through a pathless Nagi POSIX
snapshot service backed by the guest VFS. Its ABI is enabled only by the M18
feature, and CI asserts that M17 excludes it. The current VFS limits each file
to 1 KiB, so larger aggregate state reports a capacity warning without
blocking navigation. The acceptance runner defers restored page loads until
address-bar input, preserving the requirement that the first HTTPS request
comes through the QMP input path. Downloads, uploads, clipboard, IME service
events, and site permission models still lack runtime service adapters. M18
now includes M18-A's nonblocking POSIX socket and smoltcp transport code plus
the UEFI realtime seed in the same target feature. The runner explicitly keeps
Servo certificate errors enabled and records the normal verifier result.
Real-site DNS and TLS behavior passed in the M18 Ubuntu target/QEMU run
recorded below; M18-A's standalone controlled-fixture runner is not part of
this main acceptance.

Initial local macOS target attempts stopped before M18 guest code compilation:
SpiderMonkey and Mesa target-link probes used GNU ELF flags through Darwin's
native linker. The follow-up below adds a Darwin-only ELF linker adapter and
records successful local target build and QEMU acceptance. At this earlier
checkpoint, Ubuntu target build and real-site QEMU acceptance were still
required; the corrected-patch CI result below now confirms them.

## Follow-up verification (2026-09-29)

The Darwin host-link failure is resolved with a host-specific target linker
adapter. `tools/mesa/build.sh` adds a Meson linker override only on Darwin;
that override applies to Nagi target links, while build-machine tools continue
to use their native host toolchain. The adapter invokes ELF LLD directly,
filters Darwin-only driver arguments, and passes Mesa's existing `-latomic`
probe through unchanged. Compile-only target calls and host build helpers are
unchanged. Ubuntu CI continues to use the tracked Clang/LLD cross file; CI run
`36517686132` passed the Mesa build, target image, M17 QEMU regression, and
three-site M18 HTTPS/QEMU acceptance on Ubuntu. Fresh local `./nagi m18` and
`./nagi m17` also pass on macOS after the adapter was added.

The subsequent Servo requester-origin patch is applied reproducibly from
`third_party/servo-patches/0025-nagi-m18-permission-origin.patch`. It carries
the origin of the requesting document, including opaque `null`, through to
Albert. Albert records and denies the request without granting authority. Its
local M18 acceptance passes. The first fresh CI run (`36530525632`) found
that the second hunk did not apply to pinned clean `webview_delegate.rs`
because its blank-line context was too strict. The hunk now anchors on the
existing `feature()` accessor and passes sequential application against a
fresh pinned-source fixture. Corrected-patch local M17 first-web-pixel
acceptance also passes. Corrected commit
`eb22702da8e832126c32e420c8fde579b05f8a67` then passed full CI run
[`36533931477`](https://github.com/RT-NISH/NagiOS/actions/runs/36533931477):
clean Servo bootstrap passed on Windows and Ubuntu, and the Ubuntu target job
passed Mesa Softpipe, M17 QEMU, M18-B chrome, and the three-site M18
HTTPS/QEMU acceptance.
Download destination, capability-safe upload selection, shared clipboard,
IME text/composition events, and a trusted interactive permission service
remain unavailable because the repository has no user-space IPC/service
providers for them. Their browser hooks continue to fail closed, and M18's
overall workstream remains `PARTIAL` while the formal HTTPS/QEMU Acceptance is
`PASS`.
