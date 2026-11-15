# Nagi OS共同開発者向けオンボーディングガイド（AI活用）

> このガイドは既存の決定と作業規則を案内するものです。新しい製品決定、workstream割当、権限、release gateは設定しません。

## 1. Nagiの目的と現在の方針

Nagiは、AIを安全に活用しながら利用者が主導権を保つ独立OSを目指しています。現在の確定方針は **Linuxカーネル + Nagi専用ユーザーランド** で、既存Linuxディストリビューションを製品OSの土台にはしません。AIはユーザー空間の非特権・未信頼コンポーネントです。AIの出力だけで権限は得られず、操作は決定的なPolicy、Permission、Capabilityの境界を通します（[DEC-042](../product-decisions/DEC-042-linux-kernel-transition.md)、[DEC-043](../product-decisions/DEC-043-nagi-userland-independent-os.md)）。

製品方針がCONFIRMEDであることは、Linux移行や機能の実装・受入が完了したことを意味しません。具体構成にはPROVISIONALな項目があり、既存のrelease gateと受入条件は維持されます。古い0.1資料には移行前の方針が残ることがあるため、現行の製品方針はDEC-042/043、実装の事実と残件は[実装状況](../implementation_status.md)と担当workstreamの記録で確認してください。[Roadmap索引](../product-decisions/roadmaps/README.md)にある将来案も、実装許可やrelease確定ではありません。

## 2. 役割と裁量

| 役割 | 責任 |
|---|---|
| Founder（創設者） | Nagi全体の方向性と最終判断を担う。 |
| Core Developer（共同開発者） | 割り当てられた領域の設計・実装を主体的に進める。担当内の日常判断に毎回の事前承認は要しない。 |

この裁量は、確定仕様、セキュリティ境界、workstreamの所有範囲、統合条件、レビュー、受入条件、release gateの内側で使います。他の担当に影響する変更や製品全体の決定はFounderと関係するownerに相談してから統合します。[DEC-046](../product-decisions/DEC-046-co-developer-governance.md)は、個別の担当・実装開始・GitHub権限・開発環境を自動で割り当てるものではありません。 0.3のTeam Profileはデータ共有とPermissionの境界、0.4+ Multi-Agent Organizationは将来の製品機能の構想であり、人間の共同開発体制とは別です。DEC-046はこれらの実装やactivationを許可しません。

## 3. GitHubとAIアカウント

各開発者は、自分自身のGitHub、ChatGPT/Codex/Claudeのアカウントと契約で作業します。アカウント、パスワード、APIキー、access token、session linkを共有しないでください。認証情報や個人情報をソース、issue、PR、ログ、AIへの入力に含めないでください。

AIは実装支援者です。提案や生成差分は人間が確認し、許可されたbranchと作業範囲だけへ反映します。他の人の権限や資格情報を借りてAIに操作させないでください。

## 4. 初期セットアップと正規資料

作業対象のlaneを決めてから、そこに指定された開発環境・toolchain・検証方法を使ってください。手順がlaneごとに異なるため、独自のinstall/build commandを作らず、次の正規資料から確認します。

- [CONTRIBUTING.md](../../CONTRIBUTING.md)：変更前の確認、Build/verification、license、公開時の注意。
- [0.1 Developer Preview開発ガイド](../developer-preview/README.md)：ホスト準備、toolchain、既存のCLI・QEMU・診断手順。
- [Development資料](README.md)：0.2の開発状態・workstream参照手順。
- toolchainと依存の定義：[rust-toolchain.toml](../../rust-toolchain.toml)、[nagi.toml](../../nagi.toml)。
- repoの規則：[AGENTS.md](../../AGENTS.md)、[実装仕様](../Nagi_OS_0.1_Codex_Implementation_Spec.md)、[現在の実装状況](../implementation_status.md)。
- AI向け引き継ぎ：[CODEX_START_HERE.md](../product-decisions/CODEX_START_HERE.md)。

