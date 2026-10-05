# ADR 0030: Nagi TLS bootstrap roots for Servo

## Status

Accepted for the M17 Servo bootstrap on 2026-09-27. M17 remains `BLOCKED`
until the real guest first-web-pixel acceptance passes.

## Context

CI #266 (`36281382815`, head `fdc1b24610fa36ee9651283eaf77f7477820fc9f`) verified
the ELF constructor repair: the guest completed constructor dispatch, M7
persistent-storage acceptance, SpiderMonkey `TypeIdSet` insertion, and
`JS_Init`. Servo then aborted its `ResourceManager` thread while constructing
the Rustls platform verifier because Nagi has no host-platform CA bundle.

The Nagi specification says Nagi owns its trust roots. A Nagi guest must not
read the development host's certificate store. The pinned Servo workspace
already includes `webpki-roots` 1.0.9 in `Cargo.lock`, with a verified crate
checksum, and Servo already supports using that root set with its real
Rustls/WebPKI verifier. Its certificate-path override is additive when that
path is supplied.

## Decision

For `target_os = "nagi"`, select Servo's existing WebPKI verifier and the
version-locked `webpki-roots` set as the bootstrap system-root baseline. Do
not instantiate `rustls-platform-verifier` or inspect host certificate paths
for Nagi. Keep normal certificate-chain and hostname validation enabled.
Continue to merge any explicit certificate-path override through Servo's
existing root-store path.

This is a real verifier with a pinned set of public trust anchors. It is not a
certificate-validation bypass. The initial Nagi Servo embedder does not yet
provide enterprise or user roots; this ADR does not claim that those
trust-store layers are implemented. Changes to the bootstrap root set require
an explicit locked dependency update and review of this decision.

## Consequences

- Servo resource-thread startup no longer depends on a host OS trust store.
- HTTPS verification uses the checked-in Cargo lock's WebPKI roots.
- The M17 local-page acceptance can initialize Servo's network subsystem
  without requesting network access or weakening TLS validation.
- The next public target run must verify constructor completion, successful
  resource-thread creation, and the next real guest milestone.
- M17 remains `BLOCKED`; M18 remains `NOT STARTED`.
