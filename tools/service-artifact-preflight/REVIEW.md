# Review record

## Scope

Standalone standard-library Python validator and host regressions for the
catalog-selected service artifact contract. No catalog values are created or
read, and no service is launched. The assigned fixed ELF is a read-only
development fixture, not production catalog data.

## Verification commands and results

- `python3 -m unittest discover -s tools/service-artifact-preflight/tests -v`
  with `NAGI_SERVICE_PREFLIGHT_DEV_FIXTURE` set to the assigned read-only dev
  ELF: **21 passed**. Covers valid ELF, bad digest, symlink file and directory
  components, directory and missing path, missing/duplicate/unknown JSON
  fields, wrong target/architecture, truncated header, out-of-bounds program
  header table and segment, `PT_INTERP`, in-flight path replacement, and the
  retained-descriptor close/path-replacement contract.
- `python3 tools/service-artifact-preflight` with a schema-v1 request for the
  assigned dev fixture and its supplied SHA-256: **exit 0**,
  `artifact_preflight_passed`, `ET_DYN`, 10 program headers, no `PT_INTERP`.
  This fixture remains explicitly a development input.
- Python `compile()` over every `*.py` file under this directory: **passed**.
- `entire session current`: Entire reported no session state for the current
  Codex session. The result is unrecorded; no Entire setting was changed.

## Boundaries

Success is artifact preflight only. It does not establish signature validity,
canonical package identity, service role, user authentication, policy
authorization, installation, or runtime `READY`. A later pathname reopen is
outside the guarantee. A library caller can retain `VerifiedArtifact.fileno()`
and must pass/use that same descriptor through its launch mechanism; the
context manager closes it, or `detach_fd()` transfers close responsibility to
the caller.
