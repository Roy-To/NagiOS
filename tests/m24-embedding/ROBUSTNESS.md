# M24 embedding provider: artifact robustness notes

Scope: the structural validator and tokenizer of `crates/nagi-embedding-provider`
when an artifact is loaded **unpinned** (`expected_artifact_sha256: None`). The
default configuration pins the artifact digest, so a hostile artifact does not
reach these paths in production; these limits make the validator hold its
"malformed input yields `ModelError`, never unbounded work" contract by itself.
All evidence below is host-only. Guest inference is not run; M24 remains PARTIAL.

## Structural limits

| Limit | Value | Pinned model | Failure |
|---|---|---|---|
| `n_pieces` vs pieces section | `n_pieces <= pieces_len / 8` | 250,002 pieces, 4,331,359 B | `Malformed("n_pieces")` |
| Charsmap replacement (NUL-free run) | `MAX_NORMALIZED_REPLACEMENT_BYTES = 64` | 33 B | `Malformed("charsmap")` |
| Normal piece length | `MAX_NORMAL_PIECE_BYTES = 64` | 48 B | `Malformed("piece_len")` |
| Piece-table probes per insertion | `MAX_PIECE_PROBES = 128` slots | 28 | `Malformed("piece_table")` |
| Piece-table probes in total | `max_table_build_probes(n) = 4n + 128*128` slots | 361,886 of 1,016,392 | `Malformed("piece_table")` |
| Piece-table lookup | at most the table's longest stored probe (<= 128) | 28 | n/a (returns "absent") |

## Piece hash table (FNV-1a bucket collisions)

The tokenizer maps piece text to piece id with an open-addressing table
(capacity `(2 * n_pieces).next_power_of_two()`, linear probing) keyed by a
fixed, unkeyed 64-bit FNV-1a. An artifact author can choose piece texts whose
hashes share one bucket; before the bounds, `k` such pieces cost `k*(k+1)/2`
slot visits to insert and every missing-key lookup whose bucket fell in the run
walked all of it (the Viterbi pass looks up every substring of every word up to
the longest piece, so this multiplies into encode time).

Colliding keys are cheap to construct: the low `k` bits of FNV-1a depend only on
the low `k` bits of the state and the prime is odd, so the last byte of an
8-byte printable-ASCII key can be solved for any target bucket
(`colliding_keys` in `adversarial.rs`).

### Reproduction, kept separate by method

**Arithmetic (exact-hash model, no tokenizer run).** 1024 distinct 8-byte
pieces in one bucket of a 4096-slot table: `1024*1023/2 = 523,776`
occupied-slot comparisons to insert, `1024 + 1 = 1,025` slots for a missing
lookup. `unbounded_table_cost` in `adversarial.rs` recomputes this for 1024 and
4096 colliding pieces on every test run.

**Runtime, before the fix (head `c4d4220`).** A temporary, uncommitted build of
the same tokenizer with probe counters, `cargo test` (opt-level 3), aarch64
host, `-j1`; base fixture pieces plus `n` colliding pieces. Wall times are
indicative only.

| Colliding pieces | Table slots | Slots examined to build | Occupied comparisons | Missing lookup | `Tokenizer::new` |
|---:|---:|---:|---:|---:|---:|
| 1,024 | 4,096 | 524,807 | 523,776 | 1,025 | ~6.4 ms |
| 4,096 | 16,384 | 8,390,663 | 8,386,560 | 4,097 | ~116 ms |
| 16,384 | 65,536 | 134,231,348 | 134,214,957 | 16,386 | ~1.6 s |
| 32,768 | 131,072 | 536,887,303 | 536,854,528 | 32,769 | ~6.5 s |
| 65,536 | 262,144 | 2,147,535,665 | 2,147,470,122 | 65,538 | ~26 s |

The 1,024 row's 523,776 comparisons and 1,025-slot lookup match the arithmetic
exactly. The 65,536 case is a ~3 MB artifact; growth is quadratic, and the
format allows up to 1,000,000 pieces.

**Runtime, after the fix** (`adversarial.rs`, model-free, runs in CI):

- `single_bucket_piece_collisions_fail_closed_after_bounded_work`: 1,024,
  4,096 and 65,536 colliding pieces each fail closed with
  `Malformed("piece_table")` after examining 8,392 slots (bound asserted:
  `<= 129*130/2 + 2*128`), ~0.07 ms / ~0.17 ms / ~2.4 ms including artifact
  parsing; the provider load returns the same error.
- `many_collision_runs_under_the_per_piece_bound_hit_the_total_budget`: 16 runs
  of 100 colliding pieces (each under the per-insertion bound; unbounded build
  would examine 80,936 slots) fail closed at exactly
  `max_table_build_probes(1611) + 1 = 22,829` slots.
- `missing_lookups_are_bounded_by_the_longest_stored_probe`: a 120-piece run
  within both bounds loads; every stored key is found, and missing keys whose
  bucket lies anywhere in the run examine at most the table's longest stored
  probe (<= 128).

### Why the bounds keep the pinned artifact well inside

Measured on the pinned artifact (sha256 `7fb0a345…a287`, 250,002 pieces,
249,997 normal, 524,288 slots), both by the arithmetic model
(Python replica of the insertion order) and at runtime
(`real_model_bounds_inputs` in `real_inference.rs` asserts the values):
longest insertion probe 28 slots and 361,886 slots examined in total (1.45 per
piece). The arithmetic model also gives the longest occupied run, 38 slots
(what an unbounded missing lookup could have walked; the bounded lookup stops
after 28). `MAX_PIECE_PROBES = 128` gives 4.5x headroom on
the per-insertion probe and the total budget 2.8x headroom on the build. Any
other legitimate SentencePiece vocabulary at load factor <= 1/2 expects ~1.5
slots per insertion.

### Remaining limitations

- The hash stays unkeyed: a hostile unpinned artifact can still force up to
  128 slots per lookup (vs 28 for the pinned table), i.e. at most ~4.6x the
  pinned lookup cost per substring during segmentation. Encode work stays
  proportional to input length × longest piece × 128.
- A legitimate vocabulary with pathological clustering beyond the bounds would
  be rejected rather than loaded slowly; no such vocabulary is pinned.
