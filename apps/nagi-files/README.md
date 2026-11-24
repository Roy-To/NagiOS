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
or a binary-size label; bare `open` uses the focused selected item. Activity
and Wayback hooks report `Unavailable` unless an adapter is supplied.

Commands include navigation (`ls`, `cd`, `up`, `open`), multi-selection
(`select`, `select-add`, `clear`), folder creation, rename, copy, move,
duplicate, Trash/restore, confirmed permanent delete, tags, metadata Search,
and Context snapshot display. Resource rows are numbered from 1 for commands
that take an item index. `save <text>` writes UTF-8 text to the focused selected
file and requires the explicit `write` right. `help` prints the interactive
syntax.

### Interaction and display crosswalk

| User action or label | Current Files surface | Evidence and limits |
| --- | --- | --- |
| Open text from a listed row | In the host preview, `select <index>` selects a one-based row and bare `open` reads the focused item; `open <index>` reads that row directly. The component API selects by `ResourceId`, not filename text. UTF-8 contents print as text; binary contents print a byte-count label. | `preview_input_open_reads_and_displays_file_with_operation_result` and `host_component_file_info_matches_long_japanese_open_bytes` cover row/name-to-resource selection and returned contents. The preview command does not accept a filename as its selector. |
| Read-only file | `EntryKind::File` is the resource kind; read-only is not an `EntryKind`. `EntryAvailability::ReadOnly` is provider metadata (the host sandbox derives it from filesystem permissions), while each operation is separately authorized. A read-only capability scope can open a file but cannot save it without `Write`. | `run_data_profile_files_root_is_reference_only_and_reopens_without_cross_profile_changes` confirms open succeeds, save is denied, the message is drawn, and bytes stay unchanged in a synthetic host fixture. `draw_three_pane` does not render the availability field as a read-only badge. |
| Files app panel | The public `FilesComponent::draw` / `FilesCanvas` contract draws rows and Inspector metadata (name, kind, size, location) onto a caller-owned canvas. `open_selected` returns the file bytes to its caller; the canvas currently draws a Preview tab label, not the file body. | `host_component_file_info_matches_long_japanese_open_bytes` checks row and Inspector name/size plus returned bytes. This is host component evidence; the normal product Shell caller and an integrated Files panel are not present in `user/nagi-init`. |

## Contracts and boundaries

- `FilesystemProvider` owns enumeration, metadata, file reads, create/copy/move,
  rename, Trash, restore, permanent delete, and tags.
- `FilesService` checks scoped rights before returning ordinary resource data
  or performing mutations, filters Trash listings by each item's original
  location, verifies opaque resource IDs against current metadata, assigns
  transaction/action identities, enforces one-use permanent-delete
  confirmation, and reports Activity/Wayback hook results.
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
  writes and Trash operations. This is a host preview backend, not isolation
  from a hostile process running as the same OS user or a production Nagi
  capability implementation.
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

## Linux guest consumer

The optional `nagi-files-linux-guest-smoke` binary is a narrow Linux-target
consumer of the existing `FilesApp`, `FilesService`, and `SandboxProvider`
path. It uses only root-scoped `Enumerate` and `Create` grants, starts with a
new child directory under `/tmp`, enumerates it, creates one folder, reopens
the provider, and verifies that a refreshed Files view sees that folder.

Build it with an installed rustup compiler that contains the Linux target:

```sh
cargo build --manifest-path apps/nagi-files/Cargo.toml --offline \
  --target x86_64-unknown-linux-musl \
  --features linux-guest-smoke --bin nagi-files-linux-guest-smoke
```

This verifies the Linux filesystem consumer path in a Linux guest. It does not
connect Nagi login, the production capability service, or a production Files
UI; the existing host preview and this smoke consumer remain development tools.

## Known limitations for the Linux 0.1 Files UI

The Files name editor does not support entering Japanese file or folder names.
Notes keeps its saved notes in a private app store outside the Files root.
Exporting a copy from Notes to Files is a candidate for 0.2.
