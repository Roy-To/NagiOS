# Writer Document Core — host contract v1

Standalone implementation authorized by BP-SBOM-HOST-20261008. No root
workspace registration, guest runtime, UI, filesystem, network, AI, clock or
permission service is provided.

`Document` stores metadata/settings, ordered leaf Sections/Blocks, styles,
comments, review proposals and issued identity tombstones. `Engine` owns
immutable in-memory revision snapshots. Call `apply(expected_revision,
operations, provenance)` for atomic updates, or `preview` for a pure candidate.
A failed operation leaves the document, history and issued IDs untouched.

IDs are caller supplied. `ObjectId` is **the existing nagi-model type**;
`RevisionId`, `Actor` and checkpoint IDs reuse **nagi-history::activity**.
This base has no canonical DocumentId: `DocumentId(ObjectId)` is an explicit
role adapter, compatible with Notes' NoteId=ObjectId. It carries no rights.
The tombstone set stores the canonical u64 values because this registered
base's ObjectId does not implement Ord. No shared definition was changed.
A native provider preserves all Document fields, especially IDs/tombstones,
when saving/opening. Opening an in-memory snapshot preserves identity but
starts a new local history window. It does not claim disk persistence.

Host revisions increase monotonically **within a Document**. Consumers pair
DocumentId with RevisionId. Globally allocated provider revisions and durable
history/branch migration require a reviewed platform adapter; these local
sequence IDs do not claim globally unique revision allocation or Wayback.

Text edit ranges are UTF-8 **byte offsets** checked at Unicode scalar
boundaries; they are not character counts or grapheme-cluster cursor positions.
The surface must map grapheme positions to byte ranges. No implicit language,
line ending or normalization changes occur during edits.

Sections belong only to the document root; Blocks are leaves in Sections.
Consequently cycles cannot be represented. Move indices refer to the sequence
**after removal**. Styles have single-parent inheritance; missing styles and
cycles are errors. Outline uses Heading level/content, independent of visual
style. Heading levels 4–6 inherit Heading3 by default. Table row/column/cell
editing replaces a validated rectangular table while keeping its Block ID.
Cell merge/split, spans and layout are deferred product features.

Comments retain their ObjectId anchor even after object deletion. Such threads
remain inspectable as orphan threads; they are not silently reanchored.
Review suggestions contain typed content operations and separate provenance.
Suggest/Accept/Reject must each be the only operation of its transaction.
Suggest validates a trial document without editing live content; Accept applies
it only at its recorded base revision, otherwise Conflict. Reject preserves
live content. Every unrelated revision invalidates pending accept/preview;
there is no automatic rebase or last-write-wins. Accepted review operations and
the review decision appear in ChangeSet; original proposer metadata remains
in TrackedChange and reviewer metadata in the committing ChangeSet.

Limits bound input bytes, document payload, issued IDs, styles, revisions,
operations, review payloads, comments and table cells. Reaching a limit fails
explicitly; no history or tombstone is silently discarded. Defaults are 1 MiB
payload, 4096 issued identities, 256 revision snapshots, 1024 operations and
16384 cells per table. Large production histories belong to the Store provider.

Markdown supports ATX Heading 1–6, paragraphs, flat ordered/unordered lists,
block quotes and backtick/tilde fenced code. Inline formatting/links, tables,
images, HTML, nested/indented syntax and malformed fences remain literal with
line-numbered UnsupportedWarning. Unsupported syntax is never executed or
fetched. Markdown export escapes literal punctuation and chooses safe code
fences. Table/reference/figure export retains text/source descriptions with
warnings; it does not claim those structures roundtrip. Plain text is literal
paragraph text with blank-line separators. Imports normalize CRLF/CR to LF.
Text formats create fresh identities; export always warns that native identity,
style, settings, comments and review metadata require a native snapshot.
`ExportResult` is a preview returned before any provider writes output; callers
must present warnings or reject a conversion before replacing source data.
No PDF/DOCX claim is made.

`adapters` defines version-1 Store/Permission/Activity/Checkpoint seams and an
authorized Search projection of title/headings/paragraphs. Private comments
and references are excluded. ActivityRecord contains only identity, revision,
actor/time and change categories, never document/comment text. Production
adapters must enforce injected permission, stale revisions and provider
failure/partial failure semantics. Unavailable adapters return Unavailable;
there is no restore implementation or fake recording/persistence success.
Citation/linked-reference values are untrusted ObjectId/resource/revision
references for Albert, Notes and Sheets; no cross-app adoption is performed.

Run the complete host gate from repository root:

```sh
bash tests/writer-core/verify.sh
```

The script runs offline/locked build,  host acceptance, rustfmt, warning-denied
Clippy and rustdoc. `./nagi dev verify` separately validates durable State.
Shared CI wiring belongs to the Integration Owner.
