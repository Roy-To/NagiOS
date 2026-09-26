# Nagi Notes Core and Host Preview

This crate implements the Notes document model, Markdown block codec, editor
session lifecycle, autosave coordination, search provider, Activity hook, and
storage adapter boundary.

The host preview is an interactive terminal surface for exercising Notes
without the Nagi desktop runtime. Start it with:

    cargo run --manifest-path user/nagi-notes/Cargo.toml --bin nagi-notes-preview -- --root /tmp/nagi-notes-preview --locale ja-JP

Use `quick <text>` for a localized Quick Note, `new <title>` for a blank note,
and `append`, `title`, `save`, `retry`, `search`, `open`, `close`, `delete`, or
`restore` to exercise its editor lifecycle. `show` prints portable Markdown.

The directory passed to --root is the complete host storage sandbox. Note
paths are derived from Nagi Object IDs; note commands cannot select arbitrary
host paths. Saved notes are Markdown files with a small YAML front matter
record for stable Object ID, timestamps, revision, workspace, and trash state.
Every save flushes a new immutable revision before atomically installing it as
a create-only revision entry. Existing revisions are never replaced. Exported
Markdown omits Nagi metadata and block identity markers.

HostPreviewStore is a host development adapter. It is not the Nagi filesystem,
does not provide production Storage capability, and is not target runtime
verification. InMemoryNoteStore is volatile and intended for tests and explicit
in-memory orchestration.

`NotesActionExecutor` requires an injected `NotesActionPolicy`; there is no
allow-all production policy. Agent search hits and referenced Object IDs are
checked for read authority before Notes returns or attaches them. Activity
events contain semantic operation, actor origin, revision, and workspace
context, but omit note titles and body text. The search provider can bind to a
live NotesApp so dirty open documents are immediately searchable.

The crate is an isolated Cargo workspace so focused Notes checks do not load
unrelated, partially materialized Servo/Mesa patch dependencies from the Nagi
root workspace. It has no third-party dependencies.
