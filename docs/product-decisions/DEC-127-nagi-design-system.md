# DEC-127 — Nagi Design System：ミニマルな共通UIと創作の自由

**状態:** CONFIRMED（ミニマルでフラットなデザイン、共通UIによる開発支援と独自デザインの自由の両立、Desktopの軽量性を重視、統合OSの中核としての端末横断Desktop体験、端末横断ShellのDesktop first設計）／PROVISIONAL（具体配色・部品・外観・API・技術・提供版）
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

## 2026-11-25 — 統合OSの中核としての端末横断Desktop／Convergence

**CONFIRMED（製品方向）:** PC・Tablet・Phone間で可能な限りシームレスな一つのNagi体験を提供し、Desktop／Shellの設計を統合OSの中核として重視する。Samsung DeX・PixelのDesktop mode・Lomiriは利用者が示した体験の参考例であり、同等の機能・互換性・特定Shellやコードの採用決定ではない。DEC-009／010／108／109の共通Home／Desktop・Workspace・入力／表示の端末適応、DEC-065の意味的Handoffを継承する。PC画面の単純縮小や端末ごとに分断された体験ではなく、画面・入力・接続機器・端末能力に合わせて操作UIを適応させる方向とする。ミニマル・フラットな外観と軽量性、AIなしの通常GUI、前景応答優先を維持する。

**PROVISIONAL（体験・設計候補）:** 同一端末の小画面／Tablet／Desktop向けSurfaceと、外部モニター・キーボード・マウス接続時のDesktop相当体験、手動切替・接続時の案内・必要な状態保持、切断時の可視画面への退避、独立表示／拡張／ミラーリング等を比較する。アプリの共通データモデル・Public Action／Continuity契約と表示レイアウトを分離し、画面変更で編集内容・作業の意味的状態を可能な範囲で保持する。別端末への継続（Handoff）と同一端末の表示モード変更を区別する。DEC-014／065／066に従い、現行権限・Revision・Profile・Privacyを再検証し、無断同期・転送・競合上書き・Task再実行は認めない。Home／Desktop遷移、Workspace手動配置保護、既存の新規Workspace自動配置・画面切断時の例外を維持する。具体的なSurface分類・外部表示・UI・切替条件、Compositor／Wayland／Smithay等の汎用基盤の採否、対応機器・性能・実装版は未確定。Desktop first、DEC-102の段階的な機種別実機検証、既存Release／Activation／Acceptance Gate・Owner・着手条件を変更しない。

## 2026-11-25 — 既存Desktop環境を製品基盤として採用しない（明確化）

**CONFIRMED:** Lomiriを完成済みDesktop環境／Shellとしてそのまま採用したり、それをNagiの製品基盤として改造・流用したりする方針ではない。Samsung DeX／Pixel Desktop mode／Lomiriは端末横断の操作体験・設計思想の参考例に限定する。NagiのShell／Home／Desktop／Workspace／ウィンドウ管理・操作体系はNagi主体で独自設計する。これはDEC-043／108の既存方針の再確認である。Wayland／Smithay等の汎用基盤の選択的再利用は別の技術判断であり、採用は引き続きPROVISIONAL。既存Gate・着手条件は変更しない。

## 2026-11-25 — 端末横断ShellのDesktop first設計

**CONFIRMED（基本設計・開発方向）:** PC専用のShellを完成させてからモバイル用に別設計するのではなく、初めから端末横断のNagi独自Shell（Nagi Shellは仮称）を設計し、そのDesktop向け表示から実装する方針を採用する。既存の純正アプリ仕様 §27–30／44のDevice・Surface・Device Capability・Continuity・device-neutral Coreを再利用し、画面の寸法・DPI・向き・入力方式・利用可能な外部画面等に応じた適応とユーザーの手動選択を両立する。Nagi Barを端末横断の共通操作窓口とする方向、Home・アプリ・Workspace・Nagiへの指示・Task状態への到達性を重視する。Workspaceの作業内容・意味的な状態と表示モード／配置を分離し、同一端末の画面・接続変化に対する適応と、別端末へのHandoffを区別する。通常のウィンドウ管理・画面切替・入力・描画はLLM推論に依存させず、自然言語の意図解釈・必要な判断／配置提案等だけを許可されたContext・Public Action経由で連携する。AI不在時の通常GUIと前景応答性を維持する。既存Desktop環境を製品基盤として採用しない。

**PROVISIONAL（具体設計・順序候補）:**

- Shell側のCompact／Touch／Desktop／Presentation等の表示モードは候補。アプリのSurface（desktop／tablet／mobile／external-display／compact-overlay等）とWorkspaceのレイアウト方式（Floating／Tiling／Split／Focus等）を同一の列挙型に固定せず、既存契約との対応・適用単位・命名・組合せ・初期値を検証する。自動適応と手動切替の具体条件は未確定。手動選択と固定配置を尊重し、既存Workspaceの大幅な無断再配置を認めない。外部画面が切断された場合の可視画面への退避・再接続はDEC-010に従う。
- Nagi Barの具体的位置・形状・展開方式、Launcher／Task／Workspaceの導線、モバイルの操作方式、同一端末の外部モニター接続でのDesktop相当表示、端末本体と外部画面の役割分担等は候補。DEC-009のログイン後Home、直接App起動時Desktop遷移、委任Task時の原則Home維持と双方向切替を変更しない。画面変更・復元だけで中断Taskを再実行しない。
- Window Manager／Workspace Manager／Surface Manager／Shell UI／Compositorなどの責務分割は設計候補であり、プロセス構成・API／IPC・Wayland／Smithay等の採否は未確定。LLMは非特権、現行Permission／Capability／Policyと構造化Context・Public Actionを維持する。外部画面・共有時の秘密情報保護、端末間移行はDEC-014／065／066等の既存境界に従う。
- 段階案: (1) 軽量Desktop・基本ウィンドウ管理、(2) 共通Surface／Workspace／Nagi Bar、(3) ローカルLLM連携・目的別配置、(4) Touch／Tablet／Compact、(5) 外部ディスプレイ・端末横断実機検証。共通契約・device-neutral Coreは最初から考慮する。段階は提供順の案であり、独立した許可済み準備・試験を禁止しない。既存Release／Activation／Acceptance Gate、Workstream owner・開始条件、0.1／0.2等の既定範囲は変更しない。具体提供版、対応機器、測定値、実装の成功は未確定・未実証。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-25 | 共通UIと独自デザインの自由を両立する提案をユーザーが了承し、ヘルシーな印象、ミニマルでフラットな外観を明示。具体配色・画面・技術・提供版はPROVISIONAL。 |
| 2026-11-25 | Desktopを可能な限り軽量にする優先方針をユーザーが明示。具体的な描画効果・backend・性能予算はPROVISIONAL。 |
| 2026-11-25 | PC／Tablet／Phoneのシームレスな体験と統合OSの中核としてDesktopを重視する方向をユーザーが明示。DeX／Pixel Desktop mode／Lomiriは体験の参照例、具体技術・切替仕様・端末・版はPROVISIONAL。 |
| 2026-11-25 | ユーザーがLomiriのそのままの採用を否定。既存Desktop環境の製品基盤としての採用ではなく体験の参照に限定し、汎用基盤の採否とは区別する旨を明確化。 |
| 2026-11-25 | ユーザーが端末横断Shellを初めから設計しDesktop向け表示から実装する方針、共通Nagi Bar・作業状態と表示の分離・通常UIの非LLM依存を承認。具体モード・UI・技術・段階割当てはPROVISIONAL。 |
