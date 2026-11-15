# ADR — Private Linux migration development scope (2026-11-12)

> **2026-11-15 公開移管:** [DEC-050](../product-decisions/DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

Status: Accepted for the bounded development scope authorized by [DEC-044](../product-decisions/DEC-044-linux-migration-development-authorization.md). Concrete component selections and production cutover remain provisional.

> **2026-11-14 後続承認（DEC-047）:** このADRの初回「NOT ASSIGNED」および新規worktree/Git/build/VMを解放しない記述は、本人の後続承認により、DEC-047記載のlane別SSD worktreeと許可操作の範囲で優先順位を更新する。決定範囲と明示された継続事項は[DEC-047](../product-decisions/DEC-047-linux-migration-workspaces-and-integration.md)を参照。
## Context and limited precedence

DEC-042 and DEC-043 establish Linux kernel plus Nagi-owned userland and reject an existing distribution as the product OS base. The user's subsequent 2026-11-12 authorization permits private migration design, implementation and testing. This is additional authorization, not an inference from the earlier product direction.

For explicitly assigned private Linux migration work, old implementation assumptions requiring an independent Nagi kernel, prohibiting any Linux runtime, or requiring completion of old-kernel-only milestones before migration development do not apply. The historical 0.1 baseline, original ADR text, tests and release records remain intact. This scoped rule does not declare any existing milestone PASS or activate unrelated future features.

## First boundary and evidence contract

Use one development target. The first integration is Linux boot to Nagi userland to a Nagi service; subsequently connect an existing application through ordinary Nagi UI to authorized persistent save and reread after restart. Portable core and Linux adapters may advance in parallel against explicit input/output, error, authorization, cancellation and ownership contracts. An unfinished full-system design is not a global start gate.

AI remains unprivileged. Typed Public Actions, deterministic policy and authorization, revocation/cancellation, data ownership and protection are mandatory. Linux UID/root/capabilities are not by themselves proof of equivalent Nagi authority. Define and test the enforcement boundary; never route around the broker or fake guest functionality.

Distinguish portable logic tests, Linux tests of the same production code/adapter, and tests inside the Nagi Linux image. Record exact revision/diff, inputs, environment, limits and outcomes. Existing kernel or host-only PASS is not Linux target acceptance.

## Assignment and current execution status

The authorization and documentation live in existing Private main. Dot coordinates documentation, interface ownership and conflicting changes. Implementation branch/path owners and verification environments are NOT ASSIGNED in this initial record. A candidate workspace is not an accepted execution environment. Each actual assignment must state existing authorized source location, permitted files, responsible worker, integration destination and verification conditions.

No new checkout/worktree/clone, environment, installation, paid resource, VM/host action, Git operation or security-setting change is authorized merely by this ADR. Preserve applicable individual permissions and resource limits. In particular, no Git/build/VM operation is released by a documentation update where an applicable SSD protection or access restriction remains unresolved.

## Technical choices and preserved decisions

Within approved execution scope, responsible workers may choose reversible development kernel/toolchain/library/boot/adapter/test configurations after checking fit, maintenance, provenance, pinning and licensing; record reasons and changeable boundaries. Selective OSS reuse is allowed, but no distribution rootfs/service/package ecosystem is adopted as the product foundation.

Preserve legacy code, branches, unmerged changes, evidence and known issues. Avoid new old-kernel-only functionality, large fixes or dependency expansion solely to finish the retired foundation. Existing portable work continues under its current permissions.

Production cutover release/date, distribution/publication, user-data migration, compatibility guarantees and existing release Gate changes require separate decisions. They are not blanket prerequisites for this authorized development. Private material must not be copied into Public repositories, issues or CI artifacts.

## Consequences and unresolved scope

This removes only the contradictory old kernel prerequisite inside the assigned migration development scope. It does not select a kernel revision, libc, init implementation, display backend, package format or permanent security mechanism. Record those bounded development decisions as evidence becomes available. Shared build/dependency/runner infrastructure should be reused rather than independently rebuilt by every worker.
