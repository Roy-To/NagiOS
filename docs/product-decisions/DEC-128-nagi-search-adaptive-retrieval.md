# DEC-128 — Nagi Searchの適応型検索（B案）

- **状態:** CONFIRMED（Nagi独自のSearch Service／Provider、軽量検索を優先する適応型検索、権限と正本の境界）／PROVISIONAL（検索技術・具体API・性能・提供版）
- **最終合意日:** 2026-11-25
- **適用候補:** 0.2のSearch基盤／Home・Search、0.3のPersonal Agentによる検索技術の再利用。個別機能の提供版は未確定。
- **関連:** [DEC-005](DEC-005-files-albert-memory-lifecycle.md)、[DEC-012](DEC-012-workspace-naming-organization.md)、[DEC-013](DEC-013-lightweight-memory-engine-benchmark.md)、[DEC-061](DEC-061-first-party-pdf-reader-editor.md)、[DEC-090](DEC-090-common-persistence-contract-distributed-authority.md)、[DEC-123](DEC-123-kernel-unification-and-linux-0.1.md)、[0.2実装仕様 §25](../Nagi_OS_0.2_Codex_Implementation_Spec.md#25-search)・[M19](../Nagi_OS_0.2_Codex_Implementation_Spec.md#02-m19--search--index--workspace-platform)・[M26](../Nagi_OS_0.2_Codex_Implementation_Spec.md#02-m26--home--search-applications)、[0.3改訂ロードマップのSearch](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md#search)

## 目的

ユーザーがファイル・文書・Workspaceなどを探す機能と、Personal Agentが作業に必要な情報を取得する機能に、共通の検索基盤を提供する。日常の検索では端末の負荷を抑え、必要に応じて意味検索を利用できるようにする。

## 確定した仕様（CONFIRMED）

1. **Nagi独自のSearch ServiceとSearch Providerの仕組みを採用する。** OS共通のSearch Serviceが検索経路の選択、権限の適用、結果の統合と順位付けを担う。各アプリ・サービスのProviderは、許可された範囲で検索対象を提供する。
2. **基本検索は軽量な経路を優先する。** ファイル名や本文の完全一致・語句（lexical）検索、属性（metadata）、時刻・履歴（temporal／activity）を用いる。基本検索はオフラインで動作し、AIやEmbeddingを無効化しても利用できる。
3. **意味検索は必要なときに追加する。** 問い合わせと端末の資源・負荷に応じ、ローカルで動く交換可能な専用Embeddingモデルによる意味検索や、語句検索と組み合わせるHybrid検索を使える構成とする。意味検索を利用できないときも基本検索を提供する。
4. **OCR・Visionと連携する方向を採用する。** PDF・スキャンからOCRで抽出した文字や、Visionで得た画像内容を、元資料の権限に従って検索につなげる。
5. **索引は増分更新し、前景操作を優先する。** 元データの追加・変更・削除を差分で反映する。大量の索引作成やEmbeddingの生成は端末に余力があるときに進め、負荷が高いときは延期または一時停止できるようにする。
6. **検索前に現行権限を適用する。** User／Profile／Scope、Permission、元データのアクセス制御や機密区分を候補の検索前に評価し、許可された範囲で検索する。直接の結果に加え、候補表示・件数・抜粋などを通じたアクセス不能な資料の存在漏洩も防ぐ。
7. **元アプリの正本とCanonical Memoryを分離し、検索技術を再利用する。** 文書や履歴などの正本は元のアプリ・Providerまたは利用者の保存先が保持し、学習した記憶はユーザー所有・モデル非依存のCanonical Memoryが保持する。語句検索やEmbeddingの技術は双方で再利用できる。派生索引は正本から再構築可能とし、モデルの交換や索引の再構築でも正本を保持する。元資料の削除・無効化とMemoryの忘却は、既存の規則に従って検索側にも反映する。

## 未確定・暫定案（PROVISIONAL）

- 語句・全文検索の候補はSQLite FTS5とTantivy、日本語解析の候補はLinderaとN-gram等。比較評価を経て選定する。
- 個別のEmbeddingモデル、Vector backend、Hybridの統合・順位付け方式、OCR・Visionの接続方式と対応範囲。
- 検索精度、応答時間、資源消費などの目標・合格値、意味検索への切替や背景索引の閾値。
- Service／Providerの具体API・データ契約、実装・提供版、受入試験の詳細。

## 既存仕様・Roadmapとの関係

- 0.2実装仕様§25の検索層、M19のSearch／Index／Workspace基盤、M26のHome・Searchの製品面を具体化する製品方針。
- 0.3改訂ロードマップのMemory Retrievalと検索技術を共有する方向に沿う。DEC-005の元資料とMemoryの連動、DEC-012のWorkspace自然言語検索とユーザーによる復元先の選択、DEC-013とDEC-090の現行権限・Canonical Memory・派生索引の境界に従う。
- DEC-123によるLinux版0.1 Developer Previewと0.2有効化条件を適用する。既存のRelease／Activation／Acceptance GateおよびWorkstreamの担当に変更はない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-25 | 仕様検討3/12でユーザーがNagi SearchのB案を承認。軽量な基本検索と必要時のローカル意味検索、OCR・Vision連携の方向、増分・負荷適応型索引、検索前の現行権限適用、正本分離と派生索引の再構築を確定。技術・具体API・閾値・版はPROVISIONAL。 |
