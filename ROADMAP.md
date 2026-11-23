# Nagi OS Roadmap

This roadmap describes target scope, not a release schedule or a claim that a
feature is present. Acceptance results are recorded in
[`docs/implementation_status.md`](docs/implementation_status.md) and the owning
workstream documents.

Nagi OS is a Nagi-owned userland running on the Linux kernel
([DEC-123](docs/product-decisions/DEC-123-kernel-unification-and-linux-0.1.md)).
Development of the earlier custom kernel has ended. Its milestone records
M0–M30 are kept as history and are not used as acceptance evidence for the
Linux line.

## 0.1 Developer Preview (Linux)

The milestone boundaries and names below are provisional (DEC-123).

| Milestone | Intended outcome | Status |
| --- | --- | --- |
| L0–L6 | Linux x86-64 QEMU boot, Nagi PID 1 and service lifecycle, Identity and sessions, Notes save/restart/read, read-only System and writable Data, measured input latency. | Development-image acceptance passed in [PR #163](https://github.com/Roy-To/NagiOS/pull/163). |
| L7 | Desktop basics: a shell after login, launching and switching apps. | Not started |
| L8 | Files. | Not started |
| L9 | Settings, including English and Japanese. | Not started |
| L10 | Calendar. | Not started |
| L11 | Release build hardening: login, Data and Notes paths in release PID 1, Data selection without the kernel command line, a verified boot chain, a pinned kernel, app isolation, and a security review. | Not started |
| L12 | Release artifacts: QEMU image, checksums, source revision and build manifest, notices, reproducibility evidence, and documentation. | Not started |

The 0.1 scope is intentionally narrow. A `PARTIAL` result or a host-only test
does not satisfy guest acceptance. Performance targets follow
[DEC-116](docs/product-decisions/DEC-116-performance-targets-and-minimum-requirements.md).

## Later versions

- **0.2:** Albert browser, local AI runtime, voice, semantic search, A/B boot and
  recovery. Activation requires the Linux 0.1 Developer Preview to be published
  (DEC-123).
- **0.3:** Personal Agent.
- **0.4 and later:** see the [product decisions](docs/product-decisions/README.md)
  and their roadmaps.

No release date is promised by this roadmap.

## Related guides

- [Developer onboarding and diagnostics](docs/developer-preview/README.md)
- [SDK surface and sample workflow](sdk/README.md)
- [Contribution workflow](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Third-party notices](THIRD_PARTY_NOTICES.md)
