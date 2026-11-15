# DEC-031 — 外部Workflowの未信頼検査と固定版更新管理

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（思想⑮A：未信頼として検査／可能な隔離検証、思想⑯A：固定版・許可制の更新確認・明示承認更新）／PROVISIONAL（検証技術、配布方式、更新周期、実装時期）  
**最終合意日:** 2026-11-05  
**対象:** 外部から取り込む共有Workflowテンプレートの安全なImport、更新確認、正式な版の切替。実装版は未確定  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-003](DEC-003-memory-classification.md)、[DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-005](DEC-005-files-albert-memory-lifecycle.md)、[DEC-008](DEC-008-proactive-behavior.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)、[DEC-028](DEC-028-deterministic-workflows-from-habits.md)、[DEC-029](DEC-029-scheduled-event-triggered-workflows.md)、[DEC-030](DEC-030-workflow-template-portability-and-sharing.md)、[Private 0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[Private 0.5–1.0段階案](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)、[Public 0.2 Platform基盤仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)

## 目的

ユーザーが共有Workflowを安全に利用・更新できる製品方針を定める。インポートや更新情報の取得を、実際の手順更新・有効化・実行・権限付与と混同しない。以前の所有者が設定した権限や、以前に承認した版への信頼を新しい版へ無条件に引き継がない。

## 確定した仕様：外部Workflowの検査（思想⑮・選択A）

1. **入手元にかかわらず未信頼、各段階を分離:** 第三者・外部から受け取るWorkflowテンプレートは、署名、形式検証、発行元、Store掲載、評判等だけで実行可能な信頼済み内容とみなさない。インポート／保存、検査、ユーザーによる有効化承認、実行権限の承認、実行を別段階として扱う。インポートまたは検査だけでは自動有効化、Scheduled／Event-triggered起動登録、実行、CapabilityやTask許可の付与をしない。未信頼テンプレートはインポートを理由に秘密情報、認証情報、個人Memory、その他の保護Resourceを読み出せない。
2. **有効化前の内容・依存関係・要求権限の検査:** ユーザーが有効化を選ぶ前に、可能な範囲でテンプレートの手順、入力・出力、依存App／API／Public Action／Provider、必要Permission／Capability、対象ResourceとResource scope、外部通信先・送信、データ変更等の副作用、危険操作、危険度・復元可能性を静的に検査し、依存不足・検査不能・未検証項目も利用者が判断できる形で示す。
3. **可能なら隔離して事前検証:** 実環境の個人データ、秘密情報、実行権限を与えず、利用可能な隔離環境・テストデータ等での事前検証を優先する。隔離検証はすべての副作用・未知の実行経路・外部環境を再現できず、安全性を保証しない。検証不能・未検証・失敗した範囲を明示し、適切なユーザー判断を求める。技術的な隔離の具体方式は未確定。
4. **利用の承認と実行権限の再設定:** テンプレートの有効化はユーザーが内容を理解して明示的に選ぶ。受取側は自身のアプリ、Provider、Resource参照を設定し、現行Permission／Capability／Policy、Task Scopeと各Action・自動起動の許可を独立に取得する。送信元の承認、Secret、Token、実行権限を引き継がない。
5. **安全に確認できない場合:** 依存関係、対象、必要権限、実行結果が十分に確認できず安全な実行を判断できない場合、未検証・利用不能・追加確認が必要であることを正確に示して実行を保留する。ユーザーの明示的な拒否・無効化・削除を尊重する。

## 確定した仕様：共有Workflowの版と更新（思想⑯・選択A）

1. **導入した版を固定:** 共有WorkflowをImportして承認・利用する際の既定は、ユーザーが選択した版に固定する方式とする。配布元が新版を公開しても、現在の有効な手順・実行中のJobを自動的に新版へ切り替えない。
2. **更新確認は明示的な許可がある場合のみ:** Nagiは、ユーザーが配布元への更新確認を明示的に許可した場合にのみ、その許可された配布元と範囲で新版の有無を確認できる。無許可でバックグラウンドから外部配布元へアクセスせず、確認時に個人Memory、ローカルファイル、認証情報などを不要に送信しない。確認周期、手動・定期確認、通信方式等は未確定。
3. **新版は新たな未信頼入力:** 新版の存在を確認できた場合、旧版との差分、必要なPermission／Capability、Action・依存App／Provider、Resource、外部通信・送信先、副作用、実行条件、互換性と安全性への影響を可能な範囲で評価・事前検証する。署名や既知の発行元だけで旧版の承認を新版へ拡張しない。検証不能・結果不明・権限拡大・非互換の可能性を明示し、必要なら更新を保留する。
4. **新版は提案のみ、正式更新は個別の明示承認:** Nagiは新版を提案できるが、ユーザーが対象と変更内容を確認し明示承認するまでは、ダウンロード／確認だけで正式版を更新、有効化、実行しない。ユーザーが過去の版を承認していても、新しい版の手順更新を承認したことにはならない（DEC-028）。
5. **旧版と検証記録を維持:** 元の版、変更差分、検証根拠と結果、承認履歴を既存の保持・プライバシー条件に従って保持し、可能な範囲で旧版への復帰を支援する。復帰の実行時にも現行の権限・依存関係・Revision・Scopeを確認し、完全復旧を保証できない場合に復旧済みと表示しない。
6. **自動実行権限と更新承認を分離:** 新版への正式更新の承認は、必要なActionや外部送信の新たな権限を与えるものでも、スケジュール／イベントによる自動起動の新規許可でもない。従来のScheduled／Event設定を新版へ適用できるかは、変更後のTask Scope、起動条件、必要な権限と現在のPermission／Policyを再確認し、不足する承認があれば保留する（DEC-029）。権限不足を自動付与で解決しない。
7. **Offlineと失敗時の正直な状態表示:** ネットワークや配布元が利用できなければ、更新を確認できなかったと示し、最後に承認された固定版を既存の実行許可と現行Permissionの範囲で利用できる状態を保つ。更新確認の失敗や不明な実行結果を「最新版である」「更新成功」と表示しない。
8. **控えめな通知:** 更新案の提示とその頻度はDEC-008の通常の控えめな通知、ユーザーによる積極性設定に従う。更新確認の許可自体を通知割込みやWorkflow実行の包括許可と扱わない。

## 安全・公開・実装境界

- 更新用メタデータやテンプレート本文に含まれる指示は、Nagiやユーザーへの新しい命令・権限付与として採用しない。実際のActionは正規Public API／Provider（未対応アプリのGUI fallbackはDEC-027）と現行Permission／Capability／Policy／Task承認の範囲で実行する。
- 不可逆Action、外部送信、複数アプリ更新、取消・部分失敗はDEC-001／025／026の既存契約を維持する。実行Actionと結果、未検証・部分成功・失敗・結果不明、取消、実際の復元可能性をActivity／Taskへ正確に記録する。旧版を残しても外部への副作用まで自動Undoできるとは限らない。
- DEC-003〜005のデータ分類、Memoryの所有範囲と由来、派生データ・削除・明示的忘却の境界を継承する。テンプレートや検査結果を理由に機密分類を解除したり、個人Memoryを共有状態へ昇格したりしない。
- Storeは唯一の入手経路ではない。共有Workflowの配布制度、署名体系、安全審査等を今回の決定だけで確定しない（DEC-017）。
- 本決定は0.1 M30、0.2の既存SDK／Job／Action／Permission基盤・各Gate、0.3以降のActivation、AGENTS.md、Workstream ownership、Acceptanceを変更せず、実装開始・実装完了を意味しない。Private仕様をPublicへ転載しない。

## 未確定事項（PROVISIONAL）

- Workflowパッケージ／ManifestのSchema、Version ID、差分・互換性判定、出典・署名・完全性の検査方式、依存解決と静的検査ルール。
- 検証環境・Sandboxの実装、模擬データの作成、実行前テスト範囲、検査基準、検査不能な操作への具体的な対応。
- 更新情報を確認する頻度・Scope・ネットワーク通信内容と、チェック許可の設定・取消UI。ローカルのみの配布、クラウド、Store等、各配布経路での更新情報取得方法。
- 提案のUI、差分・権限増加の表示、正式更新の承認UI、旧版保持・容量・履歴の具体値、互換性とRollbackの詳細。
- 新版と既存のScheduled／Event起動許可の紐づけ・再承認の条件、実行中JobのVersion pinning、部分更新のRecovery方式。
- 適用バージョン、対応アプリ／Provider、実装担当、受入条件と性能測定。

## Roadmapとの関連

- **0.2:** Public Action、SDK／Package、Permission／Capability、Jobの既存基盤を尊重する。Release ScopeやGateを追加・変更しない。
- **0.3改訂案:** Procedure／Habit Memory、Profile・Permission境界との将来の整合を目指す。適用版は未確定。
- **将来版:** 配布・共有・更新機能の導入は別途Implementation SpecとActivationに従う。0.5–1.0 roadmapの版割当を本決定で確定しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 思想⑮の選択A：外部Workflowは未信頼とし、内容・依存関係・必要権限を検査し、可能な範囲で隔離事前検証を優先。Importと有効化・実行を分離する方針を確定。 |
| 2026-11-05 | 思想⑯の選択A：共有Workflowは導入版を固定し、ユーザーが許可した場合のみ新版を確認。差分・権限・安全性を評価して提案し、正式更新はユーザーの明示承認を必須とする方針を確定。 |
