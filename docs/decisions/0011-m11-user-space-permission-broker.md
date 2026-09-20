# M11 User-Space Permission Broker

**Status:** Accepted for M11

## Decision

Implement local accounts, sessions, lock state, Developer Mode, and resource
permission decisions in the user-space `libnagi` security module. Keep the
kernel responsible only for the capability checks it already owns. The M11
acceptance app runs the broker in the existing bootstrap user process and
proves denial of untrusted file and microphone requests.

## Rationale

The current preview has no general process-spawn ABI, no microphone service,
and no trusted UI process boundary. Adding high-level permission syscalls or
granting universal capabilities would violate the architecture. A fixed-size
user-space broker provides an executable policy boundary now; later services
can consume the same request/decision types without changing the kernel
contract.

## Security properties

- Password comparison is bounded and does not retain plaintext.
- Locked sessions cannot authorize requests.
- Untrusted apps are denied file and microphone resources.
- Developer Mode is Owner-authenticated and does not bypass those denials.
- Trusted foreground requests may return `Ask`; a later trusted dialog can
  resolve them without allowing an untrusted app to self-approve.
