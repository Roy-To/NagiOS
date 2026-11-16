# DEC-103 — Testing / Release：段階的な検証Bを基盤に、自動化・AI支援Cを最終目標とする

**状態:** CONFIRMED（Foundation #34：Host→Linux Image／QEMU→実機の段階的・証拠に基づく検証Bを先に整備し、複数環境の自動テスト・障害注入・テスト不足分析・AIによる失敗原因調査等のCを将来の正式製品目標として段階導入）／PROVISIONAL（具体試験網・AIモデル／テスト生成・ツール・費用・スケジュール・提供版・Linux受入結果）
**最終合意日:** 2026-11-16
**適用候補:** Nagi OS Foundation #34。既存Release／Activation／Acceptance Gateを変更しない
**関連:** [Foundation Checklist #34](NAGI_OS_FOUNDATION_CHECKLIST.md#f34)、[DEC-049（速度優先と証拠の正直さ）](DEC-049-development-process-speed-priority.md)、[DEC-044（Linux移行開発）](DEC-044-linux-migration-development-authorization.md)、[DEC-047（移行と統合）](DEC-047-linux-migration-workspaces-and-integration.md)、[DEC-098（統合診断とAI支援）](DEC-098-diagnostics-linux-nagi-ai-assisted-staged.md)、[DEC-102（段階的Hardware Support）](DEC-102-hardware-support-staged-multidevice-linux-drivers.md)、[0.1実装仕様](../Nagi_OS_0.1_Codex_Implementation_Spec.md)、[0.2実装仕様](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[Roadmaps](roadmaps/README.md)

## 確定した製品方針（CONFIRMED：Bを先行、Cが最終目標）

1. **Bの段階的・証拠に基づく検証を先に整える。** Hostでの単体・契約・Adapter試験から、実際のNagi Linux Image／QEMU内の統合・対象Runtime試験へ進み、Hardware固有の動作は対象実機で確認する。Host、旧Kernel Guest、Linux Adapter、Linux Image／QEMU、実機の結果と限界を区別する。Hostの成功をImage／実機の成功へ読み替えない。
2. **Cを将来の正式製品目標として段階導入する。** Bの再現可能な試験・証拠基盤の上に、複数環境の自動テスト、障害注入、テスト不足分析、AIによる失敗原因調査などを広げる。Cは単なる研究候補ではなく、将来的に製品開発・品質検証を支える正式な目標である。具体機能の実装順序・網羅範囲・版・納期は本決定では指定しない。
3. **AIの分析は原因仮説・補助とする。** 観測した事実、再現条件、実行済みの結果、欠けている証拠とAIの仮説を区別する。AIは追加試験や原因調査を支援できるが、説明の説得力・自己評価・予測だけで成功と認定しない。AIが生成した試験も実行・検証する必要があり、生成しただけでは成功証拠ではない。
4. **PASS／FAILは決定論的な試験結果と既存Acceptanceで評価する。** 未実行、失敗、部分成功、未測定、結果不明を正確に残す。単発CI成功、旧Guest成功、一般Linuxの起動、AIによる推測だけでNagi Linux全体やReleaseのPASSを主張しない。LLM不在でも試験実行・結果確認・合否評価を成立させる。
5. **既存Gateと速度優先の開発を維持する。** 0.1 M30、0.2 M21／Activation、0.3の仕様化・Activation／Acceptance、0.4のEntry Gate／Integration Owner checkpoint等を保持する。全35項目の完了やCの完成、全実機試験を一律の新Release／開発開始Gateにはしない。各段階で必要な試験は対象の既存Acceptanceと機能・環境に応じて扱う。

## 検証段階と証拠の限界

| 段階 | 検証する範囲 | この段階だけでは証明しないこと |
|---|---|---|
| Host／Adapter | 単体、契約、portable core、対象を明記したAdapterやtest doubleの動作 | 実際のLinux Image内Service接続、Guestの権限強制、実機Driver／電源の成功 |
| Nagi Linux Image／QEMU | 対象Image・設定でのBoot、Service連携、通常・拒否・障害時の対象Runtime動作 | 未試験Architecture・実機Firmware／Driver・無線・Sleep・性能の正式対応 |
| 対象実機 | 機種・構成・機能を限定した実動作、復旧・性能等の必要な受入 | 他機種・全機能・全Releaseの包括的な対応保証 |

この表は試験の責務を区別するもので、すべての変更を全段階で試験する義務や、新たな承認待ちを追加するものではない。具体シナリオ、失敗注入点、環境Matrixと必須・任意試験は後続設計で定める。

## 既存方針との整合・安全境界

- **DEC-049：** 開発速度を優先し、対象コードが変わらなければ環境・限界付きの過去結果を利用できる。Hostで進め、Linux Image受入は統合の節目にまとめられる。証拠はcommitと実行command・結果1〜3行および必要な公開可能参照で足り、段階別の重複台帳・承認手続きを増やさない。未実行の試験を成功としない、期待値決め打ちや偽実装で通さない制約を保持する。
- **DEC-044／047：** 承認済みLinux移行開発と正式Release／Activationを区別する。旧M30順序を移行開発の一律停止理由とせず、既存担当・統合責任と受入条件を保持する。
- **DEC-098／102：** 診断の事実とAI原因仮説、実機候補と正式対応を区別する。AIやテスト環境の存在だけで受入済みとしない。
- AI支援や障害注入で製品の非特権AI、Permission／Policy、情報区分、User／Profile隔離、データ保護を迂回しない。ログや試験データのAI利用・Cloud送信は既存の許可境界に従う。破壊的試験の対象・隔離・復旧方法は実際の試験設計で明確にする。

## 未確定事項（PROVISIONAL／OPEN）

- Linuxの最小統合シナリオ、正常・拒否・Negative／Security／障害・Recovery・性能試験の具体網、既存Acceptanceとの差分、対象環境・機種・必須／任意Matrixと合格値。
- 複数環境の自動実行・障害注入の方式、試験データと隔離、テスト不足分析の基準、AIモデル・Provider・Prompt／Context、テスト生成の採否・方式と生成物の検証。
- CI／Test Harness／Runner等のツール、費用・資源、実装担当、受入責任の具体割当て、スケジュール・導入段階・提供Version。
- Linux Image／QEMU・実機の実際の試験結果、Release受入。今回の文書記録では実装完了・Linux受入PASSを追加しない。

## Roadmapとの関係

0.1／0.2実装仕様と0.3／0.4+／0.5–1.0 Roadmapの既存Gate・版境界を維持する。B先行とCの正式な将来目標は確定するが、特定Versionへの必須割当て、Cの完成期限、全35項目完了という新Gateを作らない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | Foundation #34について、ユーザーが「Bを先に整備してCを最終目標にする」と明示確定。段階的検証Bと将来の自動化・AI支援C、決定論的な合否・証拠の正直さを記録。具体試験網・モデル・費用・時期・版・Linux受入は未確定。 |
