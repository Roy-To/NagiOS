# DEC-030 — Workflowテンプレートの移植・共有と権限の再承認

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（個人状態・権限から分離したWorkflowテンプレート、明示的な移植／共有操作、受取側での再設定・再承認）／PROVISIONAL（テンプレート形式・検査／署名・依存解決・配布経路・適用版）  
**最終合意日:** 2026-11-05  
**対象:** NagiのWorkflowをユーザー環境間でエクスポート・インポートし、明示操作により移植・共有する将来機能。実装時期・Release割当ては未確定  
**関連:** [DEC-003](DEC-003-memory-classification.md)、[DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-023](DEC-023-work-data-centric-os.md)、[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)、[DEC-028](DEC-028-deterministic-workflows-from-habits.md)、[DEC-029](DEC-029-scheduled-event-triggered-workflows.md)、[Private 0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[DEC-036（builder製アプリ共有・公開）](DEC-036-builder-app-sharing-and-publication.md)、[Public 0.2 Platform Foundation仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)

## 目的

ユーザーが作成・承認したWorkflowを、個人のProfileや実行権限と切り離した再利用可能なテンプレートとして、必要な場合に自分の別環境へ移植したり他者と共有したりできるようにする。テンプレートの受け取りは、個人データや実行権限を引き継ぐことを意味しない。

## 確定した仕様（思想⑭・選択A）

1. **Workflowを個人状態・権限から分離:** 共有・移植の対象はWorkflowテンプレートであり、個人Memory、Preferences、Habits、Profileの機密情報、個人ファイルの実体、認証情報、継続Taskの権限・Capability・Token・Approvalを含めず、それらを移転しない。
2. **明示操作によるExport／Import:** ユーザーは対象と内容を確認して、Workflowテンプレートを明示的にExport、Import、または共有する。保存・作成・利用の承認は共有操作を許可しない。Nagiはユーザーの明示操作と確認なしに第三者やTeamへ提供しない。
3. **テンプレートの内容境界:** テンプレートは、再利用する手順、入力パラメータ、必要な公開Actionおよび依存App／Resource参照等の非機密スキーマを含む構想とする。具体的な形式・項目・制約は未確定である。元データ由来の機密・個人情報を不用意に取り込まない設計を目指すが、自動匿名化や完全な機密除去を保証しない。Export前に内容を確認できることを目指す。
4. **受取側での再設定:** Import先のユーザーは、App／Provider、Resourceその他の参照を自分の環境に再マッピングまたは再設定する。送信元環境のResource実体や認証状態を受取側で利用可能と仮定しない。
5. **権限の独立した再承認:** 受取側では、現在のPermission／Policy／CapabilityとTask Scopeを評価し、必要なTask、Action、無人のScheduled／Event-triggered実行について、それぞれ該当する承認を独立に取得するまで実行できない。送信元のApproval、継続Task権限、Token、またはモデルに学習された権限を移転・復元しない。
6. **未信頼テンプレート:** Importされたテンプレートは実行可能な信頼済みコードや権限付与として扱わない。内容・参照・Actionを検証し、受取側の現行認可境界に通す。検証または依存解決ができない場合は実行を保留する。
7. **関連決定との一貫性:** Workflowの内容・改訂承認はDEC-028に、Scheduled／Event実行の明示許可と実行時再検証はDEC-029に従う。習慣・共有・ImportだけでWorkflowを保存、有効化、実行したり権限を拡張したりしない。
8. **アプリPackage共有との区別:** Workflowテンプレートの移植・共有は`builder`製アプリのソース／Package配布とは別の成果物・目的・検査・公開範囲である。アプリ共有の承認をWorkflow共有へ流用せず、Workflow共有の承認をアプリ共有・外部送信・公開へ流用しない。アプリ配布はDEC-036に従う。

## 例外・禁止・安全境界

- 受取側での設定・権限再承認なしに、Import直後にWorkflowを実行しない。
- Team共有は、共有対象と相手を指定する明示操作を要求し、個人MemoryやProfileを自動的にTeamへ昇格させない（DEC-003、DEC-004）。
- Workflowテンプレートにパスワード、API key、Token等の認証情報や個人ファイルの実体を埋め込まない。
- テンプレートの移植性を理由に、Resource／Providerの権限境界、Public Action契約、Permission／Policy、Action別承認を迂回しない。
- 本決定は0.1／0.2／0.3／0.4+の既存Gate、Acceptance、Workstream ownership、Public／Privateの公開境界を変更しない。記録は実装開始・有効化・Release activationを許可しない。未公開仕様をPublic repositoryへ転載しない。

## 未確定事項（PROVISIONAL）

- WorkflowテンプレートのPackage／Schema、Versioning、互換性、署名・完全性・発行元検証と信頼表示。
- Export前の内容確認、機密情報の検出・警告・除外、匿名化可能性とその限界。
- Import時の依存App／Public Actionの解決、Resource参照の再マッピング、利用不能・非互換な依存の処理。
- ローカルファイル、直接転送、Private／Public repository、Store等の保存・共有経路と各経路の公開範囲。
- 個人間・Team共有の承認UI、アクセス制御、共有の取消・更新・削除・由来表示。
- 実装方式、Release割当て、対応アプリ、受入条件、対象となるWorkflow種別。

## 既存仕様との整合

- **DEC-003／004:** 個人Memory・Preferences・Habitsの機密分類、由来、所有範囲を維持する。Workflowテンプレートを個人MemoryのExport手段として扱わない。
- **DEC-017／023／025／027:** Public SDK／Action、アプリ間Resource参照、Permission、および限定GUI fallbackの既存境界に従う。実データの正本やResource権限はテンプレートにより移転しない。
- **DEC-028／029:** Workflowの版・正式改訂承認と、自動実行条件・権限許可をそれぞれ独立に保つ。
- **Public 0.2仕様:** SDK、Permission／Capability、Package、Public Action等の既存契約を基礎とする。0.2のScope、Gate、Acceptanceを変更せず、本DECの未公開詳細をPublic仕様に追記しない。
- **Private 0.3改訂案:** Profile Portability、Multi-User、Team Profile Shared Procedures、Privacy／Permissionの方向と整合する。WorkflowテンプレートをProfile全体の移行と区別し、版割当てやRoadmapの実装順序は変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 思想⑭の選択Aを確定。Workflowを個人Memoryや実行権限から分離した移植・共有可能なテンプレートとして扱い、ユーザーの明示操作・確認を必須とする。受取側でデータ参照を再設定し、Permission／Policy／CapabilityおよびTask／Action／自動実行の承認を独立に取得する。具体形式・検証・依存解決・配布経路は暫定とした。|
