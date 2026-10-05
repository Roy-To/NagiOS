# ADR 0037: UEFI realtime seed for verified TLS

## Status

Accepted for M18-A on 2026-09-28.

## Context

The Nagi realtime syscall currently reports elapsed timer ticks from Unix
epoch zero. Servo's Rustls/WebPKI verifier uses `SystemTime` to check each
certificate's validity period. At epoch zero, certificates issued in the
present fail their `notBefore` check, while certificates whose validity spans
epoch zero could be incorrectly accepted after their real expiry.

M18-A needs ordinary certificate validity checks for HTTPS. Disabling the
verifier, substituting host time or relying on the host certificate store is
not acceptable.

## Decision

- Before `ExitBootServices`, the UEFI loader reads the firmware RTC and
  converts it to Unix nanoseconds. A known UEFI timezone offset is applied;
  when the firmware marks its timezone unspecified, the Nagi QEMU reference
  machine treats the value as UTC (`-rtc base=utc`).
- UEFI daylight-adjusted RTC values are currently marked unavailable because
  the firmware fields do not provide enough context to reconstruct the UTC
  instant without guessing.
- BootInfo version 3 carries the sampled epoch. The kernel combines that value
  with elapsed guest timer ticks through the existing realtime syscall. The
  kernel makes no UEFI runtime call after boot.
- An unavailable RTC, invalid calendar value or unrepresentable timestamp is
  represented by `u64::MAX`. The realtime syscall preserves this unavailable
  value, and POSIX reports the clock as unavailable. TLS validation therefore
  fails closed when the guest has no usable time.
- Nagi does not consult a host OS clock or network time service. This is a
  firmware clock seed for the reference machine, not a claim of a secure,
  synchronized time service.

## Consequences

- Current HTTPS certificate validity checks can use the reference machine's
  firmware time while retaining normal chain and hostname validation.
- BootInfo's versioned loader/kernel contract changes from version 2 to 3 and
  the M17 QEMU acceptance must be rerun.
- Firmware with no usable clock leaves time-dependent HTTPS unavailable rather
  than silently weakening certificate validation.
