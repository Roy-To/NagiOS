# Service artifact preflight

This standalone Python standard-library tool checks an artifact already selected
by a trusted service catalog. It does not load, install, register, or launch a
service and it does not read or create catalog data.

Requires Python 3.10 or newer.

## Request contract

Input is one UTF-8 JSON object with exactly these fields:

```json
{
  "schema_version": 1,
  "artifact_path": "/path/to/service.elf",
  "expected_sha256": "<64 lowercase hexadecimal characters from the trusted catalog>",
  "target": "x86_64-unknown-linux-musl"
}
```

Missing, duplicate, or unknown fields are rejected. The digest must be supplied
by the caller from its trusted catalog. This tool never derives an expected
digest from the artifact or promotes a measured digest to a trusted value.

The validator opens the selected file by walking directory descriptors with
no-follow flags, rejects symlink path components and non-regular files, and
compares the entry and file identity/metadata before and after reading. It
validates ELF bytes read from that same descriptor, then hashes the file from
that descriptor. The ELF checks require ELF64, little-endian x86-64,
`ET_EXEC`/`ET_DYN`, a bounded complete program-header table, valid segment file
bounds, an executable `PT_LOAD` entry point, and no `PT_INTERP` program header.
Extended (`PN_XNUM`) program-header counts are rejected as unsupported.

## CLI

Read the request from standard input:

```sh
python3 tools/service-artifact-preflight < request.json
```

Or read it from a JSON file:

```sh
python3 tools/service-artifact-preflight --input request.json
```

Success is JSON on stdout with status `artifact_preflight_passed`; failures are
JSON on stderr and exit with status 2. The success response reports only the
preflight checks. It does not prove a signature, canonical package identity,
service role, user authentication, policy authorization, installation, or
runtime `READY` state. In particular, absence of `PT_INTERP` is the checked
static-loader condition; this result is not a package trust attestation.

## Python API

Add this directory to `sys.path`, then import `service_artifact_preflight`:

```python
from service_artifact_preflight import preflight_json

result = preflight_json(request_json_text)
```

`preflight_json` returns the same success object as the CLI and raises
`PreflightError` with stable `code` and `message` attributes on failure. It
closes the artifact descriptor after checking.

Callers that need to keep the exact validated object open can use:

```python
from service_artifact_preflight import open_verified_artifact

with open_verified_artifact(request_json_text) as artifact:
    launch_from_fd(artifact.fileno())
```

`VerifiedArtifact.fileno()` is the same descriptor whose bytes were checked.
The caller must use or pass that descriptor to its launch mechanism and must
not reopen `artifact_path`. The `with` block owns and closes the descriptor;
`close()` is idempotent. `detach_fd()` transfers ownership to the caller, which
must then close the returned descriptor. This preflight tool does not itself
launch the file. Its pathname checks only cover the validation interval; they
do not make a later path-based reopen safe or guarantee that another launcher
uses the retained descriptor.

## Host regression

```sh
python3 -m unittest discover -s tools/service-artifact-preflight/tests -v
```

The optional assigned service ELF can be provided through
`NAGI_SERVICE_PREFLIGHT_DEV_FIXTURE` and is read-only. It is a development
fixture only, not a production catalog value or product artifact input. For
example:

```sh
NAGI_SERVICE_PREFLIGHT_DEV_FIXTURE=/path/to/assigned-dev-service.elf \
  python3 -m unittest discover -s tools/service-artifact-preflight/tests -v
```
