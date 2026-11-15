# 参照元・確認した版・今回の限界

## Nagi仕様の基準

2026-11-15に公開GitHubを実際に確認しました。参照基準commitは `b80a74b93e25176e38408afde90d122a747103b5`。PR #101の公開移管と、PR #102の共同開発者割当案はこの基準に含まれます。利用時は最新mainと実際の担当branchを読み直してください。

- [製品決定索引](https://github.com/Roy-To/NagiOS/blob/b80a74b93e25176e38408afde90d122a747103b5/docs/product-decisions/README.md)
- [運用規則](https://github.com/Roy-To/NagiOS/blob/b80a74b93e25176e38408afde90d122a747103b5/docs/product-decisions/OPERATING_RULES.md)
- [AGENTS.md](https://github.com/Roy-To/NagiOS/blob/b80a74b93e25176e38408afde90d122a747103b5/AGENTS.md)
- [DEC-045: Porting Toolkit](https://github.com/Roy-To/NagiOS/blob/b80a74b93e25176e38408afde90d122a747103b5/docs/product-decisions/DEC-045-nagi-porting-toolkit.md)
- [DEC-046: 共同開発者の役割](https://github.com/Roy-To/NagiOS/blob/b80a74b93e25176e38408afde90d122a747103b5/docs/product-decisions/DEC-046-co-developer-governance.md)
- [DEC-049: 速度優先](https://github.com/Roy-To/NagiOS/blob/b80a74b93e25176e38408afde90d122a747103b5/docs/product-decisions/DEC-049-development-process-speed-priority.md)
- [割当案・NDP-01〜12](https://github.com/Roy-To/NagiOS/blob/b80a74b93e25176e38408afde90d122a747103b5/docs/development/CO_DEVELOPER_ALLOCATION_PROPOSAL.md)
- [既存オンボーディング](https://github.com/Roy-To/NagiOS/blob/b80a74b93e25176e38408afde90d122a747103b5/docs/development/co-developer-onboarding-ja.md)
- [PT-01担当記録](https://github.com/Roy-To/NagiOS/blob/b80a74b93e25176e38408afde90d122a747103b5/docs/workstreams/PT-01-readonly-host-assignment-2026-11-13.md)
- [PT-02 SDK監査](https://github.com/Roy-To/NagiOS/blob/b80a74b93e25176e38408afde90d122a747103b5/docs/porting-toolkit/PT02_SDK_SURFACE_AUDIT_2026-11-13.md)
- [0.5–1.0 roadmap](https://github.com/Roy-To/NagiOS/blob/b80a74b93e25176e38408afde90d122a747103b5/docs/product-decisions/roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

最新のworkstream registry、担当State、PR、SDKのコード、実装statusはNDP-01で再確認します。本キット作成時に未pushのローカル作業や友人のPCは調査していません。競合がないと保証しません。

## 資料と提案の境界

役割・Porting Toolkitの製品境界は上記仕様に基づきます。新規path、branch名、モデルへの割当例、細かなテストケース、承認文、記録様式は本キットの実行案です。公開済み仕様へ新しいCONFIRMED事項を追加していません。taskの番号・目的は割当案に揃えています。

SDK監査が示す `publish = false` や不足APIは監査時点の情報です。最新版でも同じと断定せず、NDP-02で現物確認します。PT-01とPR #91の正規接続も未決事項を勝手に解決しません。

## Claude / Codexの操作情報

Nagi仕様とは別に、2026-11-15に次の公式資料を確認しました。

- https://developers.openai.com/codex/quickstart/
- https://developers.openai.com/codex/guides/agents-md/
- https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan
- https://code.claude.com/docs/en/quickstart
- https://code.claude.com/docs/en/memory

画面・契約・利用枠は変わり得るため、価格、固定モデル名、UIの細かい位置を本キットで固定していません。

## この作業で行っていないこと

Nagiの実装、友人の環境setup、host/targetテスト、GitHub招待・権限変更、workstreamの開始登録、上流merge、releaseは行っていません。文書の保存・検査と、ソフトウェアの実装完了を区別してください。
