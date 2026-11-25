# DEC-103 — Testing / Release：段階的な検証Bを基盤に、自動化・AI支援Cを最終目標とする

**状態:** CONFIRMED（Foundation #34：Host→Linux Image／QEMU→実機の段階的・証拠に基づく検証Bを先に整備し、複数環境の自動テスト・障害注入・テスト不足分析・AIによる失敗原因調査等のCを将来の正式製品目標として段階導入）／PROVISIONAL（具体試験網・AIモデル／テスト生成・ツール・費用・スケジュール・提供版・Linux受入結果）
**最終合意日:** 2026-11-25（12/12 リリース・受入基準Bを追記。Foundation #34の合意は2026-11-16）
**12/12の状態:** CONFIRMED（段階公開、検証済みQEMU中心の0.1、証拠に基づく合否、人間による正式公開承認）／PROVISIONAL（詳細試験、対応環境、性能合格値、版別の重点・名称、公開日）
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

## 12/12 リリース・受入基準 B案（2026-11-25）

### CONFIRMED — 段階公開と公開判断

- **各版の目的に応じて機能・品質を確保し、段階的に公開する。** [DEC-123](DEC-123-kernel-unification-and-linux-0.1.md)のLinux版0.1 Developer Previewの公開範囲B（基本Desktop、Notes、Files、Settingsの英語・日本語、Calendar）と、ログイン・ユーザー管理のIdentity基盤を引き継ぐ。公開は検証済みのQEMU構成を中心に進める。実機の正式対応は[DEC-102](DEC-102-hardware-support-staged-multidevice-linux-drivers.md)に従い、機種・構成・機能ごとの検証で裏づける。未検証の実機は正式対応の対象外とする。
- **公開順と開発順を分ける。** Albert、ローカルAI、音声、意味検索、A/B更新・Recoveryなどは既存の0.2以降の範囲に従う。後続版の独立した作業・研究は、既に許可された範囲で依存関係と担当を確認して並行できる。[0.2実装仕様](../Nagi_OS_0.2_Codex_Implementation_Spec.md)のRelease Runtime ActivationはDEC-123に従い、配布物を作成・検証したLinux版0.1の公開、明示的な0.2 Release境界、対象WorkstreamのIntegration Owner checkpointを要する。0.2以降の製品範囲、既存のRelease／Activation／Acceptance GateとWorkstream Ownerを維持する。
- **合否は適用される受入条件と再現可能な動作証拠で判定する。** Host → Nagi Linux Image／QEMU → 正式対応を求める対象実機 → 実際に公開する成果物の確認、という順で、各段階の結果をその検証範囲に結び付ける。公開候補のソースRevision、構成、再現手順、結果と検証範囲を追跡できるようにする。AIコードレビューや原因分析は補助とし、合否は決定論的な試験の実行結果に基づく。開発イメージのPASSは、そのイメージで確認した範囲の結果として扱う。
- **正式公開の最終判断は人間が承認する。** データ消失、権限迂回、情報漏洩などの重大な不具合が未解決なら正式公開を止める。Developer Previewでも既存の認証・権限・データ保護の境界を守る。
- **1.0は[DEC-021](DEC-021-roadmap-0.5-1.0.md)の実用OS Bを維持する。** 正式対応範囲で日常・業務の代表作業を安全・安定して完結し、継続利用できる品質を求める。追加アプリの遅延だけを理由とする無期限延期は避け、宣言する提供範囲の実用性、安全性、安定性、復旧・継続と実機証拠で判断する。

### PROVISIONAL — 受入詳細と公開計画

- 0.1のアプリ別機能深度・詳細試験、L7〜L12の区切り・名称、配布イメージ・ハッシュ・ライセンス・起動手順・既知の制限など公開物の細目、正式対応とするQEMU構成・実機・対応環境、性能合格値、公開日。
- 版別の名称・重点、0.8を設けるか、1.0の代表E2Eの最終範囲。[0.3 Architecture Roadmap](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)、[0.5〜1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)で既存の対象範囲と暫定計画を参照する。
- 性能は[DEC-020](DEC-020-ai-os-interaction-path.md)の8GB級の実用性評価と[DEC-116](DEC-116-performance-targets-and-minimum-requirements.md)の仮目標、計測構成、基準実機・最低動作環境案を引き継ぐ。仮目標は実測に基づいて見直す作業目標として扱う。インストールするローカルLLMに応じた推奨スペックの案はDEC-116のPROVISIONALを参照する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | Foundation #34について、ユーザーが「Bを先に整備してCを最終目標にする」と明示確定。段階的検証Bと将来の自動化・AI支援C、決定論的な合否・証拠の正直さを記録。具体試験網・モデル・費用・時期・版・Linux受入は未確定。 |
| 2026-11-25 | 12/12のB案を承認。版ごとの段階公開、Linux版0.1の検証済みQEMU中心、再現可能な証拠と人間による公開承認、重大不具合による公開停止、1.0実用OS Bを記録。既存範囲・Gate・担当を維持。詳細試験・環境・数値・版別計画・日程はPROVISIONAL。 |
