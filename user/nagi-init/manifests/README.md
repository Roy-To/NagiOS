# Acceptance application package manifests

These are M16 package manifests (`id`, `name`, `version`, `entry`, and
optional `grant=` lines). `./nagi` packages each manifest with its isolated
ELF into an `.xapp` and signs it with `nagi-pkg build-signed`. The
Supervisor launches an application only from a package with a valid
signature. It takes the application identity and grants from the signed
manifest (ADR 0049).

| Manifest | ELF | Grants |
| --- | --- | --- |
| `org.nagi.acceptance.isolated-app` | `nagi-isolated-app` | none |
| `org.nagi.acceptance.faulting-app` | `nagi-faulting-app` | `acceptance.consent-probe` (ADR 0051/0060 consent probes) |
| `org.nagi.acceptance.m19-search` | `nagi-m19-search-client` and `nagi-action-client` | `search.query`, `files.search` |
| `org.nagi.acceptance.m22-files` | `nagi-action-client` | `files.move`, `files.copy` |
| `org.nagi.acceptance.foreign-client` | `nagi-m19-search-client` and `nagi-action-client` | `search.query` |
