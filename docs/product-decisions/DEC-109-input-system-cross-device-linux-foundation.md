# DEC-109 — Input System：Linux入力基盤とNagi共通Input Service

**状態:** CONFIRMED（Foundation #20の方式B：Linuxの既存入力基盤の活用と、Keyboard／Mouse／Touchを共通Nagi Input Serviceで扱う端末横断方針、Wayland・Smithayと独立した意味的操作体系）／PROVISIONAL（具体Linux stack・IME接続・focus／seat契約・対応device・遅延値・版・受入）\
**最終合意日:** 2026-11-25\
**適用候補:** Nagi OS Foundation #20。特定Releaseへの割当ては未確定\
**関連:** [Foundation Checklist #20](NAGI_OS_FOUNDATION_CHECKLIST.md#f20)、[DEC-108](DEC-108-display-gpu-nagi-shell-oss-graphics.md)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-010](DEC-010-intent-first-adaptive-ui.md)、[DEC-060](DEC-060-responsive-ime-ai-learning.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[Roadmaps](roadmaps/README.md)

## 目的とユーザー体験

PCのキーボード・マウス、タブレット／スマートフォンのタッチなど、端末ごとに異なる入力手段をNagiの共通入力契約で扱い、Nagi Home／Desktopと通常のNagi Native Appを一貫して操作できるようにする。Linuxで成熟した機器入力処理を再利用しつつ、Nagiのfocus・shortcut・IME・Permissionと端末適応の責務を設計する。

## 確定した製品方針（CONFIRMED：方式B）

1. **Linuxの既存入力基盤を活用する。** キーボード、マウス、タッチ等の低レイヤー入力取得に、適切なLinux driver／既存OSS入力部品を選択的に利用する。機器入力処理を一から広範囲に自作する方式Cを標準方針にしない。
2. **Nagi Input Serviceによる共通管理。** キーボード・マウス・タッチ等の入力をNagi側の共通Input Service／入力契約で扱い、Nagi Home、Desktop、Workspace、アプリと連携させる。PCのポインター入力だけを前提とするUIにしない。
3. **PC・Tablet・Mobileの端末横断設計。** PCではキーボードとマウス、タブレット／モバイルではタッチ等、端末の画面・入力能力に応じて操作UIを適応させる。具体的なタッチジェスチャー、ペン対応、ソフトウェアキーボードの仕様や動作保証は別途確定する。
4. **既存IME方針を継承する。** [DEC-060](DEC-060-responsive-ime-ai-learning.md)の日本語IMEの入力応答優先、許可された情報に基づくAI学習、秘密入力の非学習、AIなしでの通常入力、モデル非依存の学習状態を維持する。IME自体を低レイヤー入力driverと同一視しない。
5. **通常操作・安全境界を維持する。** AIが利用不能でも通常のキーボード・マウス・タッチ操作とIMEが成立する方向とし、Nagi Agent／Modelが入力イベントの取得や画面操作を理由にPermission／Policyを迂回しない。イベント入力と認可されたAction実行は別に判断する。

## 例外・禁止・安全境界

- アプリのfocus・shortcut・Text Input・IME composition・アクセス可能な入力情報は、User／Session／Profile／Permission／Privacy、秘密入力の保護に従う。入力イベントを全量保存・AIに共有する包括許可にはしない。
- 入力デバイスを認識・接続しただけで、アプリやAIへの任意の入力監視・操作権限を与えない。遠隔端末のTrustとローカル入力機器の認識も混同しない。
- Nagi Input Serviceの採用は、特定のLinux input stack（evdev、libinput等）、Wayland、IMEエンジンの採用決定ではない。
- 既存Desktop first、実機候補とRelease／Activation／Acceptance Gateを変更しない。方針確定はLinux target上のtouch・IME等の実動作を証明しない。

## 未確定事項（PROVISIONAL／OPEN）

- Linux evdev／libinput等の部品採用、Nagi Input Serviceのプロセス／API／device→seat→focus→app契約、Wayland／Compositor境界。
- IME／Text Input protocol、composition、secure入力、IME candidate描画、language／keyboard layout、shortcut／focus優先規則。
- Touch gesture、pen、ソフトウェアキーボード、外部入力機器接続・取り外し、複数seat・アクセシビリティの対応範囲。
- device別driverとfirmware、Linux target／実機（PC・Tablet・Mobile）の対応条件・受入結果。
- 入力遅延とばらつきの測定方法・数値目標・負荷時優先度、版・順序・実装担当。

## Roadmapとの関連

[DEC-108](DEC-108-display-gpu-nagi-shell-oss-graphics.md)で確定した「Nagi独自Shell＋Linux／OSS描画基盤」と整合させ、Nagi固有の画面体験は維持しつつ、入力デバイスの低レイヤーを適切に再利用する。[DEC-010](DEC-010-intent-first-adaptive-ui.md)のPC・タブレット・モバイルへ適応する製品目標とDesktop firstを維持する。0.2／0.3／将来版への新規割当てやAcceptance変更は行わない。

## 2026-11-25 — 通常入力の基盤と将来の意味的操作の分離

[DEC-108](DEC-108-display-gpu-nagi-shell-oss-graphics.md)により、Wayland・Smithayを現在の2D画面表示・通常入力の基盤として条件付き採用する方向をCONFIRMEDとする。Nagi Input ServiceとNagi独自の操作体系はそれらの入力型・座標に固定せず、入力手段と意図・対象の解釈、認可された操作を分離する。正確なポインティングに依存しない長期ビジョン（VISION-001／DEC-097）と、DEC-020の正規Public Action・非特権AI、DEC-060の通常IMEと秘密入力保護、AI不在時の通常GUIを維持する。Waylandの採否については本追補を優先する。具体的な入力stack・focus／seat・protocol／adapter・新しい入力技術・実装・性能・提供版はPROVISIONALであり、既存Gate／Ownerを変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #20の方式Bを承認。Linux入力基盤を活用し、キーボード・マウス・タッチを共通Nagi Input Serviceで扱う。PC・Tablet・Mobileの適応設計とDEC-060のIME方針を継承。具体stack・IME契約・対象機器・性能・提供版・受入は未確定。 |
| 2026-11-25 | DEC-108の条件付き採用と、通常入力・意図解釈・認可された操作の分離を反映。新規入力技術と具体実装は未確定。 |
