# DEC-066 — 端末間接続：同一LANから始める共通Connectivity基盤

**状態:** CONFIRMED（初期接続ターゲット・接続方式を固定しない設計・安全境界）／PROVISIONAL（名称・具体方式・UI・遠隔構成・運用費用・対象機器・時期・性能）  
**最終合意日:** 2026-11-16  
**適用候補:** Nagi Share／Handoff等のNagi端末間連携。特定リリースへの割当ては未確定  
**関連:** [DEC-002](DEC-002-cloud-ai-data-sharing.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[端末候補DEC-052](DEC-052-physical-test-device-candidates.md)、[純正ソフトウェア仕様](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[ロードマップ索引](roadmaps/README.md)

## 目的とユーザー体験

Nagi Share／Handoff等が使う端末間接続を、まず同一Wi-Fi／LAN上のNagi端末から成立させる。離れた場所にあるNagi端末とのインターネット越し接続も将来の拡張可能性として残し、初期のローカル連携を常時クラウド接続やサービス運営に依存させない。

## 確定した仕様（CONFIRMED）

- **初期接続ターゲット:** 同一Wi-Fi／LAN上のNagi端末を対象とする。これは端末間連携の初期ターゲットであり、OS全体の常時ネットワーク接続要件や特定リリースの必須機能を追加する決定ではない。
- **共通の抽象:** Device Connectivity／Trust／Transportの共通抽象を整備し、Share／Handoff等の体験と、発見・接続・信頼・転送の具体方式を分離する。アプリや機能に同一LAN専用の接続方法をハードコードせず、接続経路の追加で共通の認証・認可境界を迂回しない構造とする。正式なサービス名・API・Schemaを本決定で固定しない。
- **段階的な拡張:** 近距離の直接Wi-Fi通信等を次の候補として検討できる設計とする。その次に、インターネット越しの遠隔端末接続も拡張可能性として保持する。VPN／P2P、必要なら中継サービスやオンラインNagiアカウントを利用する可能性を閉じないが、いずれも採用方式・提供順序・実装完了の約束ではない。
- **初期に運営基盤を必須化しない:** 遠隔接続の実装、Cloudの常時接続、オンラインNagiアカウント、Nagi運営によるサーバーの設置・維持を初期要件としない。将来のrelayやアカウント構成の検討から、運用主体・費用負担・サービス提供の決定を導かない。
- **Offline-first:** ローカル利用を維持し、同一LAN内連携にインターネットやCloudを必須としない。ただし、実際の到達性・対応能力・現行権限・必要なファイル実体が揃わない場合まで連携成功を保証しない。

## 例外・禁止・安全境界

- **端末認証・信頼と暗号化:** 相手端末の認証、信頼状態、暗号化を必須境界とする。同一LANにいること、近くにいること、端末名、アカウントへの登録、接続・暗号化の成功だけでtrustedやデータ操作認可としない。Device Managerのuntrusted／pending／trusted／revokedと整合させる。
- **ユーザーの明示許可と現行認可:** 送受信先・対象データ・目的・範囲を明示的に許可された内側に限定し、User／Profile／Owner、Resource Permission、Privacy、Capability／Policy、機密分類を各正規経路で適用する。信頼登録と、読む・送る・受け取る・保存する・同期する・Actionを実行する許可を同一視しない。
- **取り消しとScope:** 接続の信頼・共有許可を取り消せるようにし、取り消し後の接続・送受信・適用で現行状態を再検証する。外部通信は宛先、サービス、情報区分、目的、Profile、期間・Task等の許可Scopeに従う。端末間接続の許可をCloud AI、relay、アカウントサービスへの包括的送信許可にしない。DEC-002の外部送信・許可取消の境界を維持し、取消だけで送信済みデータの回収・外部削除を保証しない。
- **無断同期・転送・Task再実行なし:** 到達可能・trustedになったことを契機に、勝手なデータ同期・転送、中断AI Taskや副作用Actionの自動再開・再実行をしない。Source有効性、明示的忘却、Revisionと競合を再評価し、未確認・失敗・部分結果を成功と表示しない。
- **DEC-014を維持:** Workspaceの標準である暗号化Export／Import、端末ごとの独立管理、明示選択する移行／同期、将来の暗号化直接同期とCloud同期の別許可を維持する。接続基盤の採用は標準移行方式の置換、同期の既定有効化、本文・アプリ・端末設定の自動移行ではない。
- **機能採用の範囲:** 本決定は接続方針であり、Shareの単発ファイル転送とHandoffの意味的な作業継続の区別を維持する。直前に提案されたNagi Clipboardは未採用であり、今回の合意からClipboard採用やクリップボード同期を導かない。

## 既存・並行記録との関係（REVIEW NOTE、2026-11-16）

本記録の作成時点で、Nagi ShareのDEC-064は[PR #125](https://github.com/Roy-To/NagiOS/pull/125)、Nagi HandoffのDEC-065は[PR #126](https://github.com/Roy-To/NagiOS/pull/126)に記録され、どちらもmain未マージである。本DECは最新mainから独立して記録し、両PRの内容を取り込んだり、それらのマージ済み・実装済みを主張したりしない。

両DECに残る「接続方式は未確定」は、本DECの初期LANターゲット・共通抽象という確定方針の内側で、具体技術が未確定であると読む。ShareにおけるHandoff未採用という記述はShare決定時点の範囲であり、後続DEC-065の独立した採用決定と区別する。接続基盤を共有しても、ShareをHandoffの必須転送方式にはしない。

## 未確定事項（PROVISIONAL）

- サービス・機能の正式名称、共通抽象のAPI／Schemaやプロセス構成。
- mDNS、Bluetooth、Wi-Fi Direct等の発見・接続方式、プロトコル、暗号・鍵交換・鍵保管・失効の具体方式。
- ペアリングUI、発見可能性、信頼登録・送受信確認・取り消し・障害時の表示詳細。
- 遠隔P2P、NAT traversal、VPN、relayの採用選択と構成、オンラインNagiアカウントの採否・役割。
- クラウド運用主体、サーバー設置・維持、費用・料金・継続運用条件。
- 対象機器・OS間互換性、リリース割当て、実装順序・時期・担当、受入条件。既存の実機候補選定は正式対応の保証ではない。
- 速度、遅延、接続／転送成功率、到達性、容量・同時接続数等の性能値と保証。

## Roadmapとの関連

純正ソフトウェア仕様の§27–30 Device／Surface／Capability／Continuity、§65 Device Manager／Trust／Handoff、§97の意味的Continuity契約を再利用する。§65のremote job observationは、直ちにインターネット越し接続や全面的remote controlを初期必須にする記述ではない。

0.3の将来Nagi Account Sync、0.4+のMulti-device／Remote Agent、0.5〜1.0の複数端末展望は本方針の内側で検討する。いずれにも接続機能の提供版・時期を割り当てず、Desktop first、既存Release／Activation／Acceptance Gate、担当・着手条件を変更しない。本記録は製品方針であり、接続基盤の実装、実機接続・遠隔通信の成功を示さない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | 同一Wi-Fi／LANを初期ターゲットとし、Device Connectivity／Trust／Transportを共通化する方針を確定。近距離直接通信と遠隔接続の拡張可能性を保持し、初期の遠隔実装・常時Cloud・運営サーバーを必須化しない。Offline-first、認証・信頼・暗号化・明示許可・現行権限・Privacy・取消・外部Scope、DEC-014、無断同期／転送／Task再実行禁止を維持。具体方式・運用・時期・性能は未確定、Clipboardは未採用。 |
