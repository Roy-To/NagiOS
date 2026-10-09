# Hark handoff: clipboard decode pre-allocation bound (host-only)

Owner of the code: CLIP-01 (`.dev/workstreams/clip-01`, status PASS). This lane is a
bounded defect fix on `hark/clipboard-decode-limits`, base main 084f219. Not registered
in `.dev/workstreams.json` (no shared registry edits); no state.json is written here.

## Defect
`decode_content(input, limits)` never calls `limits.validate()`; `ClipboardLimits` fields
are public and no documented precondition requires validated limits. With
`max_items`/`max_representations_per_item = usize::MAX`, a 16- or 20-byte envelope declaring
`u32::MAX` items/representations reached `Vec::with_capacity(count)` before any payload
bytes existed. `ClipboardService::new` validates limits, but the service never decodes;
the only callers of `decode_content` on main are the acceptance tests.

Runtime (release, `ulimit -v 524288 -t 30`, base 084f219): items case aborted with
"memory allocation of 103079215080 bytes failed" (SIGABRT, exit 134); representations
case 240518168520 bytes, exit 134.

## Fix
`Vec::with_capacity` for items and representations is capped at
`min(count, remaining_input / minimum_record_size)` (4 bytes per item, 10 per
representation). No API, error-variant, format, or accepted-data change. Under loose
limits the same inputs now return `Err(DecodeError::Truncated)`.

## Left for the owner
Whether `decode_content` (and `encode_content` / `ClipboardContent::validate`) should
also reject limits that fail `ClipboardLimits::validate()` is a contract decision — doing
it would change results for callers passing out-of-cap limits and needs an error mapping
(no existing `DecodeError` variant fits exactly). Not done here.
