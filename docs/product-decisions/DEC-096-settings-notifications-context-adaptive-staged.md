# DEC-096 — Settings / Notification：Cの状況適応を最終目標に、Bの統合基盤を段階導入

**状態:** CONFIRMED（Foundation #27：作業状況・習慣等に応じて設定・通知を調整するC方式を最終的な製品目標とし、その実現段階でB方式のNagi統合Settings／Notification基盤を導入する）／PROVISIONAL（状況判定・学習と調整の詳細・初期既定値・技術方式・導入時期・適用版・Linux target受入）\
**最終合意日:** 2026-11-16\
**適用候補:** Nagi OS Foundation #27。製品方向の決定であり特定ReleaseのActivation／Acceptanceを変更しない\
**関連:** [Foundation Checklist #27](NAGI_OS_FOUNDATION_CHECKLIST.md#f27)、[DEC-008（控えめな働きかけ・重大警告）](DEC-008-proactive-behavior.md)、[DEC-020（決定論的な認可／非特権AI）](DEC-020-ai-os-interaction-path.md)、[DEC-004（習慣・明示設定優先・忘却）](DEC-004-memory-learning-forgetting.md)、[DEC-057（成長するPersonal Agent）](DEC-057-growing-personal-agent-os.md)、[DEC-058（Habit-first）](DEC-058-habit-first-learning-and-optional-teach-mode.md)、[0.2実装仕様 §12／§15／M11／M14](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[Roadmaps](roadmaps/README.md)

## 目的とユーザー体験

Nagi OSは、Settings画面と通知センターの単なる統合にとどまらず、ユーザーが何に取り組んでいるか、明示した好み、許可された習慣学習を考慮し、通知の表示方法・タイミングや扱える設定を適切に調整する**状況適応型の体験（C）**を最終目標とする。ただし、機能の土台にはユーザーが直接制御できる通常の設定・通知管理（B）が必要であり、Bを途中段階として導入することを明確に認める。

## 確定した製品方針（CONFIRMED：C最終目標、B段階導入）

1. **最終的な製品目標はC（高度な状況適応）。** User／Profile／App／Workspace／Task等の許可された文脈や習慣を踏まえ、不要な割り込みを減らし、必要な情報を適切な時点と方法で提示する。設定と通知をユーザーの使い方に合わせて洗練させること自体を製品目標とし、単なる研究候補に留めない。
2. **実装はB（共通の統合管理基盤）から段階的に進めてよい。** 型付きSettings ServiceとNotification Serviceを正規の基盤とし、Nagiの統合Settings UI、通知センター、手動Quiet／Focus設定、User／Profile／App別Scopeと表示ルールを統合する。基盤が安定した上で、状況に基づく提示方法・時機の調整、許可された習慣学習との連携へ段階的に発展させる。各段階のReleaseと順序の詳細は未確定。
3. **ユーザーの明示指示と制御を維持する。** Cの推定・学習よりも、ユーザーが固定・変更した設定、通知の許可・制限、Profile境界、明示的な「自動調整しない」指示を優先する。状況適応を停止・訂正できる手段を設け、AIの有無にかかわらずBの通常Settings／Notification操作が成立する方向とする。具体UI・設定項目は未確定。
4. **通知を勝手に消失・権限化しない。** 状況に応じた延期、集約、Quiet／Focus表示は、許可された規則の範囲で扱う。通知を無断削除したり、通知を見せたことだけでAction実行・権限拡大を認めたりしない。通知Actionの実行時は最新のCaller／Session／Permission／Policyを正規経路で再検証する。期限切れ・明示Dismiss等は既存Notification契約に従い、通常の遅延表示と混同しない。
5. **重大警告はDEC-008の安全条件を維持する。** 客観的なOSイベントを根拠とする重大警告は通常通知と区別し、必要な画面割り込みを許容する。AIの推測だけで重大度を確定したり、監視の信頼性が未確立のハードウェア／システム異常を重大警告に昇格させたりしない。重大警告の音声提示はユーザー許可がある場合に限る。Cによる状況適応がこれらの契約を弱めない。
6. **習慣学習との統合は既存Memory／Privacy規則に従う。** 繰り返し利用や訂正から許された通知・設定の好みを学び、提案と体験の改善に使う方向を維持するが、単発の動作だけで好みを断定しない。無制限の画面監視、秘密入力の収集、無断Cloud共有・履歴の永久保存は認めない。Memoryのモデル非依存性、訂正・保護・自然忘却・明示忘却を維持する。
7. **Settingsの責務とNotificationの責務を分ける。** Settingsはscope付き設定のSchema／Migration／永続化／検証・購読・書込み認可、Notificationはsource／priority／profile帰属／read状態／group／quiet／redaction等の通知管理を担う。Cの文脈判断やAIによる提案は、これらの正規Service／Policyを迂回して内容やPermissionを直接変更する独立権限を持たない。

## 段階的な製品体験（方向のみCONFIRMED、詳細PROVISIONAL）

- **段階B — 統合基盤:** Nagi Settings画面と通知センターで設定・通知を普通に管理できる。基本操作とPermission／PolicyはAIを介さず成立する。0.2のM11 Settings／M14 Notificationsの型付き契約とtarget受入を維持する。
- **段階C-1 — 状況への適応:** 許可された前景・Workspace／Task等の文脈や明示設定に応じて、通常通知の表示タイミング・方法・集約等を調整する体験を目指す。重大警告と明示的な通知の扱いを尊重し、実際の制御範囲は後続設計で定める。
- **段階C-2 — 習慣からの個人適応:** DEC-004／057／058の習慣学習・訂正可能性に接続し、利用者の好みに合わせた提案・通知の扱いを改善する。自動適用・ユーザー確認・明示同意が必要となる変更の分類・優先順位は後続判断とする。

これは段階の論理的な依存関係を表すものであり、3つの個別Releaseを制定したり、C実装時期・AI常時推論・全状況の自動切替を約束するものではない。

## 既存契約から継承する境界

- **0.2-M11 Settings:** 型付きKey、System／User／Profile／App Scope、Defaults、Validation、Migration／Version、原子的書込、読書き認可、購読・破損時回復。UIはServiceと別。
- **0.2-M14 Notification:** Profile別永続・read state、group／expiry、quiet／focus、敏感情報のredaction、通知ActionのActivation時認可。メール／SMS／Cloud Push等をNotification Coreに自動追加しない。
- **DEC-008:** 通常提案は作業を妨げない画面提示が基本であり、重大警告の客観的な根拠・音声許可・積極性設定を維持する。
- **DEC-020／001:** AIは非特権の提案者で、設定・通知Actionは正規Serviceと現行Permission／Policyに従う。学習や文脈から権限を推定しない。

## 未確定事項（PROVISIONAL／OPEN）

- CのContext signalと利用許可の範囲、作業中判定、誤判定・不在時動作、ルール優先順位、動的設定の対象とユーザー確認要件。
- 通知の重大度・優先順位・分類、通常通知の延期・集約・再表示・期限・取りこぼし防止・取り消し・復旧の詳細、Quiet／Focus／ロック画面のprivacy・redaction UI。
- 利用者による状況適応の表示・停止・訂正・透明性、手動ルールと学習ルールの競合、複数Profileでの状態分離・保存と移行。
- AI利用範囲とLLM非依存のルール、学習処理のCPU／RAM・バッテリー負担、必要な測定baseline、使いやすさ・誤抑止・重要通知漏れの受入基準。
- Linux service・IPCとNagi Settings／Notification Coreの結線、採用OSS／内部schema、具体Release・担当・Activation／Acceptance、実機検証。

## Roadmapとの整合

0.2のSettings／Notification基盤（M11／M14）をCの最終的な製品体験への足場とし、DEC-057の継続的成長とDEC-058のHabit-first、DEC-008の控えめな働きかけへ接続する。これらを正式な特定版への新規割当てやAcceptance Gateの変更として扱わず、Nagi OSの通常GUIとOffline-first、Desktop firstと既存の安全境界を維持する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #27で「Cを最終目標とし、途中段階でBの統合基盤を導入する」方針を明示承認。Bを単なる代替標準やCを単なる研究候補とせず、段階的な状況適応・習慣学習の製品目標をCONFIRMED。具体的な技術・自動調整範囲・導入版と実機受入はPROVISIONAL。 |
