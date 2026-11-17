# ADR 0069: M19 bundled Files Search client

Status: accepted for M19
Date: 2026-10-08
Builds on: ADR 0043 (isolated app process), ADR 0044 (authenticated Search IPC), ADR 0046 (Supervisor launch registry), ADR 0049 (signed package launch), ADR 0051 (user consent), ADR 0060 (trusted consent dialog), ADR 0068 (owner Files Search runtime)

## Context

M19 has a Files-only `search@1` handler over a Supervisor-owned Channel, but
the ordinary signed-in Files panel still searches `Runtime` directly. The
handler is therefore not yet exercised by a normal signed first-party app.
The desktop UI and Search runtime remain in init (PID 1); ADR 0043 deliberately
does not give isolated children display, input, or User Data capabilities.

## Decision

1. **One bundled first-party client.** Add a signed `org.nagi.files` client
   package for the M19 Files Search request. Its manifest requests only
   `search.query` and `files.search`. The normal desktop image receives this
   product `.xapp` through a product-package build input separate from
   `NAGI_ACCEPTANCE_PACKAGES`. The package is built and checked with the
   existing Developer Preview signing path. This is a built-in M19 client,
   not installation through the M16 Package Service.
2. **One request at a time.** When the signed-in Files panel submits a query,
   init synchronizes the dedicated owner Files directory, launches one client
   with a fresh `AppSessionId`, and sends the bounded query as a Files Search
   launch intent over that child's private Channel. The child sends one
   `search@1` File request over the same Channel and exits after decoding its
   response. Init serves that request on the child's endpoint, verifies clean
   exit, and reaps the launch before accepting another Files Search request.
3. **Caller identity and authority.** The child is the only Search caller.
   The service resolves the kernel-stamped sender PID through the Supervisor
   launch record; it never trusts identity fields from the client or labels
   PID 1 as an application. The signed manifest must request both capabilities
   and the live launch must hold explicit owner decisions for `search.query`
   and `files.search`. Search replies contain only bounded Object IDs. Init
   maps those IDs to owner-visible titles for the existing Files panel; the
   child receives no file contents, filesystem handles, or device capability.
4. **Owner consent.** If either capability is `ConsentRequired`, init queues
   the existing OS-owned consent dialog for the authenticated, unlocked owner.
   `Allow`, `Allow once`, `Deny`, and dismissal retain ADR 0051/0060 semantics.
   A query is sent only after both live grants are effective. Denial,
   dismissal, session loss, or an internal error cancels the pending request,
   closes/reaps the child, and leaves the UI without results. `Allow once` is
   scoped to the one child launch and expires when it is reaped.
5. **Process cleanup.** A client exists only for its one foreground query.
   Init reaps it after a result and on cancellation or error. No process is
   left as a background Search worker.

## Bounds and non-goals

- The existing desktop remains the trusted UI host and Search service host.
  This decision does not move the Files UI into a child process or add GUI IPC.
- This is not a general app registry, launcher, service registry, installed
  package loader, package-store integration, background job system, process
  restart policy, or lifecycle manager.
- The single client uses the existing init-only Supervisor and one private
  Channel per launch. It does not activate the gated Nagi 0.2 M13 Supervisor
  runtime or any other 0.2 workstream.
- Acceptance `.xapp` packages and fixture identities are not used by the
  ordinary desktop route.
- The pinned Developer Preview signing key remains a preview trust root;
  production key provisioning is unchanged.

## Acceptance

The focused guest acceptance boots an ordinary signed-in desktop image with
the signed product package, not the acceptance package bundle. It verifies:

- a real `org.nagi.files` ELF is launched as a child and the Search handler
  observes its kernel-stamped PID and matching live launch record;
- a submitted Files query reaches `search@1` and returns the exact visible
  Object ID, which init maps to the visible title;
- missing consent, denial, a query-only grant, a foreign app, and a revoked
  launch return no results;
- `Allow once` permits the current request and is absent after child exit;
  persisted `Allow` is restored for the signed-in owner after restart;
- no file contents, root-level metadata, or unrelated object IDs are exposed;
- the child exits and its launch/grants are revoked after success, denial,
  dismissal, or an error.

The M19 milestone remains `PARTIAL` until this guest acceptance and the other
M19 producer lifecycle acceptance criteria pass.
