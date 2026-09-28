# M18-C browser acceptance harness

This directory owns M18-C test fixtures, acceptance inventory, and case-level
reporting. It does not implement the browser or substitute a host browser for
the Nagi guest.

## Deterministic fixtures

`fixture_server.py` starts ephemeral HTTP and HTTPS servers bound to
`127.0.0.1` by default. It provides fixed HTML pages, a two-hop redirect, an HTTP-to-HTTPS
redirect, a redirect loop, status failures, malformed response, bounded delay,
connection-refusal coverage, and download/upload endpoints. The HTTPS
certificate and private key under `fixtures/tls/` are test-only localhost
credentials; the self-signed certificate is trusted explicitly by tests and
must never be used outside this fixture.

An isolated QEMU user-network run can use
`FixtureServer(bind_host="0.0.0.0", advertised_host="10.0.2.2")`; the fixture
certificate includes that guest gateway address. Keep that bind mode on an
isolated CI runner. Host tests use the loopback-only default.

Fixture contracts run with Python's standard library:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests -p 'test_m18c_browser_acceptance.py' -v
```

No public website or external network is contacted.

## Persistence and recovery inputs

`storage_fixtures.py` materializes deterministic input trees for missing,
empty, clean multi-tab, truncated, malformed, incompatible-version, partial
record, interrupted-write, and repeated-restore scenarios. These versioned
M18-C envelopes are fixture data only; they do not define Nagi's browser
storage schema or implement restore behavior. The guest adapter must map them
to the integrated M18 storage boundary before recording product evidence.

## Browser evidence contract

`cases.json` lists each guest-facing acceptance case and its owning
workstream. A browser integration test may emit a version-1 evidence bundle:

```json
{
  "schema_version": 1,
  "cases": [
    {
      "id": "navigation.direct",
      "status": "PASS",
      "evidence": "Nagi guest serial marker and final URL/title"
    }
  ]
}
```

The bundle must use an ID from `cases.json`, report `PASS`, `FAIL`, or
`BLOCKED`, and include concrete evidence. `BLOCKED` evidence maps to
`SKIP-BLOCKED` in the report and carries the command or dependency reason.
Unknown and duplicate IDs, missing evidence, and unsupported status values
fail validation. Cases without evidence remain `SKIP-BLOCKED` with their owner
and manifest reason; a blocked case can never become a pass by omission.
Overall status is `FAIL` if any case fails, `PASS` only when every listed case
passes, and otherwise `PARTIAL`.

Generate the current inventory report with:

```sh
python3 tests/browser_acceptance/report.py --output out/test-results/m18c-browser-acceptance.json
```

The host CI report includes the fixture-test outcome even when those tests
fail. The target job runs `feature_gates.py` against the actual M17 Cargo
feature graph and joins that evidence into a second report. The M18-on graph
remains blocked until the integration owner provides the shared M18 build
entry point. Host fixture tests and report generation do not claim guest
navigation, persistence, input, or transfer acceptance.
