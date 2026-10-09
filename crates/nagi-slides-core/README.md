# Nagi Slides Core — standalone host foundation

`nagi-slides-core` is a bounded, UI-independent presentation model using canonical
`nagi_model::ObjectId` and `nagi_history::activity::RevisionId`. It uses its own
workspace and lockfile and does not register a runtime service.

```rust
use nagi_slides_core::{native, Edit, Limits, Presentation, RevisionId};
# fn example(p: &mut Presentation) -> nagi_slides_core::Result<()> {
let expected = p.revision();
p.apply(expected, &[Edit::SetTitle("日本語 / English".into())], Limits::default())?;
let bytes = native::encode(p, Limits::default())?;
let reopened = native::decode(&bytes, Limits::default())?;
assert_eq!(*p, reopened);
# Ok(()) }
```

IDs are caller-supplied global canonical identities, never indexes, pixels or
host paths. Successful batches advance one revision; every error leaves the
snapshot, revision and issued-ID tombstones unchanged. Delete rejects dangling
internal navigation links; clear/delete referring objects in the same batch.
Duplicate takes fresh slide/object IDs, remaps copied internal targets, and keeps
external source identities/revision metadata. Reorder uses the final zero-based
index and preserves all identities and links.

Every intermediate snapshot must also fit `max_bytes`, including issued-ID
tombstones, before any edit content is cloned. A later delete cannot excuse an
oversized intermediate duplicate. Exact sizes use the same native writer in a
count-only mode without allocating an encoded buffer. The separate batch payload
budget counts explicit edit text plus the full native size of each dynamically
resolved duplicate source; repeated duplicate/delete cycles cannot evade it.
Duplicate transfers its one copied slide into the candidate without another clone.

Geometry uses integer logical millipoints (1/1000 point); display pixels/DPI do
not enter the model. Coordinates may be negative, bounded to ±1e9; positive
dimensions and their endpoints cannot exceed 1e9. Rotation is canonical clockwise
millidegrees in 0..360000; z-order is signed i32 (ties use stored object order).

Theme fonts/colors and slide size, named layout kinds, speaker notes, text/shape
payloads, source metadata and internal navigation are validated. Image/media/table
rendering, layout execution, grouping/snap, AI generation, presenter UI, guest
persistence, Activity/Wayback and live provider connections are deferred.

Native v1 uses `NAGISLD\0`, u16 version, little-endian scalars, u32 lengths,
UTF-8 strings and explicit tags. Maps and issued IDs are sorted. Decoder rejects
unknown versions, bad UTF-8, duplicate/noncanonical entries, unknown tags,
truncation, trailing bytes and invalid model structures. Every decode/encode is
byte/count bounded; tombstones survive reopen. This codec is an owned host format,
not a registered platform media type or a durable guest storage implementation.

Source revision detection is read-only through an injected `SourceRevisions`.
Changing the accepted baseline is an explicit `SetObject` transaction; no source
is fetched or automatically updated. Missing adapters and PDF/PPTX exports return
`AdapterUnavailable`. Metadata grants no authority and locators are never run.

Run from the repository root with nightly-2025-08-01:

```sh
bash crates/nagi-slides-core/verify.sh
```

First obtain the locked dependency cache with `cargo fetch --manifest-path
crates/nagi-slides-core/Cargo.toml --locked` if needed. Formatting is package-scoped
so canonical path dependencies do not expand into the shared root workspace.
