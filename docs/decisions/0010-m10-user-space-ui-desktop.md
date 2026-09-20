# M10 User-Space UI/Desktop Decision

**Milestone:** M10

**Decision:** Implement the first toolkit, font path, Japanese text path,
widgets/layout, and four GUI apps as bounded user-space modules inside the M9
bootstrap Window Server. Keep the kernel display/input ABI unchanged.

**Reason:** M9 has a real Surface VMO, scanout, raw input, and compositor, but
the repository does not yet have general process spawning or a user-space
service transport capable of launching separate app processes. In-process
application clients preserve honest guest execution and allow each app to own
state and focus handling without adding high-level kernel syscalls.

**Alternatives considered:**

1. Adding `window_create` and app syscalls to the kernel would violate the
   kernel boundary.
2. Rendering four host-side panels or checking host-generated strings would
   not prove Nagi UI execution.
3. Waiting for the future process model would leave the M10 acceptance
   incomplete and provide no usable desktop surface.

**Consequences:** M10 proves simultaneous user-space GUI clients and the
Japanese text path, while separate address spaces/process identities remain a
later milestone concern. No security authority is expanded: all drawing and
input routing remain within the existing display/input capabilities.
