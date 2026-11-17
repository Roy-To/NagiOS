# DEC-095 — App Lifecycle：Linux基盤＋Nagi App Supervisorを標準に独自Runtimeを並行研究

**状態:** CONFIRMED（Foundation #26：Bを基本採用し、独自Runtimeの拡張余地を保持して並行研究する方針。Linux標準プロセス・隔離機構＋Nagi独自App Supervisorを製品の基本経路とし、研究成果の製品採用は比較検証後の別判断）／PROVISIONAL（独自Runtimeの形式・用途・実装・採否・定量性能・端末／版・受入）\
**最終合意日:** 2026-11-16\
**適用候補:** Nagi OS Foundation #26。実装開始条件、対象Version、Release割当ては未決定\
**関連:** [Foundation Checklist #26](NAGI_OS_FOUNDATION_CHECKLIST.md#f26)、[DEC-017（Nagi Native Apps）](DEC-017-third-party-developer-ecosystem.md)、[DEC-043（Linux＋Nagi専用ユーザーランド）](DEC-043-nagi-userland-independent-os.md)、[DEC-020（AIと通常操作）](DEC-020-ai-os-interaction-path.md)、[DEC-075（builder製アプリ試用・正式導入）](DEC-075-builder-app-trial-and-installation.md)、[DEC-035（導入済みアプリ更新）](DEC-035-builder-created-app-maintenance-updates.md)、[DEC-041（削除・データ保持）](DEC-041-builder-app-uninstall-and-data-deletion.md)、[DEC-094（Resource Manager／研究方針）](DEC-094-resource-manager-linux-baseline-custom-scheduling-research.md)、[0.2実装仕様 §13・M12／M13](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[Roadmaps](roadmaps/README.md)

## 用語と目的

**App Supervisor／Lifecycle Runtime**はアプリの起動・ready・stop・crash・再起動制限・依存関係・Session／権限・診断等を管理するOS側の仕組みを指す。**独自実行Runtime**は、Nagi Native Appのコード実行形式・Sandbox／実行環境・ホストAPI等をさらに拡張または独立実装する研究候補であり、前者をそのまま全面置換するものではない。

Nagi Native Appsの統一したアプリ体験、Workspace・公開Action／Contextとの連携、User／Profile別保護を維持しつつ、Nagi独自Runtimeで実際に付加価値が出るかを比較研究する。

## 確定した製品方針（CONFIRMED：B標準＋Cの研究）

1. **製品標準はB：Linux基盤＋Nagi独自App Supervisor。** Linuxの既存プロセス実行・メモリ管理・アプリ隔離の機構を利用し、NagiがManifest／Package／AppId、起動・ready・stop・crash・依存関係・異常時のbounded restart・ユーザー／Session所有・診断・更新連携を統合管理する。Linuxプロセス管理を全面独自実装するCを初期標準としない。
2. **Nagi Native Apps契約は共通。** 純正／第三者アプリはDEC-017のPackage、Manifest、SDK／Public API、Permission／Capability／Policyに従い、通常GUIの基本利用にLLMを必須としない。Linux／Android／iOSなどの既存アプリを無変更で実行する包括互換性を本判断から追加しない。
3. **独自Runtimeの拡張余地を維持し、並行研究する。** Nagi向けの小型／builder製アプリ等を候補に、必要に応じた実行形式・Sandbox／隔離・起動／復帰・メモリ効率・OS APIの統合方式を、Linux Native Process＋App Supervisorの基準経路と比較する。研究対象や具体方式は後続設計に委ね、Nagi独自の仮想マシンや言語の全面新規開発を確定しない。
4. **研究成果の採用は、実測に基づく別判断とする。** 起動・readyまでの時間、RAM／CPU、電力、動作安定性、Isolation／Permission拒否、Workspaceとの連携、開発者・移植負担、アプリ品質・保守性等を代表端末と現実的なシナリオで比較する。十分な効果と安全性が実証された用途のみ部分導入を検討し、製品Runtimeとしての正式採用は現時点では決めない。
5. **Linux Native Process／標準Supervisorを継続可能な経路として保持する。** 研究機構が利用不能・不適合・効果不足の場合でも、対応Nagi Native Appsが標準経路で動ける構成を重視する。実行形式の違いでユーザーPermissionやアプリデータ正本を暗黙に変更しない。
6. **既存の導入・更新・削除・Task境界を継承する。** Manifest検証と安全なruntime実行、builder製アプリの試用と正式導入、導入済みアプリの更新、通常アンインストールと完全削除、AI Task停止・再開を区別する。独自Runtimeの研究は、Packageの正式登録、Capability付与、ファイル削除、Actionの再実行の許可にならない。

## 既存の決定・実装契約との責務分担

- **0.2-M12**：Manifest／Package Lifecycle Contractの正本。AppId、Version、Entry Point、Permission／Capability、提供／利用Service等を定義・検証する。新Runtime研究でManifestやPackageの別の権限正本を作らない。
- **0.2-M13**：App Supervisor／Process Lifecycle RuntimeとBackground Job Runtime。起動・依存／ready・crash／bounded restart・Job lifecycle／cancel／idempotency等の受入契約を維持する。独自Runtime研究で現行の受入Gateや実装開始条件を一括変更しない。
- **DEC-017**：Nagi Native Apps専用、Windows／Linux等からの移植支援と開発者向けSDKの正本。実行形式研究が既存バイナリ無変更互換の採用やNagi専用言語の必須化を意味しない。
- **DEC-075／035／041**：builder製アプリの安全な試用・正式導入の別承認、導入済み更新の別承認、ユーザーデータを原則残す通常削除と確認付き完全削除を維持。研究機構を使う場合も変更しない。
- **DEC-020／Nagi Power／Resource Manager**：ユーザーの前景操作優先、Permission／Policy、非特権AI、資源制御、Jobの停止／資源待機・障害復旧の安全境界を維持。Runtimeは独自の無制限Schedulerや隠れたCapability経路にならない。

## 研究候補・未確定事項（PROVISIONAL／OPEN）

- Linux Native Processを比較Baselineとした上で、実行形式・軽量なSandbox／隔離・独自実行APIの各研究候補を選ぶ。**WebAssembly／WASI等は候補例**であって採用決定ではない。
- 独自Runtimeが適するアプリ分類（小型アプリ、builder生成アプリ、通常ネイティブアプリ、複数端末など）、CPUアーキテクチャ別の実行・署名・互換・デバッグ・起動の実際の可否。
- Cold／warm start、ready遅延、常駐RAM・ピークRAM、CPU・電力／熱、ストレージ、アプリ処理品質、Workspace状態保存と復元の成功率、crash／復帰／sandbox拒否の比較方法・数値目標。
- Runtime導入時に必要なOS Service／IPC／SDKの契約、AppId・Permission・Capability・Sessionの一致性、更新・データ移行・失敗時復旧・版管理、脆弱性修正と依存ライセンス／保守体制。
- 研究の具体Workstream・着手権限、実験実装、Linux target／実機検証、どの候補をいつ採用するか、Release／Activation／Acceptance Gate。

## Roadmap・実装境界

本DECは**Bの製品基本方針とCの並行研究を進める方向**をCONFIRMEDとする。研究のための具体的技術採用、実装開始、開発担当、特定releaseでの独自Runtime搭載、Linux target／実機での受入成功や体感高速化を確認したものではない。既存の0.2 M12／M13／M21、0.3、0.4 Entry Gate、Desktop first、Nagi Native Appsの通常操作・認可契約を変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #26で「Bを基本採用して、独自Runtimeの拡張余地を残し、#25と同様に研究を進める」方針を明示承認。Linux基盤＋Nagi App Supervisorを標準とし、独自Runtime／隔離／起動効率等を比較研究する。製品採用・具体Runtime・性能・端末・時期は未確定。 |
