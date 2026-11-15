# DEC-052 — Memoryを活用した個人最適化Nagiアプリ制作

**状態:** CONFIRMED（ローカルLLMを使う初心者向けNagi Native App制作と、許可された個人の反復傾向・嗜好を設計へ生かす製品方向）／PROVISIONAL（Memoryの選定・許可UI、動的適応、実装方式、提供時期）  
**最終合意日:** 2026-11-15  
**適用候補:** Nagi純正のアプリ制作・改善体験。Release割当てと実装時期は未確定  
**関連:** [DEC-002（Cloud AI・外部共有）](DEC-002-cloud-ai-data-sharing.md)、[DEC-003（Memory分類・所有範囲）](DEC-003-memory-classification.md)、[DEC-004（Memory学習・忘却）](DEC-004-memory-learning-forgetting.md)、[DEC-010（Intent-first UI）](DEC-010-intent-first-adaptive-ui.md)、[DEC-015（builder Role）](DEC-015-builder-role-review.md)、[DEC-017（アプリ開発環境）](DEC-017-third-party-developer-ecosystem.md)、[DEC-018（隔離編集）](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-035（導入後の保守・更新）](DEC-035-builder-created-app-maintenance-updates.md)、[DEC-036（新規アプリの試用・導入）](DEC-036-builder-app-trial-and-installation.md)、[DEC-036（共有・公開）](DEC-036-builder-app-sharing-and-publication.md)、[DEC-040（開発Project保持）](DEC-040-builder-app-development-project-retention.md)、[DEC-040（Action API）](DEC-040-builder-generated-action-api-integration.md)、[VISION-001](VISION-001-human-adaptive-os.md)、[0.3改訂Roadmap](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)、[0.5–1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

## 目的とユーザー体験

Nagiの純正AIアプリ制作環境は、プログラミングに詳しくない人でも、日常で使う小規模なNagi Native Appを自然言語とローカルLLMで作り、利用・配布できる方向を目指す。既存の移植支援・AIコーディング支援の拡張として位置付ける。開発実行には、DEC-015で確定した `builder` Roleを用いる。`Nagi Studio` は会話で使われた仮称であり、製品名や独立IDEの決定ではない。

目指す体験は、コードを生成するだけでなく、利用者それぞれの操作傾向や嗜好を、本人の管理下にあるMemoryと許可の範囲でアプリの機能・画面設計に生かすことにある。同じ用途のアプリでも、使う人により使いやすい設計は異なりうる。人間の感性と選択を尊重することをNagiの開発体験の差別化方針とする。

## Nagiの差別化の狙い

「自然言語からコードを作る」だけでなく、ローカルLLMを使うNagi Native App制作、本人が許可したMemoryによる個人ごとの機能・UI設計、作ったアプリをNagi上で試用・利用・共有する体験をつなげることを狙う。これは製品の位置付けであり、ChromeOS等の他製品との具体的な機能比較や市場優位性を実証した主張ではない。

## 確定した仕様

1. **初心者向けのNagi Native App制作を目指す。** 利用者が日常で使う小規模なアプリを自然言語で作れる体験をNagi純正環境の製品方向に含める。ローカルLLMを利用できることを目標とし、特定モデル、性能、対応範囲は別途決める。大規模な業務・開発環境全般を短時間で生成できる保証ではない。
2. **個人の利用傾向をアプリ設計に生かす。** 現在のPolicy／Permissionで利用が認められた同一User／ProfileのMemoryを、アプリの設計・改善に使える方向を目指す。対象には、繰り返し観測され利用者が採用した操作傾向、明示的な嗜好、UIの好みを含みうる。単発操作だけを定着した習慣と見なさず、明示的な指定は推定より優先する（DEC-003／004）。
3. **同じアプリでも利用者ごとの違いを認める。** 例えば電卓では、小数を頻繁に入力する人向けに小数点入力を見つけやすくする案と、指数・科学表記をよく使う人向けに指数入力を前面に出す案は異なりうる。Nagiは一律の標準UIだけを前提にせず、許可されたMemoryと本人の選択に基づいて設計案を変えられることを目指す。
4. **通常のアプリ利用とOS境界を維持する。** 生成アプリはNagi Native App契約、Public API、Permission／Capability／Policyに従い、通常のGUIから利用できる。AIによる設計・コード生成は、Memory参照権限、Action実行権限、アプリ導入・更新権限を生まない（DEC-017／027）。
5. **作成・導入・更新・配布を別の操作として扱う。** 開発は承認されたScope内の隔離Projectで行う。初回導入前の試用・採用判断・正式導入、導入済みアプリの正式更新、第三者への共有・公開は、それぞれDEC-036（試用・導入）、DEC-035、DEC-036（共有・公開）の独立した手続きに従う。Memoryを使って作ったという理由で既存アプリを無断変更せず、配布物に個人Memoryを含めない。

## ユーザー体験の提案（具体的UIはPROVISIONAL）

- 利用者が「普段使いの電卓を作って」と依頼したとき、Nagiは現在許可されているProfile Memoryに適切な傾向があれば、その根拠を短く示して個人向け設計案を提案する。Memoryが利用できない、許可されていない、または傾向が不明な場合は、Memoryから推測せず、利用者の指定だけで設計する。
- 小数入力が繰り返し使われている例では、「小数点キーを押しやすくする」「小数の表示精度を選びやすくする」といった案を提示する。指数・科学表記をよく使う例では、「指数入力や科学表記への入口を見つけやすくする」案を提示する。これは候補であり、特定の操作履歴から必ずこのUIを選ぶという決定ではない。
- 利用者は案を採用・修正・却下できる。builderは選ばれた要件をもとに隔離Projectで制作し、確認できた動作・テストと未検証点を示す。初回導入前の試用と正式インストールにはDEC-036の別承認を要する。後から設計改善が望ましい場合も提案し、正式更新にはDEC-035の確認・承認を要する。

## 例外・禁止・安全境界

- **このDECはMemoryへの包括許可を作らない。** Memoryの参照・検索・保存・Cloud送信・Team共有の権限は別々である。利用時には既存のUser／Profile所有者、分類、Purpose、Task Scope、現行Permission／Policyを適用し、不明・競合時は推測で取得・共有しない（DEC-002／003）。
- 個人MemoryをAgent、Team、Organization、開発Projectへ暗黙に昇格しない。開発Projectはアプリのソース・設定・検証情報を扱う別の保護対象であり、Canonical Memoryの保存場所ではない（DEC-003／040）。
- Memoryの原文、個人の事実、Memory ID、認証情報、Permission／Approvalをアプリのソース、実行時データ、配布Packageへコピー・継承させない。本人が選んだ設計をアプリのUIに反映することと、元のMemoryやアクセス権をアプリへ持ち込むことは別である。共有・公開時はDEC-036の内容確認と個人Memory等の分離を行う。
- Memoryを使った生成は、アプリが実行中にCanonical Memoryへ常時アクセスすること、アプリが新たな使用履歴を収集・保存することを許可しない。そうした機能は別の目的・権限・製品判断を要する。
- 既に導入されたアプリをMemoryの変化に応じて自動的に書き換えない。改善は承認済みScope内の隔離編集として提案・検証し、正式適用にはDEC-035の明示承認を得る。
- 本決定は既存の0.1／0.2／0.3 Acceptance、0.4 Entry Gate／Integration Owner checkpoint、Activation、Workstream、DEC-017の段階的ロードマップを変更せず、実装開始・Release commitmentを意味しない。

## 未確定・暫定事項

- Memoryをアプリ制作時だけ参照するか、導入後の改善提案やアプリ実行時にも用いるか。実行時Memory連携を将来採用する場合のアプリ別Permission、Purpose、可視化、失効方式。
- 対象とするMemoryカテゴリ、反復・採用・信頼性の判定、競合する嗜好の扱い、候補を提示する条件とユーザーへの説明・修正・取消UI。
- Taskごとの許可、Project単位の設定、事前に設定されたProfile Scope等、Memory参照の認可・同意・監査UI。既存Permissionを前提とし、具体方式は未決定。
- UIを生成時に固定するか、複数案から選択するか、利用中に動的変更するか。動的再配置、表示要素の優先度変更、手動調整の保持、プレビューと復元の方式。
- 開発画面をNagi Home／Desktop内のbuilder体験にするか、独立したIDE／アプリにするか。「Nagi Studio」の名称、画面構成、対応言語・Framework、生成・Build・Test・Package方式。
- ローカルLLMのモデル選択、必要Hardware、性能・品質指標、外部Providerを使う場合の扱い（DEC-002に従う）。
- Memory由来の設計を開発Projectにどう記録するか。ProjectはCanonical Memoryと分離し、個人Memory自体を格納しない原則を維持する。
- 配布形式・対象・経路、機能別Acceptance、担当Workstream、実装・提供時期。既存のDEC-017が定める1.0開発環境目標への詳細な割当ても未確定。

## Roadmapとの関連

- **VISION-001／DEC-010:** 人間に合わせるコンピューター、Intent-first UI、学習した好みを尊重する方向を補う。Workspace全体のレイアウト提案と、生成アプリ内部のUI変更は別の対象であり、本決定は後者の動的再配置を確定しない。
- **0.3改訂Roadmap:** Profile、Memory、Preference／Habit、Context、Permission境界の将来基盤と整合する。0.3でこのアプリ制作機能を実装・有効化する決定ではない。
- **0.4+ Roadmap／DEC-015／017:** 初期builder Role、Nagi Native Apps、AI codingに最適化する既存方針を拡張する。新しいAgent、IDE、0.4の必須Acceptance、版別の実装順序を追加しない。
- **DEC-035／036／040:** 開発Projectの保持、初回導入、導入後更新、共有・公開の境界を維持する。個人Memoryの参照やアプリの個人化は、インストール・更新・共有の承認を代替しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-15 | ユーザーが、ローカルLLMで初心者も日常的なNagi Native Appを作成・配布し、許可されたMemory上の反復する操作傾向・嗜好・UI好みをアプリの個人最適化に生かす高水準の製品方向を確認した。電卓の小数入力重視と指数入力重視を例とした。Memoryの選定・権限UI、動的再配置、実装方式、提供時期はPROVISIONALとし、既存のプライバシー・導入・更新・共有承認境界を維持する。 |

---

**End of Decision**
