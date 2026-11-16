# DEC-092 — 周辺機器：USBストレージ・基本HIDを標準対象とする段階対応

**状態:** CONFIRMED（Foundation #22の方式B：USBストレージ・基本HIDを標準対象、カメラ・プリンター・スキャナーの段階対応、Linux／OSS活用とNagi管理・権限・通常UI連携）／PROVISIONAL（具体機種・対応範囲・API・プロトコル・実装版・実機受入）
**最終合意日:** 2026-11-16
**適用候補:** Nagi OS Foundation #22。特定Releaseへの割当ては未確定
**関連:** [Foundation Checklist #22](NAGI_OS_FOUNDATION_CHECKLIST.md#f22)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-106](DEC-106-device-manager-local-remote-unified-ui.md)、[DEC-107](DEC-107-network-devices-ethernet-wifi-bluetooth-staged.md)、[DEC-109](DEC-109-input-system-cross-device-linux-foundation.md)、[Roadmaps](roadmaps/README.md)

## 目的とユーザー体験

日常の外部データ利用と基本入力を優先し、周辺機器をNagiの機器管理・権限・プライバシー・通常UIへ接続する。ユーザーがFoundation #22の方式Bを明確に承認した内容を記録する。

## 確定した仕様（CONFIRMED：方式B）

1. **USBストレージを標準対象とする。** USBメモリ・外付SSDをNagi OSの標準的な周辺機器の製品対象に含める。具体的な機種・接続規格・filesystem・操作範囲は別途定める。
2. **基本的なHID入力機器を標準対象とする。** キーボード・マウス等の基本入力機器を対象とし、DEC-109のNagi Input Service／共通入力契約と連携する。すべてのHID機能や特殊機器への対応を確定するものではない。
3. **カメラ・プリンター・スキャナーは段階的に対応する。** 製品対象から除外せず、機器・機能ごとに対応を進める。全機能の初期実装や提供順序は確定しない。
4. **Linuxの既存ドライバーとOSSを活用する。** DEC-043の選択的OSS利用とDEC-106の機器管理方針に従い、既存の低レイヤー基盤を再利用し、Nagi Device Manager／Permission／Privacy／通常UIと連携する。特定のdaemon・ライブラリ・API・プロトコルは本決定では選定しない。
5. **全Linux対応機器の動作保証とはしない。** Linux側のdriverの存在、機器の認識、Nagiでの機能対応、実機受入を区別する。標準対象という製品方針だけで、特定機種の正式対応・実装済み・動作確認済みとは扱わない。

## 例外・安全境界

- 機器の接続・認識だけでアプリやAIへ包括的な利用権限を与えない。ストレージ内データ、撮影・スキャン内容、印刷対象も既存Permission／Privacy／Policyに従う。
- AIが利用不能でも通常UIの基本操作を維持し、AIによる操作は既存の型付きPublic Actionと認可境界を迂回しない。
- DEC-106のローカル周辺機器と他のNagi端末の責務分離、DEC-107のBluetooth段階対応、DEC-109の共通入力・秘密入力保護を維持する。基本HIDを標準対象とすることはBluetooth全機器の初期対応を意味しない。

## 未確定事項（PROVISIONAL／OPEN）

- 対応する具体機種・driver／firmware・接続規格と機能範囲、未対応機器の表示。
- USBストレージのfilesystem・mount・安全な取り外し・読み書き等の対応範囲、切断・失敗時の扱い。
- HIDの対象機能、カメラの撮影、印刷、スキャンの各機能範囲と段階・順序。
- 採用OSS、Nagi側API／プロトコル・サービス構成、通常UIとPermission／Privacyの具体連携。
- 実装版・担当・時期、Linux image／実機の受入条件と結果。実機動作・性能は未確認。

## Roadmap・実装との境界

Foundation #13（Storage / Volume）、#17（Device Manager）、#18（Network Devices）、#20（Input System）、#21（Audio System）、#33（Hardware Support）と整合させる。0.2／0.3／0.4+／0.5–1.0の版割当て、Desktop first、既存Release／Activation／Acceptance Gateを変更しない。本記録は製品方針の保存であり、実装・実機受入の完了ではない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #22の方式Bを明確に承認。USBストレージ（USBメモリ・外付SSD）と基本HIDを標準対象とし、カメラ・プリンター・スキャナーは段階対応。Linux既存driver／OSSとNagi管理・権限・Privacy・通常UIを連携。全Linux対応機器の動作保証はせず、具体機種・対応範囲・API・プロトコル・実装版・実機受入は未確定。未マージPR #134でDEC-088〜091が使用済みのためDEC-092を採番。 |
