# DEC-036 — `builder`製アプリの個人利用・共有・公開

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（共有・公開方式C：個人利用を標準とし、明示希望・内容確認・承認後に範囲別に出力）／PROVISIONAL（形式・検査・配布経路・適用版）  
**最終合意日:** 2026-11-05  
**適用候補:** `builder`が作成するアプリの将来の共有・配布体験（具体的なRelease割当ては未確定）  
**関連:** [DEC-002（外部データ共有）](DEC-002-cloud-ai-data-sharing.md)、[DEC-003（Memory機密分類）](DEC-003-memory-classification.md)、[DEC-015（builder Role）](DEC-015-builder-role-review.md)、[DEC-017（第三者アプリ開発・配布）](DEC-017-third-party-developer-ecosystem.md)、[DEC-030（Workflow移植・共有）](DEC-030-workflow-template-portability-and-sharing.md)、[DEC-035（builder製アプリ保守・更新）](DEC-035-builder-created-app-maintenance-updates.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)、[0.5–1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md), [DEC-041（アンインストール・関連データ削除）](DEC-041-builder-app-uninstall-and-data-deletion.md)

## 目的

ユーザーが望む場合に限り、`builder`製アプリを個人利用の境界を越えて共有・公開できる道筋を設ける。アプリの作成、ソース編集、試用、正式導入、保守更新、Workflowテンプレート共有、第三者への送信・一般公開は異なる操作であり、各操作の承認を混同しない。

## 確定した仕様 — 選択C（2026-11-05）

1. **個人利用を初期標準とする。** `builder`が作成したアプリは、ユーザーが共有・公開を明示的に希望しない限り、個人利用の成果物として扱う。アプリ作成・試用・正式導入の承認だけで共有可能とはみなさない。
2. **希望時に出力前レビューを行う。** ユーザーが共有・公開を望んだ場合、Nagiは出力候補のソースコードまたは配布Packageについて、コードと含有ファイル、依存関係、個人情報、秘密情報・API key等、個人設定・学習Memory、ライセンス、必要PermissionとCapability、配布対象・宛先を確認する。必要な情報の分離・削除・置換と検証を、許可された範囲内で行う。確認または必要な検証ができない項目・リスクは明示し、解消されるまで出力・送信・公開を保留する。
3. **内容と範囲をユーザーが選ぶ。** Nagiは含まれるソース／Package、依存関係、Permission、既知の未解決事項、および選択した配布範囲を提示する。ユーザーは共有する内容と範囲を確認し、出力を明示承認する。ローカルへのソース／Package出力、特定の友人等への限定共有、GitHub／Store等への一般公開を区別する。限定共有と一般公開は別々に明示選択し、片方の承認をもう片方へ流用しない。
4. **共有作成の許可と実際の外部送信／公開を分ける。** 共有用成果物の準備・作成・ローカル出力が許可されても、それだけで外部サービス、アカウント、Repository、Store、または受取人への送信・アップロード・公開を認可しない。実際の外部送信・公開の直前に、対象成果物、宛先／サービス、公開範囲、内容を示し、そのActionに対する独立した明示承認を得る。必要なPermission／Policy／Capabilityも現行状態で評価する。
5. **受取側で独立に検証・認可する。** 受取側は受領したソース／Packageと依存関係を自身の環境で未信頼の成果物として検証し、必要なPermission／Capabilityを自身のPolicyに基づき確認・再承認する。送信側での検証や承認を受取側へ引き継がない。
6. **個人状態・権限・認証情報を移転しない。** 元ユーザーのProfile／個人Memory／学習Memory、個人設定、Permission／Capability／Approval、Session／Task権限、Credential／Token/API keyを受取側へ含めず、移転・継承させない。アプリがユーザー由来の設定・Memory・情報を必要とする場合は、共有用成果物から分離し、受取側が自ら設定・認可する。
7. **過剰な保証をしない。** 目視・自動検査を問わず、秘密情報の完全除去、個人情報の完全匿名化、ライセンスの自動判定、全依存関係の安全性、完全な互換性を保証しない。検査範囲・検査不能項目・残存リスクを正直に示す。公開配布のStore implementationや運営を本決定で確定・保証しない。

## 既存決定との境界

- **DEC-002:** 外部送信前の情報分類、現行Permission／Policy、宛先、目的確認を維持する。ソース／Packageの外部送信・公開はCloud AI Context送信と別のPurpose・Actionであり、各許可を相互流用しない。
- **DEC-015／016:** `builder` Roleと開発Taskの承認Scopeはアプリ作成・検証に関するもので、第三者共有、外部送信、一般公開の権限を生まない。
- **DEC-017:** 公開App SDK、package検証、Side-loading、開発者環境、Store候補に関する既存契約を尊重する。本DECは公開範囲を選べる将来の製品挙動を記録し、0.2-M21等の既存仕様を再定義しない。
- **DEC-030:** Workflowテンプレートの移植・共有は別成果物・別承認である。アプリのソース／PackageをWorkflowテンプレートとして扱わず、Workflow共有承認をアプリ配布へ流用しない。
- **DEC-035:** 正式導入後の保守・更新と、本DECの共有・公開は別操作である。正式更新の承認は配布を許可しない。
- **DEC-041:** アプリ共有・公開の承認はアンインストールや関連データの削除を許可しない。保存ソースと開発Projectを削除する場合は、対象別の説明・選択・承認を要する。

## 未確定事項（PROVISIONAL）

- ソースと配布Packageの具体的な形式、再現可能なBuild、provenance、署名、manifest、Version、SBOM等の要件。
- コード・依存関係・秘密情報・個人情報・Memory・ライセンス・Permissionをどのように検査し、分離・修正・記録するか。
- ローカルExport、直接転送、個人間共有、GitHub／Store等の経路、宛先確認・公開設定・取消・更新・削除の具体的UIとPolicy。
- 受取側の検証要件、対応Package形式、Permission再承認UI、依存解決、互換性Acceptance。
- 具体的なRelease割当て、実装Owner、受入条件、Store実装の有無。

## Roadmap・Gateとの関係

本決定を記録しても機能実装、Workstream開始、Release activationを許可しない。0.2-M21、0.3 Acceptance、0.4 Entry Gate／Integration Owner checkpoint、既存Workstream owner・依存関係を変更しない。0.4+および0.5–1.0上の配置は将来方向との整合を示すに留まり、Release割当てと完了要件は別途明示決定を要する。Private仕様をPublic repositoryへ転載しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | ユーザーが共有・公開方式Cを確定。個人利用を標準とし、ユーザーの希望時に内容確認・必要な分離と検証・明示承認後に出力し、限定共有と一般公開を区別。実際の送信／公開の別承認、受取側の独立検証とPermission再承認、元ユーザーのMemory／権限／Credential非移転を記録した。 |
