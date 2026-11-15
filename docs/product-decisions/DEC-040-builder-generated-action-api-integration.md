# DEC-040 — `builder`製アプリのNagi連携とAction API提案・生成

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（方式C：目的に応じたAction候補の自動提案、必要なAPIの実装・検証と通常GUIの両立）／PROVISIONAL（生成・発見・検証の技術詳細と適用版）  
**最終合意日:** 2026-11-05  
**適用候補:** Nagi OS 0.4+の`builder`によるアプリ開発体験および将来のアプリ連携基盤。具体的なリリース割当ては未確定  
**関連:** [DEC-015（builder Role）](DEC-015-builder-role-review.md)、[DEC-016（開発Taskの承認範囲）](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-017（アプリ開発者・段階的なNagi統合）](DEC-017-third-party-developer-ecosystem.md)、[DEC-027（Public Action優先・限定GUI fallback）](DEC-027-public-action-and-scoped-gui-fallback.md)、[DEC-036（新規アプリの試用・正式導入）](DEC-036-builder-app-trial-and-installation.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)

## 目的とユーザー体験

Nagiに`builder`でアプリを作ってもらう際、通常のGUIで使えるだけでなく、適切な機能については「このフォルダのPDFを画像に変換して」のような自然言語依頼からNagiが正規Actionを呼び出して操作できる体験を目指す。すべてのアプリにAI連携を強制せず、通常利用のわかりやすさと安全性を両立する。

## 確定した仕様 — 選択C：必要なActionを提案し、GUIと両立

1. **アプリの用途に応じてAction候補を提案する。** `builder`は新規アプリの目的や主要操作を把握し、Nagi経由の自然言語操作・自動処理で利用価値のある機能を選んで、型付きの正規Public Action／Service APIの候補として自動提案する。提案時には機能、想定入力・出力、対象Resource、必要なPermissionや副作用を可能な範囲で示す。
2. **必要な場合に実装・検証する。** ユーザーが承認した開発Task、対象Workspace／Resource、許可Action／Permissionの範囲内で、必要なAction APIをアプリに実装・検証する。新しい作業ScopeやAction種別、権限が必要なら、既存の承認手順で確認する。承認済みScope内の通常の修正・Build／Testの自律実行や有限RetryはDEC-016／018に従う。
3. **通常のGUI操作も利用できる。** アプリの基本機能は従来のGUI等から利用可能とし、Nagi連携を利用できない／利用しないときも成立させる。AI連携が不要な単純なアプリではAction APIの追加を必須にしない。一般のアプリの統合レベル0〜3を再定義せず、0.2 Platform Foundation仕様の既存契約を尊重する。
4. **Nagiによる実行は正規Action経路に従う。** 利用者が自然言語で依頼した操作は、Nagiが型付きPublic Action／Service APIと正規IPC／Provider契約を優先して実行する（DEC-027）。Actionの存在、生成、一覧掲載、発見、テスト成功は、**実行承認、ユーザー／アプリのPermission付与、対象データ閲覧や外部送信の許可を意味しない**。実行時にも現行のOS Permission／Capability／Policy、Task Scope、Resource／Revision、必要な確認、Activity記録、Stop／Cancelを守る。
5. **GUI fallbackは既存の限定条件のみ。** 正規Actionに未対応の操作については、DEC-027が定めた明示許可・限定ScopeのGUI／Accessibility fallbackが成立する場合だけ利用する。正規Actionより広い権限を得るための回避経路にはしない。
6. **根拠と制限を説明する。** Nagiは実装したActionと未対応の機能、検証できた項目・失敗・未検証事項、要求権限と影響を報告する。Action生成の成功申告だけでアプリの安全性、正式導入、互換性、Nagiからの実行可否を保証しない。新規アプリの試用・採用・正式インストールはDEC-036の別手続きを維持する。

## 例外・禁止・安全境界

- `builder`が生成したAction APIから直接カーネル内部・非公開API・権限外のOS状態にアクセスできるとは扱わない。純正アプリと同じ公開契約・現行Policyを用いる。
- Actionの自動提案は、ユーザーの開発目的を変更したり、無断の監視・実行・外部送信・アプリ導入・Git commit／pushを許可したりしない。
- 既に確定しているNagiのPublic Action優先（DEC-027）と、アプリごとの段階的な統合と通常機能の独立（DEC-017）を維持する。すべてのGUI機能のAction化、どのアプリでも自然言語操作可能、互換性の全面保証を意味しない。
- 0.1 M30、0.2 M21 SDK／manifest／package／Action契約、0.3 Acceptance／Implementation Spec／Activation、0.4 Entry Gate／Integration Owner checkpoint、Workstream owner／AGENTS.md／既存実装境界を変更しない。Target Runtimeでの実装・Acceptance通過や実装着手許可を表さない。未公開仕様をPublicリポジトリへ転載しない。

## 未確定・暫定案

- Action候補の自動抽出・選定基準、ユーザーへの候補提示・承認UI、Action不要判定、API設計や変更時の判断境界。
- ActionのSchema／IDL／manifest登録、命名・Version管理、発見・宣言・Parameter検証・Binding・互換性の具体的な方式。既存0.2契約を正とする。
- Action API生成・テストの実装手法、テストケース、権限・失敗・Stop／Cancel・競合・副作用のTarget Runtime Acceptance、Performance／Budget。
- GUI操作とAction操作の機能差、API非対応アプリのfallback適格条件、段階的な利用可能版・Workstream・責任Owner。

## Roadmapとの関連

- **0.2:** 現行のSDK、アプリ統合レベル0〜3、Public Action、Permission、IPC、package／manifest契約と既存Gateを維持する。
- **0.3:** Personal Agentが正規Public Actionを利用する既存の方針を継承する。新機能の0.3実装を要求しない。
- **0.4+:** `builder`でアプリを作成する際の連携体験として、必要なAction APIの提案・実装・検証とGUIとの両立を製品方向に追加する。初期0.4の必須Acceptance・Workstreamを追加せず、版別導入は別途判断する。
- **1.0に向けて:** DEC-017の一般向けアプリ開発・配布環境とAIコーディング優先方針を補完する。全アプリへの強制統合やStore専用配布は求めない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | ユーザーが方式Cを確定。builderは用途に応じてNagi操作に適したAction APIを提案し、必要なものを承認済み開発範囲で実装・検証し、通常GUIを維持する。Actionの実装は権限付与や実行承認を意味せず、適用版と技術詳細は暫定。 |
