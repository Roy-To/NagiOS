# DEC-097 — 通常のアクセシビリティ基盤の先行整備とAI-native操作の長期方向

**状態:** CONFIRMED（Foundation #28：通常のアクセシビリティ基盤を先に整え、将来のAI-nativeで正確なポインティングに依存しない操作体系と矛盾しない設計にする）／PROVISIONAL（機能範囲・具体技術・提供時期・適用版・受入条件、time／localeの詳細方式）  
**最終合意日:** 2026-11-16  
**適用候補:** Nagi OS Foundation #28。近い将来の基盤整備方針と遠い将来の操作思想を区別し、特定Releaseへ割り当てない  
**関連:** [Foundation Checklist #28](NAGI_OS_FOUNDATION_CHECKLIST.md#f28)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-010](DEC-010-intent-first-adaptive-ui.md)、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)、[VISION-001](VISION-001-human-adaptive-os.md)、[言語アーキテクチャ](../architecture/language-architecture.md)、[Roadmaps](roadmaps/README.md)

## 目的とユーザー体験

まず、キーボードによる操作・ナビゲーション、screen reader等を含む一般的なアクセシビリティに向けた基礎を整える。その設計は、長期的にAIが利用者の意図や文脈を補い、正確なポインティングに依存せず操作できるOSを目指す方向と矛盾しないものにする。通常のa11y整備と遠い将来のAI-native操作思想を別々の段階として扱う。

## 確定した製品方針（CONFIRMED）

1. **通常のアクセシビリティ基盤を先に整備する。** keyboard／navigation、screen-reader等を含む一般的a11yに向けた基礎を優先する。長期ビジョンの実現やAI利用を、その整備・通常操作の前提にしない。
2. **将来の操作思想と両立する設計にする。** AI-nativeで正確なポインティングに依存しない操作体系という長期方向を別に維持し、当面のa11y基盤がその発展と矛盾しないようにする。この両立方針は確定だが、将来の具体的な入力方式・製品機能の採用確定とは区別する。
3. **既存の直接操作を維持する。** GUI、keyboard、mouse等の操作を維持し、AIが不在・無効・利用不能な場合にも通常のOSとして操作できる。AIとの会話や入力推定を唯一の操作経路にしない。
4. **AIの補助と認可・確認を分ける。** 将来AIが曖昧な入力や指し示しを補う場合も、Permission／Policyと必要なconfirmationを維持する。AIの推定・確信度、a11y情報への到達可能性は、実行許可・権限拡大・確認省略の根拠にならない。

## 近い将来の基盤と遠い将来のビジョン

| 区分 | 確認した方向 | 未確定の範囲 |
|---|---|---|
| 通常のa11y基盤 | keyboard／navigation、screen-reader等の一般的a11yに向けた基礎を先行整備 | 最初の対象画面、機能の完全な一覧、tree／focus／読み上げの契約、採用技術、受入基準・提供版 |
| AI-nativeな操作思想 | 正確なポインティングに依存しない長期方向を維持し、当面の設計と両立させる | 曖昧さの判定・訂正UI、利用Context、具体入力技術、性能・対応端末・搭載時期 |
| Time／Locale | 既存の言語アーキテクチャを維持 | time authority、monotonic／wall clock／RTC、同期・offline時刻ずれ、timezone等の詳細方式 |

視線、ジェスチャー、空間入力等は将来の可能性を示す例に留め、今回の採用決定に含めない。特定のセンサー、モデル、protocol、a11y backend、screen reader製品も選定しない。

## 既存決定・安全境界との整合

- **DEC-009／010:** Nagi-first／Intent-first、従来GUIとAI不在時の通常操作を継承する。両文書のa11y細部の未確定記述は、本決定の先行整備方針と両立する。
- **DEC-027:** 人が使うa11y基盤の整備は、AIに無制限のGUI自動操作を認めるものではない。正規の型付きPublic Actionを優先し、GUI／Accessibility fallbackは既存の明示許可・限定Scope・検証・停止と追加承認の条件を維持する。
- **VISION-001:** 人間に合わせるOS、不確かな入力や指し示しの補助という長期方向を維持する。通常a11y基盤の先行整備を、そのビジョンの実装完了や特定入力技術の採用と同一視しない。
- **Time／Locale:** 英語の内部識別子、en-US／ja-JPの同格、system language／region・locale／input language・keyboard／AI会話言語の分離等の既存契約を維持する。今回、time／localeの詳細方式を新たに確定しない。

## 未確定事項（PROVISIONAL／OPEN）

- a11y treeとUIの意味情報、focusとkeyboard navigation、読み上げ連携の具体契約、最初に対応する画面・アプリ、動的UI・通知・確認画面の扱い。
- 支援技術からの参照・操作の権限、秘密入力やProfile境界、許可取消・AI不在時・誤解釈時の具体動作と検証方法。
- 長期AI補助に使う入力・Context、誤解の訂正やconfirmationのUI、視線／ジェスチャー／空間入力等の採否。
- time authority、時刻同期、offline時刻ずれ、timezone保存・変更、期限認可と時計の巻戻り等の詳細方式。既存の認可要件を弱めない。
- 各段階の実装技術、対応機器、提供時期、適用版、担当、Linux target／実機の受入方法・合格値。

## Roadmapとの関連

0.2の通常UI・言語・Public Action基盤、0.3のPersonal Agent／Intent-first方向、0.5–1.0案のoffline／accessibilityテーマに接続する。先行整備は相対的な優先方針であり、0.2等への新規必須機能割当てや長期AI入力の提供時期を意味しない。既存Release／Activation／Acceptance Gate、Desktop first、実装担当を変更しない。仕様記録であり、a11y機能の実装・screen reader動作・Linux実機受入の完了を主張しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | Foundation #28について、通常a11y基盤の先行整備と、将来のAI-nativeで正確なポインティングに依存しない操作思想と両立する設計をユーザーが明示決定。短期と長期を分離し、通常GUI・AI不在時の操作・Permission／Policy／confirmationを維持。具体入力技術・搭載時期とtime／locale詳細は未決定。 |
