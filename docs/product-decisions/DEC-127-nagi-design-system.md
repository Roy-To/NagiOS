# DEC-127 — Nagi Design System：ミニマルな共通UIと創作の自由

**状態:** CONFIRMED（ミニマルでフラットなデザイン、共通UIによる開発支援と独自デザインの自由の両立、Desktopの軽量性を重視）／PROVISIONAL（具体配色・部品・外観・API・技術・提供版）
**最終合意日:** 2026-11-25
**適用候補:** Nagi純正アプリ・公開SDK／共通UI基盤。具体的な提供版は未確定。
**関連:** [DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-010](DEC-010-intent-first-adaptive-ui.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-071](DEC-071-nagi-native-app-creation-experience.md)、[DEC-097](DEC-097-accessibility-foundation-and-ai-native-long-term-direction.md)、[DEC-108](DEC-108-display-gpu-nagi-shell-oss-graphics.md)、[DEC-109](DEC-109-input-system-cross-device-linux-foundation.md)、[0.2仕様 M07](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[純正アプリ実装仕様](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)

## 目的とユーザー体験

初心者でも統一感があり心地よいアプリを作りやすく、専門家は独自の表現を追求できる。純正アプリの共通基盤による実装負担の軽減と、第三者開発者の創作の自由を両立する。

## 確定した仕様（CONFIRMED）

- ユーザーが望むヘルシーな印象、**ミニマルでフラットなデザイン**をNagi純正UIの高水準の方向とする。装飾より可読性と操作の理解しやすさを重視する。
- 共通Design System／UI部品を純正・第三者に利用可能な公開基盤として整備する方向を採用する。統一感のある標準部品の利用を容易にし、第三者アプリに一律の画面構成・外観・共通部品の採用を強制しない。独自UI・拡張部品も正規の公開契約と安全境界に従って利用できる方向を維持する。
- 既存0.2 M07、日英Localization、通常のアクセシビリティの先行整備、AIなしの通常GUI、現在のPermission／Capability／Policy、Public Action優先とユーザーの手動配置保護を維持する。表示上の統一感や意味情報が認可を代替することはない。

## 例外・禁止・安全境界

- 独自UIを採用してもOSの正式な権限承認、認証・システムUIの信頼境界を迂回・偽装してはならない。意味情報・accessibility tree・AI認識が追加の操作権限を与えることはない。
- AIの常設パネルや推論を通常操作の条件にせず、既存の低負荷・日英・入力／フォーカス・アクセシビリティ方針を尊重する。
- 本決定は既存Release／Activation／Acceptance Gate、Workstream ownerや開始条件、特定実装の完成・提供時期を変更しない。

## 未確定・暫定案（PROVISIONAL）

- 白／淡いグレーを基調としたLight、Dark、アクセント色、余白・角丸・文字サイズ・フォント・アイコン・アニメーション・コントラスト、OS設定への追従等の具体design tokenとテーマ。
- Standard／Custom／Independent UIという利用方式の分類、共通Sidebar／Inspector／Data Grid／Tree／Timeline／Editor／Canvas／Command Palette／Dialog／Progress等の部品と拡張・独自描画のAPI。
- 構造化されたUI意味情報、セマンティックな状態・Accessibility、公開ガイド（Nagi Human Interface Guidelines仮称）、人とAI向け資料・サンプル、検証ツール、実際の画面レイアウト。
- 描画backend、具体UI framework、サポート範囲、性能／a11y受入、版・着手順は未確定。コンセプト画像は視覚的検討材料であり確定デザイン・稼働画面ではない。

## Roadmapとの関連

0.2 M07とLocalization等の共通契約を再利用し、後続純正アプリ・公開SDK・Builderによる開発支援へ発展させる。Desktop firstと端末適応方向を維持する。

## 2026-11-25 — Desktopの軽量性を重視

**CONFIRMED（優先方針）:** NagiのDesktop環境は可能な限り軽量にする。外観の洗練も重視し得るが、装飾のために不必要な常駐資源消費や操作遅延を増やさない方向とする。ミニマルでフラットな外観、DEC-020の8GB級での通常操作とローカルAI連携・前景優先、DEC-094の統合資源管理、DEC-108のOSS描画基盤と独自Shell、DEC-116の実測に基づく性能評価を維持する。AIが不在でも通常GUIを操作できること、必要なAccessibility・安全性・ユーザーの手動配置保護を軽量化の名目で損なわない。

**PROVISIONAL（具体策・測定候補）:** 背景ぼかし／半透明／影／常時アニメーション等の既定を抑える、静止画の壁紙、必要時だけ表示・更新するWidget／AIパネル、差分描画とイベント駆動、非表示Surfaceの更新抑制、必要時のモデルload／unload、端末能力に応じた効果の縮小等を比較する。低負荷時／前景アプリ利用時／ローカルLLM併用時のRAM、CPU／GPU、入力から表示までの遅延、フレーム時間、電力・熱、推論速度・品質を計測し、通常UIを優先する。QEMUと物理実機の結果を区別し、既存DEC-116の仮目標を新たな合格値に置換しない。具体効果の採否・描画backend（Wayland／Smithayを含む）・数値予算・性能保証・提供版は未確定。既存Gate／Owner／作業開始条件を変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-25 | 共通UIと独自デザインの自由を両立する提案をユーザーが了承し、ヘルシーな印象、ミニマルでフラットな外観を明示。具体配色・画面・技術・提供版はPROVISIONAL。 |
| 2026-11-25 | Desktopを可能な限り軽量にする優先方針をユーザーが明示。具体的な描画効果・backend・性能予算はPROVISIONAL。 |
