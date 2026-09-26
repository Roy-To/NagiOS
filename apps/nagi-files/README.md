# Nagi Files

This standalone package implements the M-APP-05 Files domain, operation
coordinator, provider contracts, in-memory backend, and sandboxed host preview.
It is intentionally outside the root Cargo workspace so this workstream does
not take ownership of shared manifests or generated runtime contracts.

The host preview is a development tool. It operates only below the explicit
directory passed as its sandbox root and only with rights named on `--allow`.
Its host grants are not Nagi production capabilities. A production Nagi adapter
must implement `FilesystemProvider` and use the public Nagi capability service.

## Run the host preview

The sandbox directory must already exist. Use a disposable directory when
trying destructive commands:

```sh
mkdir -p /tmp/nagi-files-sandbox
printf 'sample\n' > /tmp/nagi-files-sandbox/readme.txt
cargo run --manifest-path apps/nagi-files/Cargo.toml -- \
  /tmp/nagi-files-sandbox \
  --allow read,enumerate,create,write,rename,move,delete,restore,permanent-delete,metadata \
  --locale ja-JP
```

The preview does not execute shell commands or launch host applications.
`open <index>` reads a file through the sandbox provider and prints UTF-8 text
or a binary-size label. Activity and Wayback hooks report `Unavailable` unless
an adapter is supplied.

Commands include navigation (`ls`, `cd`, `up`, `open`), multi-selection
(`select`, `select-add`, `clear`), folder creation, rename, copy, move,
duplicate, Trash/restore, confirmed permanent delete, tags, metadata Search,
and Context snapshot display. Resource rows are numbered from 1 for commands
that take an item index. `help` prints the interactive syntax.

## Contracts and boundaries

- `FilesystemProvider` owns enumeration, metadata, file reads, create/copy/move,
  rename, Trash, restore, permanent delete, and tags.
- `FilesystemProvider::authorization_location` can resolve provider-specific
  path aliases before capability checks; virtual and case-sensitive providers
  keep their supplied spelling. The sandbox maps existing case-insensitive
  aliases to their actual directory-entry spelling using stable file identity
  and fails closed when it cannot resolve an alias unambiguously.
- Host providers can override `list_authorized`, `metadata_authorized`, and
  `read_file_authorized` to bind policy checks to provider-owned operations.
  The sandbox opens and verifies the directory before authorizing enumeration,
  captures metadata from the verified parent handle, and opens files before
  authorizing reads. Reads use the same verified file handle after the policy
  callback, so a raced path cannot redirect the operation to another resource.
  These handle-bound operations fail closed if the host filesystem cannot
  provide stable identities for the opened entries.
- `FilesService` checks scoped rights against that resolved location before
  returning ordinary resource data or performing mutations, filters Trash
  listings by each item's original location, verifies opaque resource IDs
  against current metadata, assigns
  transaction/action identities, validates confirmation bindings before
  consuming bounded one-use permanent-delete challenges, supports explicit
  user cancellation, and reports Activity/Wayback hook results.
- `FilesActionApi` provides typed local dispatch for every Files action ID and
  routes calls through `FilesService`. Registering it with Nagi's shared Action
  Registry remains a target integration task.
- Agent actions require an Activity sink to accept a `Started` record before
  the provider can mutate data. User actions remain available without Activity.
- The host sandbox reserves one internal `.nagi-files` directory for persistent
  Trash and tag metadata. Public filesystem operations traverse
  `cap-std::fs::Dir` handles opened from the selected root, so path resolution
  remains within that directory tree. It rejects path traversal, selected-root
  symlinks, static symlink paths, and case-insensitive aliases of its reserved
  metadata path; recursive copy never follows a symlink. On Unix, it also checks
  the opened root against the selected path at startup, and checks the held
  metadata-directory handles against their in-sandbox entries before metadata
  writes and Trash operations. Permanent delete records a versioned journal
  containing the affected resource IDs before writing tag removal or deleting
  the Trash payload. Startup completes an interrupted confirmed deletion;
  Trash listing fails closed while recovery is pending. This prevents failed
  Trash-index writes from leaving stale resource metadata on disk. This is a
  host preview backend, not isolation from a hostile process running as the
  same OS user or a production Nagi capability implementation. Resource IDs
  use device/inode metadata on Unix and volume serial/file index metadata on
  Windows; when a non-Unix filesystem cannot provide stable identity metadata,
  the provider falls back to a path-derived ID whose value can change after a
  move.
- Search returns filename, location, kind, size, modified-time, and tag
  metadata. It checks read permission per returned resource and skips denied
  resources without revealing their names.
- Context, Workspace, Activity, Wayback, and production Nagi filesystem
  connections are typed adapters. The first-party shared runtime services are
  not implemented by this Files branch.
- File previews are limited to 1 MiB. Providers receive the byte limit and the
  service rejects providers that return more than the requested limit.

## Verification

Focused package tests cover the in-memory operation model and real temporary
sandbox filesystem. No user files are used by tests.

```sh
cargo test --manifest-path apps/nagi-files/Cargo.toml --offline
cargo clippy --manifest-path apps/nagi-files/Cargo.toml --all-targets --offline -- -D warnings
cargo fmt --manifest-path apps/nagi-files/Cargo.toml -- --check
```

If the host shell resolves a compiler for a different architecture than the
running macOS system, set `RUSTC` and `RUSTDOC` to the matching rustup-managed
tools and pass the matching installed macOS target. This changes only the
host test executable architecture; it is not Nagi target-runtime verification.
