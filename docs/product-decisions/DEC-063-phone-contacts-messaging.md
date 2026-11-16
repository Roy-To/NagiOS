# DEC-063 — 電話・連絡先・SMSを端末横断の純正アプリ体系へ採用

**状態:** CONFIRMED（採用対象・共通モデル・適応UI・能力と認可に基づく提供）／PROVISIONAL（アプリ構成・通信方式・連携詳細・実装版）

**最終合意日:** 2026-11-16

**適用候補:** Nagi OSの将来の純正アプリ／機能。対応機種・実装時期・適用版は未確定

**関連:** [純正アプリ仕様](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md) §27–30・60–61・64–65・80–90、[共通端末・アプリモデル](../architecture/unified-device-application-model.md)、[0.2仕様](../Nagi_OS_0.2_Codex_Implementation_Spec.md) §15、[DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-003](DEC-003-memory-classification.md)、[DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-010](DEC-010-intent-first-adaptive-ui.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-052](DEC-052-physical-test-device-candidates.md)

## 目的とユーザー体験

電話・連絡先・SMS/Messagingの基本機能をNagi純正アプリ／機能として揃える。NagiはPC・タブレット・モバイルを横断するOSであり、これらをモバイル専用アプリとして切り離さず、同じアプリ体系の中で利用する。

## 確定した仕様 — CONFIRMED

### 採用対象と端末横断の原則

- **電話、連絡先、SMS/Messagingの基本機能を採用対象とする。** 3個の独立アプリとしての分割や名称を確定する決定ではない。
- 端末種類により機能群を閉じ込めず、共通アプリ／共通の意味モデルと適応UIを原則とする。画面寸法、入力方法、利用可能なCapabilityに応じて表示・操作を適応させる。Desktop firstという実装順序と、PC専用・モバイル専用という製品分断を混同しない。
- 同じアプリ体系であることは、すべての端末に同じ通信能力があることや、データが自動同期されることを意味しない。

### 実際に利用できる機能の条件

- 通話、SMS送受信、通話／メッセージの端末間連携は、利用可能なハードウェア、モデム／SIM／eSIM、OS能力、ユーザー認可、および必要なネットワーク・外部サービス・キャリアのサポートに依存する。
- PC／タブレットでも、権限と利用可能なデータの範囲で連絡先管理、履歴の閲覧、下書き等は成立し得る。これらは利用形態の例であり、端末間の履歴同期や遠隔送信の実装を確定するものではない。
- **通話／SMSの送受信を全デバイスに無条件保証しない。** 端末の種類だけで可否を決めず、実際のCapabilityと現行の認可を確認する。未対応・権限不足・オフライン・失敗・結果不明を、送信済みや通話成功として表示しない。

### Peopleと既存アプリ・共通基盤との整合

- 連絡先は、可能な限り既存の**People共通モデルへ統合**する。純正アプリ仕様の第64章PeopleにあるPerson／Organization／Contact point／Phone／provider identityを基盤とし、独立した住所録や人物identityを重複して設けない。
- Peopleの既存方針を維持する。弱い根拠による人物の自動mergeはせず、mergeは可逆かつActivity記録対象とする。権限がある関連情報をReferenceで接続し、通信本文をPeople DBへ大量複製しない。人物評価や関係推定の採用には拡張しない。
- Mail／Calendarの既存機能・provider契約を置き換えない。共通の人物・連絡先参照を活用し、SMS等の通信経路をメールと同一視しない。
- Device／Devices、Continuity、Permission、Notification、Workspaces、Activity等の共通基盤を再利用する。共通アプリであることや同一ユーザーの所有端末であることから、別端末の信頼・権限や遠隔操作権限を自動的に得ない。

## 例外・禁止・安全境界 — CONFIRMED

