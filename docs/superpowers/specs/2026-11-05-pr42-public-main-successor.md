# PR42 public-head and main successor

## Scope and design

Preserve public PR42 head d817c0a619fb035a65e8096016de4f89a2e1e36b, reviewed repair candidate 9f80873405595248a6ce9fe45dbdf7374f808c5c, and main c170ebf1d056aea277e3aa5f464d148d4ecff7fb after PR92, without force-pushing or rewriting the frozen candidates. Build the tree from the repair candidate and apply only main's changes since their common ancestor 0362427747e769b00558053e4c7f4c756d303802. This source successor remains a child of the frozen repair candidate. Before publication, an independently reviewed ancestry integration must retain both public head and main as ancestors; no merge or push is performed in this source-preparation stage.

Main adds required-file/change-scope CI checks, Calendar boundary assertions and historical evidence. Keep these exact paths and combine the independent implementation-status additions. Keep the candidate's Linux Recovery checks and nonzero libnagi/adapter test-count gate alongside main's CI gates. No production source behavior is changed by this integration. Strict Clippy exposed one pre-existing collapsible-if in delete undo validation; combining the two predicates preserves the exact fail-closed condition without adding lint allowances.

## Public-head feature accounting

The candidate retains the original Files/Trash UI, localizations, runtime integration, M27 inspection/explicit Recovery helper and bounded fixed-token storage diagnostics. login_screen.rs is identical to the public head. Runtime differences add RecoveryRequired tracing and identity-based managed-Trash visibility; storage tracing adds Indeterminate. The old writable-buffer adapter workaround is superseded by the kernel direction contract and readable-buffer adapter regressions. The ordinary mount helper was deliberately replaced by explicit recover_existing in the authorized Recovery helper. The broad diagnostic sanitizer and unrelated main work must not be rolled back.

## Verification and gates

Check exact source hashes, all three ancestry relations, main-only paths, combined workflow regression tests and full local diff. Run the pinned Mac host-only libnagi formal tests, standalone Recovery tests, exact-source Runtime/Files/Search harness, Calendar boundary tests and formatting checks. Mac's Linux-only adapter binary running zero tests is not a PASS for that suite. The prior candidate's Linux adapter 69/storage64 results are historical and are not this successor's run.

Remain PARTIAL: independent source review and Linux/hosted CI for this successor are pending. No Nagi target or guest/VM acceptance, real-media arbitrary persistence ordering, unknown existing sector-zero compatibility, or cross-process ownership is established. The separately denied process-shared ownership work is not retried. Existing replace/write partial-update and damaged legacy journal limitations remain. No public push, PR update or public merge is authorized before independent review.

## Formal release gate continuity - 2026-11-06

Main's M23–M30 `PARTIAL` record was retained as development progress so that
integration would not erase newer implementation history. It does not replace
public PR42's formal M30 `BLOCKED` limitation. The formal release gate remains
`BLOCKED` until its outstanding guest/release acceptance and existing release
requirements are met. Host tests, prior fixtures and a correct ancestry graph
are not release acceptance. No code, CI/gate rule, receipt or
`m30_acceptance=NOT_EVALUATED` value changes in this documentation correction.
