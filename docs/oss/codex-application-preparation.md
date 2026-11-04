# Codex for Open Source application preparation

Status: **preparation only; not submitted**.
Initial program/form review: 2026-10-11. Source-license wording updated: 2026-10-27.
This update is not a fresh verification of program eligibility or form fields.
Recheck [OpenAI's program overview](https://developers.openai.com/community/codex-for-oss)
and [application form](https://openai.com/form/codex-for-oss/) before submission.

日本語要約: 応募用の下書きです。Nagi 独自コードへの MPL-2.0 付与は
LICENSE_SCOPE.md の範囲に限定し、既存ライセンスやモデル等の条件を維持します。
字形・画像・依存関係等の未解決事項を隠さず、リポジトリ全体が権利確認済みとは
説明しません。対象変更の main への反映を確認してから応募文を使用してください。
応募の送信、資格充足、採択を示す資料ではありません。

## Project summary and evidence

Nagi OS is an experimental, independent, local-first operating system with its
own kernel, an Albert/Servo browser port and permission-constrained AI design.
The QEMU x86-64 Developer Preview remains under development. Consult the exact
revision's [implementation status](../implementation_status.md); historical
acceptance results do not prove a finished agentic desktop or production safety.

The maintainer's original OS source has a scoped MPL-2.0 grant in revisions
containing [LICENSE](../../LICENSE) and [LICENSE_SCOPE.md](../../LICENSE_SCOPE.md).
Existing dual-license package declarations, third-party code and model terms
remain separate. The [scope review](mpl-2.0-scope-review.md) explicitly records
its limits and unresolved glyph/media/dependency rights checks. Do not describe
the entire repository or an OS image as fully license-cleared.

| Claim | Evidence to check |
| --- | --- |
| Project scope and limitations | [English README](../../README.md), [Japanese README](../../README.ja.md), [architecture](../architecture/README.md), [specification](../Nagi_OS_0.1_Codex_Implementation_Spec.md) |
| Active maintenance | [PRs](https://github.com/Roy-To/NagiOS/pulls), [commits](https://github.com/Roy-To/NagiOS/commits/main/), [CONTRIBUTING](../../CONTRIBUTING.md); identify actual maintainer work rather than assuming authorship of every commit. |
| Guest evidence | [Status](../implementation_status.md), historical [M18 CI](https://github.com/Roy-To/NagiOS/actions/runs/37601664873), README screenshots with their capture dates and limits. |
| Source licensing | [Scope](../../LICENSE_SCOPE.md), [review](mpl-2.0-scope-review.md), [third-party inventory](../../THIRD_PARTY_NOTICES.md). |
| Video | [Recording plan](demo-recording-plan.md) only; no video is represented as published by this preparation. |

No stars, downloads, user counts, downstream dependents or adoption figures are
invented. Obtain current, dated evidence for any such claim.

## Form drafts

The prior form review recorded 500-character narrative fields. The drafts below
are respectively 370, 312 and 361 Unicode characters (excluding Markdown quote
markers). Confirm the live form limits before use, and use licensing claims
only after verifying the relevant grant in the public default branch.

**Why the repository matters:**

> Nagi OS is an experimental, local-first operating system with its own kernel, a Servo browser port and permission-constrained AI integration. Public QEMU acceptance records document progress and limits. The maintainer's original code has a scoped MPL-2.0 grant; existing package/model licenses remain separate. We do not claim broad adoption or clearance of every asset.

**Proposed use of API credits:**

> We would use credits for code review, CI-failure analysis, regression-test drafting, dependency-notice review and English/Japanese documentation. Human review and real QEMU acceptance remain release gates. The support would fund development workflows, not make Nagi's local-first guest depend on cloud inference.

**Anything else:**

> Nagi is an experimental Developer Preview, not a production-ready OS. Licensing scope and unresolved glyph/media/dependency reviews are documented publicly. Four packages preserve MIT OR Apache-2.0 declarations. Support eligibility, adoption metrics and account details require applicant verification. No award or completed full-release rights audit is claimed.

## Before submitting

- Verify the grant and scope on the actual public main revision, not only a draft
  PR. Do not equate a partial source grant with clearance of every dependency.
- Confirm the applicant's maintainer role, GitHub username and the correct
  personal service account. Supply identity/email/account IDs directly in the
  official form, not in public preparation documents.
- Check current program terms and eligibility independently. A license choice
  alone neither guarantees qualification nor an award. Anthropic's program
  requires its own separate checks; this file does not establish those.
- Review all claims, disclose known scope limitations, and submit only with the
  applicant's authorization to accept the relevant terms.

No application or external terms acceptance is performed by this repository
change. Selection and benefits are determined by the program operator.
