# Codex for Open Source application preparation

Status: **preparation only; not submitted**. Initial review 2026-10-11; licensing policy updated 2026-10-26. Recheck the current application against
[OpenAI's program overview](https://developers.openai.com/community/codex-for-oss)
and [application form](https://openai.com/form/codex-for-oss/).
Requirements and form fields can change; recheck before submission.

日本語要約: 応募文の下書きと証拠をまとめた資料です。応募送信は行いません。
Nagi 独自コードへの MPL-2.0 採用方針は決定しましたが、権利確認が未完了であり、OSS としてのライセンス付与を完了した
プロジェクトとはまだ説明できません。利用規模・maintainer の役割・アカウント情報は
本人が確認します。採択や支援の保証はありません。

## Project summary

Nagi OS is an independent, local-first operating system targeting a QEMU
x86-64 Developer Preview. Its capability-based kernel keeps high-level policy
in user space. Albert ports Servo to the guest; deterministic checks bound
AI-proposed actions. The project explores owner control, native interaction,
and recoverable operations.

The baseline has M0–M18 recorded PASS; M19–M30 remain PARTIAL.
Individual local AI/provider experiments do not establish a completed agentic
desktop or production security. The repository is public, and MPL-2.0 is now the selected policy for eligible
first-party OS code, but it has **not been applied as an OSS license grant**.
The [MPL adoption plan](mpl-2.0-adoption-plan.md) records review gates; the
[older Apache-2.0 audit](apache-2.0-readiness.md) remains dependency evidence.

## Evidence to attach or reference

| Claim | Reviewable material |
| --- | --- |
| Project scope and architecture | [English README](../../README.md), [Japanese README](../../README.ja.md), [architecture](../architecture/README.md), [specification](../Nagi_OS_0.1_Codex_Implementation_Spec.md) |
| Active maintenance | [Repository PRs](https://github.com/Roy-To/NagiOS/pulls), [commits](https://github.com/RT-NISH/NagiOS/commits/main/), [contribution workflow](../../CONTRIBUTING.md); select concrete review/triage/porting examples and confirm the applicant's role. |
| Real guest behavior and honest limits | [Status record](../implementation_status.md), [M18 CI](https://github.com/RT-NISH/NagiOS/actions/runs/37601664873), existing QEMU screenshots embedded in README |
| License preparation | [Third-party notices](../../THIRD_PARTY_NOTICES.md), [MPL decision and application gates](mpl-2.0-adoption-plan.md), [Legal/SBOM tooling](../legal/README.md) |
| Demo | [Recording plan](demo-recording-plan.md) only. No video URL is available from this preparation. |

No stars, downloads, user counts, contributor counts, or adoption claims are
invented. Capture such metrics with their date/source if the applicant elects
to use them. Existing PR history is evidence of work; it does not by itself
prove ecosystem adoption or that the applicant authored every change.

## Form text drafts

The form currently has 500-character narrative fields. These drafts are under
that limit, but should be reviewed by the applicant and adapted after the
license decision. They do not claim the application qualifies automatically.

**Why the repository matters (draft):**

> Nagi OS explores a local-first, capability-based operating system with a native desktop, a Servo browser port, and AI proposals constrained by deterministic validation. Its QEMU Developer Preview has public guest acceptance evidence and documented limitations. We aim to share reusable porting and authority-boundary work. The project is early; we do not claim broad adoption, and MPL-2.0 is selected for eligible first-party code but not yet applied.

**Proposed use of API credits (draft):**

> We would use credits for maintainer workflows: reviewing Rust and C/C++ port changes, triaging reproducible guest failures, checking native dependency notices, improving English/Japanese documentation, and drafting tests for authority boundaries. Human review and real QEMU acceptance would remain release gates. Credits would support development rather than make the local-first guest require a cloud service.

**Anything else (draft):**

> M0–M18 are recorded PASS; M19–M30 remain PARTIAL. Local Granite guest inference has evidence, but normal model-service integration is incomplete. MPL-2.0 is the selected first-party OS licensing policy, pending authorship and scope review; third-party code, fonts and models retain their own terms. A dedicated documentation PR prepares licensing review and README material without changing implementation or milestone states.

## Maintainer checks before submission

- Confirm the applicant's primary/core maintainer role and repository write
  access; use their own public GitHub profile and correct ChatGPT account.
- Complete the MPL-2.0 ownership/scope audit and formal license application before
  presenting Nagi as an OSS-licensed project. Update the drafts with the
  actual decision; do not use a pending license proposal as proof.
- Confirm any usage/ecosystem claim with public evidence. Explain the
  significance of the OS/porting work without calling Nagi widely used.
- Choose the requested support based on actual workflow needs; review the
  current program terms. The official program considers active maintenance
  and ecosystem relevance; the maintainer's application is subject to review.
- Supply identity, email and OpenAI organization details directly in the
  official form. Do not commit personal contact details, organization IDs or
  credentials to preparation documents.
- Submit only when the applicant is ready to accept the program terms.
  This task authorizes preparation and a documentation PR, not submission.

Selection and benefits are determined by OpenAI. This document neither
guarantees eligibility nor records an award.