Linux移行laneの作業場所と操作許可は[DEC-044](../product-decisions/DEC-044-linux-migration-development-authorization.md)、[DEC-047](../product-decisions/DEC-047-linux-migration-workspaces-and-integration.md)、[移行開発ADR](../decisions/2026-11-12-linux-migration-development-scope.md)の範囲に従います。DEC-047のLinux移行向け許可を別laneに一般化しないでください。

## 5. Task、branch、PR、CI、受入

コード変更の前に[workstream registry](../../.dev/workstreams.json)と、その行が指す担当State・設計資料を読み、次を確認します。

- 担当ownerと現在の状態、依存関係
- `owner_branch`、`allowed_paths`、`forbidden_paths`
- `activation_gate`、`merge_boundary`、対象の受入条件

既存担当のpath、branch、worktree、生成物を横取り・上書きしません。作業範囲や所有者が不明なら、編集を始める前にFounderまたは記載されたownerと範囲を決めます。割当branchから作業し、PRのbaseと統合先はregistry／laneのmerge boundaryに合わせます。共有履歴を書き換えたり、gateを迂回してmergeしたりしません。

PRには目的、対象pathとworkstream、関連するDEC、実行した確認と結果、CI、未検証事項・blockerを記載します。 統合はreview、CI、受入条件を確認し、merge_boundaryが認める担当者・手順で行います。CI成功はそのcommitのcheck結果であり、release acceptanceとは別です。host-only test、Linux test、Nagi image内のtarget acceptanceを混同せず、条件を満たしていないmilestoneやreleaseをPASSと報告しません。

## 6. 仕様を変えるとき

担当領域内の技術判断はDECとgateに適合する範囲で自主的に進められます。確定済みの製品仕様、共通API、権限・security model、他laneの契約、またはrelease gateを変える提案は、実装前にFounderと関係者へ相談します。

会話で出た案をCONFIRMEDにしません。未合意案はPROVISIONALまたはREVIEW NOTEとして扱います。Founderの明確な承認が得られた製品決定は[OPERATING_RULES.md](../product-decisions/OPERATING_RULES.md)に従い、該当DECを更新し、[決定索引](../product-decisions/README.md)と変更履歴も更新してcommitを確認します。

## 7. 最初の1タスク

まずFounderに小さく独立した担当を依頼し、既存workstreamと競合しない範囲を一緒に確定します。着手前にtask briefへ次を記録してください。

1. 目的と成果物、担当owner / workstream
2. 使用branchまたは既存worktree、変更可能・禁止path
3. 依存関係と適用されるactivation gate
4. 受入条件、正規資料に記載された検証手順
5. PRのbase、review担当、merge boundary

その後、repoの現在状態と差分を確認し、関連仕様・workstream Stateを読んでから実装します。自分のAIには担当範囲と参照資料を明示し、提案差分を自分でレビューします。該当laneで必要な確認を実行し、証拠と残件をPRにまとめ、ownerのreviewとCI・受入条件の確認を受けてください。割当やgateが未確認のまま、具体的な製品タスクを自己割当しないでください。

## 8. トラブル時の引継ぎと問い合わせ

path競合、権限不足、想定外の差分、CI失敗、gate不明、認証情報の混入が起きたら、影響する作業を止め、他laneや別経路で迂回しません。branch・差分・ログを保全し、既存ownerに相談してください。

引継ぎにはworkstream、branchとcommit SHA、変更path、実行した確認と終了結果、CI、確認済み／未確認の事実、再現できるエラー、必要な判断を含めます。通常の設計質問は該当taskのPR discussionまたはissueでownerに伝え、秘密情報や脆弱性の報告は公開せず[SECURITY.md](../../SECURITY.md)の案内に従ってください。

## 参照資料

- [製品決定の索引](../product-decisions/README.md)と[運用規則](../product-decisions/OPERATING_RULES.md)
- [共同開発者の役割](../product-decisions/DEC-046-co-developer-governance.md)と[仕様正本の決定](../product-decisions/DEC-050-public-specification-repository.md)
- [貢献ガイド](../../CONTRIBUTING.md)と[開発者向け資料](README.md)
