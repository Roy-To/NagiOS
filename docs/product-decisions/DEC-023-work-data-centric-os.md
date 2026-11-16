# DEC-023 — 作業・データ中心のOS基本思想

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（基本思想・従来操作とPermission境界の維持）／PROVISIONAL（具体的な連携方式・自動化条件・適用版）  
**最終合意日:** 2026-11-05  
**対象:** Nagiの基本的なOS体験、アプリ間連携とデータ／Task／Workspaceの扱い。実装版・Activation時期は未確定  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-010](DEC-010-intent-first-adaptive-ui.md)、[DEC-012](DEC-012-workspace-naming-organization.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[Public 0.2仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)、[Public純正アプリ仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)

## 目的

従来のOSでは、利用者がアプリケーションの区切りを意識して文書・表・Web資料などを行き来することが多い。Nagiは利用者が達成したい**作業・目的・利用するデータ**を中心に捉え、アプリをその目的を進めるための道具として位置付ける。これはアプリを廃止・統合して一つの画面にするという意味ではない。

## 確定した基本思想（選択A）

1. **作業・データを中心にする:** Nagiの基本的な作業体験は、利用者の目的、Task、関連Resource／Document／Object、Workspaceの関係から整理する。特定アプリを開くこと自体を唯一の作業単位とせず、関連する情報・成果物・作業の連続性を重視する。
2. **アプリは目的を実現する道具:** Nagiは必要に応じて複数の対応アプリを選択・連携し、作業の目的に沿って利用できるようにする。これによって各アプリの独立性や提供する機能、ユーザーによるアプリの選択・操作を否定しない。
3. **従来の操作を完全に維持:** 利用者はNagiのAIやTaskを介さずに、個別アプリを起動し、マウス・キーボード・通常のGUIから操作できる。AIが利用できない場合や統合に未対応のアプリでも、そのアプリがサポートする通常の機能を利用できる設計を保つ（DEC-009、DEC-017）。
4. **同じ公開契約で連携:** 対応するアプリ間の連携は、既存のNagi PlatformによるResource／Document／Object／Revision、アプリが意図して公開するContext、型付きPublic Action／Service API、Workspace等の契約を基礎とする。純正アプリだけに秘密のAI操作経路を設けず、第三者アプリも対応範囲に応じて段階的に参加できる（DEC-017、DEC-020）。
5. **Permissionとデータ所有権を維持:** TaskやWorkspaceで情報を関連付けても、それ自体で別アプリのデータを読んだり変更したりする権限は発生しない。各Resource、Provider、User／Profile、Session、現在のPermission／Capability／Policy、承認済みTaskのScopeを、参照・取得・実行の各正規経路で確認する。Nagiは未公開のアプリ内部状態やMemoryを勝手に収集しない。
6. **自動整理と実行は別:** 作業の関連付け、アプリ候補の提示、Workspaceの表示・復元だけでは、未承認のデータ変更、外部送信、App間共有、終了した／中断したTaskの再実行を意味しない。実行にはDEC-001と正規Actionの承認・権限境界を適用する。
7. **状態・結果を正直に扱う:** 対応アプリで保持・復元・横断参照できる範囲と、未対応・権限拒否・部分成功を区別する。Workspace状態の復元はファイル内容の巻き戻しや任意アプリの完全復元と同一視しない（DEC-014）。

### ユーザー体験の例（実装済み・確約済み機能ではない）

「昨日の調査をもとにプレゼン資料を仕上げたい」という意図に対し、Nagiが現在アクセスを許されたAlbertの参照、Notesの記録、Slidesの成果物の関係を示し、対応する公開Actionや通常GUIを通して作業を進められることを目指す。必要な権限・アプリ対応がなければ、できない範囲を説明する。

## 維持すべき境界と禁止事項

- Nagiがすべてのアプリデータの正本を所有する、あらゆる形式を単一形式へ強制変換する、すべてのアプリをAI対応させる、といった方式は本決定から導かれない。
- Resource IDやWorkspaceへの所属だけでアクセス権限、Resourceの恒久的同一性、内容の最新性を保証しない。既存のRevision／Permission検証を維持する。
- サードパーティアプリの既存コード・保存形式を不必要に作り直すことを必須条件とせず、DEC-017の段階的Integration Level 0–3を維持する。
- 本決定はPrivateの製品方針であり、0.1 M30、0.2 M21等、0.3 Implementation Spec／Activation、担当Workstream、既存Acceptanceを変更せず、実装開始許可を意味しない。

## 未確定事項（PROVISIONAL）

- Task／Workspace／Resourceの関連付け・状態保持の具体的なデータモデル、アプリ選定・切り替え・優先順位のルール。
- Cross-app参照のスキーマ、共通メタデータの範囲、正本の配置、同期・競合・削除・移植時の取り扱い、未対応アプリへのfallback。
- UIの表示形式、Taskを提案・開始する条件、ユーザーの手動選択の反映方法、アプリ間編集や部分失敗の具体的UX。
- 具体的な実装Workstream・適用バージョン・測定指標とAcceptance。従来のPublic API契約を変更する場合は別途技術設計・承認が必要。

## ロードマップとの整合

- **0.1:** 現行MilestoneとAcceptanceを維持。新しいApp連携の有効化を要求しない。
- **0.2:** 既存のResource／Document／Object／Action、Workspace、Permission、SDK・第一者アプリ共通基盤の契約を利用し、勝手なRelease blockerを追加しない。
- **0.3:** 既存のPersonal Agentの許可されたContextと型付きPublic Actionによるアプリ横断操作方針に接続する。新たなAgent権限を付与しない。
- **0.4以降:** 専門Agentが加わってもアプリ境界、ユーザー所有データ、公開API、委譲権限と既存Gateを維持する。

## PDF Reader / Editorとの整合（2026-11-16）

[DEC-061](DEC-061-first-party-pdf-reader-editor.md)により、PDF Reader兼EditorをNagi Project / Smart Paste / Writer / Sheets / Slides等の文書作業へ接続する方向を追加する。Public Resource / Document / Object / Revision / Provenance / Action APIを基礎とし、通常GUI、元資料の権限・機密分類・分散正本を維持する。Studio等の既存名称の変更や実装済み連携を意味しない。 具体的な連携契約・適用版はPROVISIONAL。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 選択A：Nagiは作業・データ中心のOSを基本思想とし、アプリを目的達成の道具として扱うことを確定。従来の個別アプリ操作、第三者アプリの段階的統合、Permission境界を維持する。具体的な連携方式・実装時期は未確定。 |
| 2026-11-16 | DEC-061の純正PDF Reader / Editor方針への参照を追加。既存の公開API・AI非必須・権限境界と整合させ、実装版・既存Gateは変更しない。 |
