# DEC-108 — Display / GPU：Nagi独自ShellとOSS描画基盤の組合せ

**状態:** CONFIRMED（Foundation #19のB改良案：Nagi独自Shell／画面体験・端末横断の共通契約と、Linux／OSSの描画・GPU基盤の選択的活用）／PROVISIONAL（Wayland・Smithay等の個別採用、Compositor／Renderer詳細、機器対応、性能・版・受入）\
**最終合意日:** 2026-11-16\
**適用候補:** Nagi OS Foundation #19。特定Releaseへの割当ては未決定\
**関連:** [Foundation Checklist #19](NAGI_OS_FOUNDATION_CHECKLIST.md#f19)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-010](DEC-010-intent-first-adaptive-ui.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[純正ソフトウェア仕様 §27–30](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[Roadmaps](roadmaps/README.md)

## 目的とユーザー体験

Nagi Home／Desktop、Workspace、ウィンドウ管理、AIと通常GUIの協調、PC・タブレット・モバイル間の端末適応をNagi独自のOS体験として設計する。一方、画面合成、GPU・ディスプレイ制御など汎用的でハードウェアに近い層では、Linuxと適切なOSS技術の再利用を優先し、描画基盤を全面的に新規自作する負担を避ける。

## 確定した製品方針（CONFIRMED：B改良案）

1. **OS体験の上位層はNagi独自設計。** Nagi Home、Nagi Desktop、Workspace、ウィンドウ管理方針、端末に応じたUI・レイアウトの適応をNagiのShell／Platformの責務として構築する。OS体験を既存Linuxデスクトップ環境の外観変更だけで実現する方針にはしない。
2. **描画・GPU・ディスプレイの基盤はLinux／OSSを積極的に活用。** Linuxの既存GPU／Displayドライバーと、必要な描画・画面合成関連OSSコンポーネントを評価・再利用する。独自性のためだけに低レイヤーの全描画エンジンを自作する方式Aは標準方針にしない。
3. **PC・タブレット・モバイルの共通契約を保持。** 同じNagiのWorkspace／表示状態・画面管理契約を端末横断で維持しつつ、画面サイズ、入力方法、端末の能力と利用状況に応じてUIを適応させる。PC画面の単純縮小は標準としない。
4. **端末別の描画backend差を吸収できる設計。** ハードウェア・ドライバーの条件に応じて適切なbackendや最適化を選択できる構造とし、同一のGPU機能・同一のwindow配置・同一の性能を全端末へ強制しない。
5. **既存の確定UX・安全境界を維持。** DEC-009のHome／Desktop切替、複数ディスプレイのWorkspace切替・指定画面保持、DEC-010の新規Workspace配置／固定ウィンドウ保護／画面切断時の可視画面への退避、AI非必須の通常GUI操作、非特権AIとPermission／Policyを継承する。描画基盤の採用によって認可されたPublic Action経路を迂回しない。

## 例外・禁止・安全境界

- **具体的技術は未採用。** Wayland、Smithay、Mesa、DRM／KMS、特定Compositor、Renderer、Toolkit、GPU driver等は現時点では候補・検討対象であり、個別採用決定ではない。Linux／OSS活用は既存ディストリビューションの完成済みデスクトップ環境を製品基盤に採用することを意味しない（DEC-043）。
- **描画可能性と対応保証を区別。** 実機GPUの利用、動画／画面合成の性能、タッチ対応、複数画面の正常切替、Raspberry Pi／PineTab2／スマートフォン対応は、targetで個別に確認する。既存の旧guest成果をLinux版PASSに読み替えない。
- **UI管理とAI Actionを区別。** ウィンドウ状態の観測・配置や描画契約が、アプリ本文や他ユーザーデータへのアクセス、外部送信、AI Task開始・再実行を許可するものではない。
- 既存のDesktop first、各Release／Activation／Acceptance Gate、担当・実装開始条件を変更しない。この製品判断だけでLinux版実装や実機受入を完了扱いしない。

## 未確定事項（PROVISIONAL／OPEN）

- Wayland系の採否、Smithay等の候補評価、Compositorの自作部分／再利用範囲、描画API、buffer／surface／input・focusとの契約。
- DRM／KMS、Mesa、GPU driver／renderer、software fallback、hardware overlay、GPU隔離とAI推論等の描画・compute資源優先制御の具体構成。
- PC・Tablet・MobileのSurface契約、画面回転・DPI・マルチ画面、タッチ・ペン、画面の抜差し、異常時復元の詳細。
- 対応ハードウェア・firmware／driver、消費電力・描画遅延・フレームレートの測定と達成値、QA／securityの受入条件。
- 適用Version、実装順序、担当、Linux targetと実機試験結果。

## Roadmapとの関連

Foundation #19の方針は、Linuxカーネル＋Nagi専用ユーザーランド（DEC-043）とNagi独自Home／Desktop・Intent-first適応UI（DEC-009／010）を接続する。PC／Desktop firstを維持し、Tablet／Mobileへの製品目標と共通設計を前提にする。ただし対応端末・提供Versionや1.0の必須条件は増やさない。

## 2026-11-25 — Vello採用方針（DEC-127）

[DEC-127](DEC-127-nagi-design-system.md)により、Nagi独自の共通2D描画APIの主要描画基盤としてVelloを採用する方向をCONFIRMEDとする。Velloについては本追補が従前の個別技術未採用の記述に優先する。初期はCPU版を第一候補とし、GPU版は検証に応じた高速化候補。具体版・依存・ライセンス・実装・性能・機器対応・提供版はPROVISIONAL。Wayland／Smithay、Compositor、Mesa／DRM／KMS等の採否は別判断であり、既存Gate／Owner／着手条件を変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがAndroidの「独自OS体験の上位層＋ハードウェア寄り技術の再利用」との比較を確認し、Foundation #19の「B改良案：Nagi独自Shell＋OSS描画基盤」を明示採用。PC／タブレット／モバイル共通契約と端末適応を維持。Wayland／Smithay等の具体採用、対応実機・性能・実装版は未決定。 |
| 2026-11-25 | DEC-127で共通2D描画のVello採用方針を確定。その他の描画・合成基盤の採否と実装・受入は未確定。 |