- 通話・連絡先・メッセージとその履歴は機密性の高い情報として、現行Permission／Capability／Policy、User／Profile、端末、アカウント・回線、相手先の境界を尊重する。読む権限、下書きする権限、送信・発信する権限を同一視しない。WorkspaceやPeople上で関連付けてもアクセス・共有・送信権限は増えない。
- 発信・送信は外部副作用として、必要なユーザー確認と**送信・発信に対する明示承認**を適用する。宛先・内容・使用する端末／回線・操作範囲を推測で拡張しない。下書き生成やAIの提案、着信・受信イベントは送信承認にならない。
- DEC-001とMail §60.5に従い、明示依頼または明示的な事前承認が対象・操作・範囲を満たす場合は、現行Policyの範囲内で重複確認を省ける。曖昧な宛先、範囲外の操作や認可不足は確認に戻す。本DECは包括的な無人送信許可や新たな自動化方式を採用しない。
- 着信・受信通知は共通NotificationのProfile分離・privacy・quiet mode等の方針と整合させる。通知を受け取ることと、本文閲覧・応答・発信／送信の権限を分け、通知Actionの実行時にも現在の権限を再評価する。Notification coreはSMS通信や任意の遠隔配送そのものを担わない（0.2仕様 §15）。通知先端末、ロック画面の詳細表示、割込みUIは未確定。
- 履歴の保持・削除は既存のデータ保護、Activity、Memory／忘却、Wayback、Providerの境界に従う。機密本文をActivityへ無条件複製しない。アプリ履歴・派生索引／要約・Memory・バックアップ・相手先や外部サービス側の記録を区別し、実際の削除範囲を正確に示す。送信済みの通信や実施済みの通話をUndo／履歴削除で取り消せると表示しない。具体的な保持期間・削除連動方式は未確定。
- アプリ採用はSMS本文等からのAI学習、通話の録音・文字起こし・音声解析、Cloud送信を一括承認するものではない。これらの採否・方式は未確定であり、既存の分類・認可・忘却の境界を維持する。

## 未確定・暫定案 — PROVISIONAL

- アプリを3個に分割するか、People／Phone／Messages等でどう構成するか。名称、最小機能、画面構成、適応UIの詳細。
- 対応プロトコル・キャリア・外部サービス、RCS等の採否、SMS以外のMessaging範囲、モデム／SIM／eSIMへの具体的対応方式。
- 端末間の遠隔操作・通話やメッセージの引き継ぎ・同期、ペアリング、通信経路、競合処理、通知転送、利用端末／回線の選択方式。
- SMS等からのAI学習、音声処理・録音・文字起こし、関連する法規制・キャリア要件の整理と対応。これらの適合や対応済みを本DECで保証しない。
- 対応機種・地域、実装担当・実装時期・適用版、段階的提供範囲、Acceptance条件。DEC-052の実機候補選定を通話／SMS対応の確約に読み替えない。
- Permissionの具体的粒度と確認UI、着信通知・機密表示の詳細、履歴の保持期間、削除・同期・バックアップ間の連動方式。

## Roadmapとの関連

- [0.3 Revised](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md): Profile／Permission-filtered Context、型付きAction、Memoryの境界を継承する。通信機能や通信データ学習を0.3へ自動追加しない。
- [0.4+](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md): Agentの役割や委譲で通信権限を増やさず、既存の単一ユーザー・単一端末からの段階的拡張方針を維持する。
- [0.5–1.0](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md): multi-deviceや同期の版割当ては引き続き暫定。共通アプリモデルの採用と、実際の通信・同期サービスの有効化を区別する。
- 本記録は製品採用方針であり、コード実装、端末／キャリア受入、実装開始やreleaseの承認を意味しない。既存の版境界、担当、release GateとAcceptanceを変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | 電話・連絡先・SMS/Messagingを純正の採用対象として確定。共通アプリ／モデルと適応UI、実能力・認可依存、People統合原則を記録し、アプリ構成・通信／端末間連携・版等を未確定として分離。 |
