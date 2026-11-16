# DEC-106 — Device Manager：内部の責務分離と統合管理UI

**状態:** CONFIRMED（Foundation #17の方式B：ローカル周辺機器管理と他のNagi端末管理の内部責務分離、Settings／DevicesのUI統合、Linux機器基盤の活用）／PROVISIONAL（具体stack・対象機器・UI細部・適用版・受入）  
**最終合意日:** 2026-11-16  
**適用候補:** Nagi OS Foundation #17。特定のReleaseは未確定  
**関連:** [Foundation Checklist #17](NAGI_OS_FOUNDATION_CHECKLIST.md#f17)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-066](DEC-066-device-connectivity-lan-first.md)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[純正ソフトウェア仕様 §65](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[Roadmaps](roadmaps/README.md)

## 目的とユーザー体験

ローカルのUSB、GPU、ネットワーク、入力・音声機器などと、他のNagi端末を、ユーザーがNagi Settings／Devicesから一貫して管理できるようにする。一方で、両者の内部的な認識・権限・信頼の責務は混同しない。

## 確定した仕様（CONFIRMED：方式B）

1. **内部の機器管理責務は分離する。** Linuxホスト側のローカル周辺機器の認識・状態・権限・イベント管理と、接続する他のNagi端末の発見・信頼・連携管理は、異なる内部サービス／責務として扱う。
2. **管理UIは統合する。** Nagi Settings／Devicesのユーザー向け管理体験では、ローカル周辺機器と他のNagi端末をまとめて確認・操作できるようにする。具体的な画面・ナビゲーション構成は未確定。
3. **Linuxの既存機器基盤を活用する。** Linuxの適切な既存ドライバー・機器管理機構を活用し、Nagi側のDevice Service／管理層が状態・権限・追加／取り外し等のイベントをNagiの機能へつなぐ。特定のLinux daemonやSDKを確定するものではない。
4. **権限と信頼は区別する。** ローカル機器の検出と、他のNagi端末の `untrusted／pending／trusted／revoked`、ならびにアプリの実際の機器利用許可、データ送受信・Actionの許可はそれぞれ別の判断とする。既存Permission／Privacy／PolicyとDEC-066のTrust／明示許可を維持する。
5. **機器の認識だけでは対応保証としない。** Linuxに対応ドライバーがあってもNagiで全機器が正式対応・実機動作済みとは扱わない。

## 例外・禁止・安全境界

- ローカル周辺機器を認識しただけで無制限なアプリ利用権限を与えない。Nagi AIが非公開の低レベル経路を使って許可を迂回しない。
- 他のNagi端末が発見・信頼済みになっても、無断で同期、転送、AI Taskの再実行を行わない。
- 既存Release／Activation／Acceptance Gate、Desktop first、DEC-052の実機候補を変更せず、記録のみで実装済みとは扱わない。

## 未確定事項（PROVISIONAL／OPEN）

- udev等の機器管理stackの採否、Nagi Device Serviceの具体API／process構成、hotplug状態機械、firmware更新方法。
- 初期対象bus／機器クラス、切断・再接続時の検証、対応driverと端末。
- 設定画面の分類、機器状態の診断・エラーUI、remote Devicesとの画面共有範囲。
- リリース・開発順序、Linux target／実機試験・性能・受入条件。

## Roadmapとの関連

Linux + Nagi専用ユーザーランド（DEC-043）とNagi Devices／Trust（DEC-066、純正仕様§65）の接続点となる方針。Foundation #18〜22／#32〜33の詳細技術と、各Roadmapの割当て・Gateを先取りして確定しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | Foundation #17でユーザーがB方式を明示承認。内部責務は分離、管理UIは統合、Linux既存機器基盤の活用とNagi側管理層による状態・権限・イベント連携を採用。詳細stack・対象機器・対応保証・実装版は未確定。 |
