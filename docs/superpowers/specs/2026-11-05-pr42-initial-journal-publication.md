# PR42 initial Trash journal publication

## Problem and scope

At parent 809690ca, Journal::save first creates an empty file in a durable create transaction and then writes its initial journal. A reported write failure after create can leave an empty slot that subsequent save rejects. Empty unknown files must not be automatically overwritten or deleted: they are indistinguishable from unrelated existing data.

## Design

Add a single-block initialized-create API that uses the existing UndoAction::Create record. Validate the payload bound before mutation. Zero-pad and write its newly allocated block before publishing the inode and parent entry, set size and block accounting consistently, and retain current transaction flush/rollback/explicit-Recovery semantics. No persistent format change or repair of foreign slots is introduced.

Use this API only for a missing journal slot. Existing journals retain validation and write behavior. This deliberately does not claim to solve ordinary write failure while overwriting an already valid journal, nor existing poisoned-slot repair or arbitrary device write ordering.

## Regression plan and execution boundary

A targeted NAGITR01 payload-write fault distinguishes the original empty-file bug from generic allocation faults. Require the injection to be reached, recover/remount, verify source identity and bytes plus Search, retry Trash/restore, and check integrity. Keep foreign empty-slot refusal coverage.

The VFS regression enumerates write/flush positions for initialized create in both modeled cache modes and verifies absent-after-Recovery, payload retry and unchanged neighbor. Verify no writes for oversized inputs and no overwrite on existing paths. These test sources are prepared but not executed. No alternate host/environment is used to rerun a denied test. Pinned rustfmt parsing and whitespace checks are permitted source checks, not typechecking or test PASS.
