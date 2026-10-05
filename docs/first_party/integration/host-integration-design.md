# First-Party Host Integration Design

## Scope

`tests/first-party-integration` is an isolated host-preview and cross-app
contract harness. It depends on the four merged first-party workstreams and
uses their real package APIs. It is intentionally outside the Nagi root Cargo
workspace so it can run without fetching the separately pinned Servo/Mesa and
`cc-nagi` source inputs. A host-preview PASS never implies a Nagi target PASS.

## Shared contracts

- `nagi_model::ObjectId`, `WorkspaceId`, `AppId`, and `TransactionId` remain
  canonical. Notes uses canonical object/workspace IDs directly.
- Files' `ResourceId(u128)` is not narrowed to `ObjectId(u64)`. The host
  adapter uses an explicit, in-memory, bidirectional resolver for the lifetime
  of the preview. A target adapter must use the OS object resolver.
- Home/Search owns result presentation and typed activation. Activity events
  and Wayback checkpoints use their source `EventId` and `CheckpointId` types
  through dedicated typed actions.
- Notes' injected action policy, Files' per-location capability authorizer,
  and Activity/Checkpoint visibility policies remain separate authority
  checks. Search UI capability context is only an additional presentation
  filter.
- Host Activity context is explicitly authenticated as a preview user. Agent
  events are rejected until the producer supplies valid delegated provenance
  and authority evidence.

## Providers and state

- Notes search calls `NotesSearchProvider::search`, then applies the injected
  Notes `Get` policy before it constructs a candidate. It never reads the
  provider's unfiltered `records()` list.
- Files search calls `FilesSearchProvider` over the actual `FilesService`,
  which applies the configured `CapabilitySet` to enumeration and metadata.
  Resource IDs are resolved to canonical object handles without truncation.
- Activity and Wayback search query the shared in-memory stores through their
  mandatory visibility policies, render summaries in `en-US` or `ja-JP`, and
  return typed event/checkpoint actions.
- Notes save events append canonical Activity records. For persisted note
  revisions, the host preview stores opaque references back to the real
  `NoteStore::load_revision` snapshots, then creates a canonical revision and
  document checkpoint. Snapshot bytes stay in the note store and never enter
  Activity metadata.
- Files Activity events enter the same ledger. Before a reversible operation,
  the host service checks `Read` on the source, verifies the resource handle,
  and sends a bounded (16 MiB maximum) regular-file snapshot to the checkpoint
  hook. The adapter stores bytes in an in-memory Wayback snapshot backend,
  creates canonical revision/checkpoint records, and links the checkpoint to
  the operation event. Snapshot bytes never enter Activity or Search. Folder,
  multi-object, source-less, unreadable, unverifiable, and oversized snapshots
  fail truthfully without creating a checkpoint; the underlying operation
  still reports its own outcome. This is host preview evidence only, not a
  target Files version store or restore path.
- The Home registry is a canonical `RegistrySnapshot` containing the actual
  Notes and Files app IDs. Their launch status is `HostPreview`; it does not
  claim a target app runtime.

## Target boundary

Target persistence, authenticated target context, capability-service
adapters, OS object resolution for Files, durable Files snapshots, native
application launch, and checkpoint restore into target storage are not
implemented by this host harness. Record these as `BLOCKED` or `NOT RUN` in
the integration state. Do not reinterpret host preview evidence as target
acceptance.
