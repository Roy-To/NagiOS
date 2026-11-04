# Writer ordered-list numbering (bounded Hark lane)

Base: main `084f2196a8a81cde08c6418cfebe2c5500290759` (PR #37 merge). Host-only.
Owned paths: `crates/nagi-writer-core/src/formats.rs`, `tests/writer-core/acceptance.rs`,
this directory. No model, registry, CI, root manifest or other workstream changes.

## Defect
`list_line` returned only `(ordered, text)`; the ordinal digits were dropped, and
`BlockKind::List { ordered, items }` (model.rs) has no field to hold them. Export
always writes `1.`, `2.`, ... So `9. item` imported as `List{ordered:true,["item"]}`
with **no warning** and exported as `1. item`. Same silent loss for gaps
(`1. a\n2. b\n4. c`), leading zeros (`01.`), `0.`, and a run resumed after a blank
line or another block (`1. a\n\n2. b` -> `1. a\n\n1. b`).
Evidence: `repro-before-fix.txt` (`REPRO import_kinds=[List { ordered: true, items: ["item"] }] import_warnings=[] export_text="1. item"`).

## Spec basis
- docs/workstreams/NagiOS_0.2_Writer_Document_Core_Workstream.md L71: import/export that may lose
  user data warns beforehand; unsupported features are not silently discarded.
- same, L100 (W4): unsupported content returns UnsupportedWarning and is kept/held/rejected;
  silent drop of important content is forbidden.
- same, L123 WRITER-H08: supported Markdown subset imports/exports/roundtrips correctly.
- same, L124 WRITER-H09: unsupported elements produce a warning or explicit error; no silent loss.
- docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md L77 (forbidden: silently discarding
  unsupported data on import), §57.2 L1755 (List is a model object), §57.14 L1906-1923
  (Markdown import/export; warn, never silently discard), WRITER-007 L1941.
- formats.rs module contract: "Unsupported syntax is retained literally and reported".
- writer-core-01 State WRITER-H09 PASS wording: "unsupported content literal preservation and line warnings".

## Choice: literal preservation + exact per-line warnings (no model migration)
The model cannot hold an ordinal, and a model change is out of scope. The supported
subset is therefore narrowed to what export can reproduce: an ordered run imports as a
List only when its markers are exactly `1.`, `2.`, ... `n.` (canonical decimal).
Otherwise the whole contiguous ordered run is kept byte-for-byte as a Paragraph, with one
`UnsupportedSyntax` warning per line (line numbers 1-based, as for other literal content).
Export then escapes it (`9\. item`), which re-imports as the same Paragraph with no warning,
so the number survives import -> export -> import. Markers are compared as text, never
parsed, so u64::MAX, u64::MAX+1, usize::MAX and 400-digit numbers cannot overflow or panic.
Canonical lists (incl. multi-digit 1..12, Japanese items, the bilingual fixture) are unchanged.

Rejected alternative: keep the List and only warn -> still rewrites the user's numbers on
export (warned loss), whereas literal preservation loses nothing.
Limitation: a non-canonical ordered run is no longer a list object in the document model
(it is literal paragraph text) until the model gains a start/ordinal field (owner decision).

## Tests (tests/writer-core/acceptance.rs, +8; total 39 = 31 original + 8)
h09_ordered_list_non_one_start_is_kept_literally_not_renumbered,
h09_ordered_list_gaps_and_resumed_numbering_are_not_lost,
h08_canonical_ordered_lists_still_roundtrip_exactly,
h10_huge_ordered_list_numbers_do_not_overflow_or_get_rewritten,
h09_mixed_ordered_unordered_runs_keep_every_number,
h08_escaped_list_markers_are_text_not_lists,
h10_japanese_ordered_items_keep_numbers,
h09_import_export_import_never_drops_an_ordered_number.
On unfixed main formats.rs: 6 of 8 fail (the two h08 guards pass on both) — `new-tests-on-unfixed-code.txt`.
After fix: `bash tests/writer-core/verify.sh` exit 0 (fmt check, locked build, 39/39 tests,
all-targets clippy -D warnings, rustdoc -D warnings, git diff --check) — `verify-after-fix.txt`.
