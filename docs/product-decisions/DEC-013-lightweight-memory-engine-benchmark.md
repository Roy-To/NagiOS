# DEC-013 — Hardware-Adaptive Memory Engineの先行設計と性能評価

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（適応方針・評価方針）／PROVISIONAL（profile・技術選択・閾値）  
**最終合意日:** 2026-11-05  
**対象:** 0.1中の先行設計・ホスト専用PoC、0.2 Forward Compatibility、0.3での正式Memory Engine実装  
**関連:** DEC-003、DEC-004、DEC-005、DEC-007、`roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md`、`benchmarks/MEMORY-BENCH-01.md`

## 目的とユーザー体験

Nagi 0.3のMemory Engineは、特定世代の低スペック端末向けに固定最適化するのではなく、端末の能力と実測状態に合わせて検索・Context構築・推論経路を選べるHardware-Adaptiveな設計とする。ユーザー所有のモデル非依存Canonical Memoryを再学習なしで保ち、端末に依存する派生物だけを再生成できるようにする。検索品質・前景応答性・RAM・容量・消費電力を評価し、機種名だけで方式を決めない。

## 確定した方針

1. **独立した重要設計テーマ:** 限られた資源を含む各種Hardwareで効率よく動く、LLM非依存の軽量Memory EngineをNagi OS 0.3の主要な技術課題として扱う。既存0.3候補の `MEMORY-01`、`MEMORY-02`、`LEARN-01`、`CONTEXT-01` に接続する。実装Workstream登録・採用技術は後日正式判断する。
2. **先行検証:** 0.3本格実装前に、検索・記憶統合/圧縮・学習/忘却・Context選択の候補を設計し、再現可能な合成データによる比較・評価方法を整える。独立したhost-only PoCは本番実装やTarget Acceptanceではない。
3. **比較候補:** (A) 構造化Memory＋metadata/全文検索、(B) A＋重複統合・重要度/習慣評価・予算内Context選択、(C) B＋必要時のみEmbedding/Hybrid検索。CはEmbedding不在でもA/B系へ退避可能であるべき。これらは**比較する候補**であって採用済み方式ではない。
4. **容量・応答性の評価:** 1,000 / 10,000 / 100,000件の合成記憶を使い、索引サイズ・ストレージ占有・RAM・cold/warm検索・Context構築・品質を同じ条件で計測する。前景操作への影響とアイドル時の整理負荷も確認する。
5. **記憶の品質と安全性:** 関連記憶の検索成功、ユーザーの明示指定優先、正確なSource/由来、Scope/Permissionによる結果除外、明示的忘却後の再学習抑止、Profile分離、再起動・モデル切替後の一貫性を評価する。
6. **既存ルール維持:** DEC-003（機密性・所有範囲）、DEC-004（Active→Decay→Archive→Delete、保護・忘却）、DEC-005（Files/AlbertとSource削除連動）は本決定より優先して維持する。メモリ圧縮・要約によってprovenanceや機密性を失わせない。
7. **段階的な開発境界:** 0.1中は技術設計と本体非侵襲のホストベンチ準備、0.2ではIdentity・Search・Storage・Jobs・Model Runtime・Permission・IPCの将来互換性を確認する。正式0.3 runtime実装は既存のrelease/activation gateとIntegration Owner境界を越えない。
8. **Canonical Memoryと派生物の分離:** User/Profileの好み・習慣・provenance・権限と忘却の状態・プロフィールを、ユーザー所有のモデル非依存Canonical Memory/Stateとして維持する。権限は学習から推論・昇格せず、検索時には現行の認可元から独立に再評価する。検索索引、キャッシュ、Context Adapter、推論方式などHardware依存の派生物は再生成可能とし、機種更新時も記憶を再学習しない。
9. **Hardware-Adaptiveな処理選択:** CPU、RAM、GPU、NPU、ストレージ、電力条件、現在の負荷とベンチ測定を踏まえ、低負荷のlexical検索からoptional embedding/hybrid検索、高度なrerankingまで選択肢を広げる。端末能力が限られる場合、資源逼迫時、または前景操作を妨げる場合は安全なLite経路へfallbackし、余裕がある時に高度な処理を使う。性能向上を常時稼働処理やbaseline overheadの増大には結び付けず、前景応答性を優先する。
10. **世代間移行と評価:** 機種名ではなく、体感応答性、検索精度、RAM、ストレージ容量、消費電力などを測定して比較する。Hardware世代をまたぐMemory移行と派生indexの再構築を試験し、Canonical Memory、provenance、現行権限、明示的忘却が変わらないことを確認する。
11. **段階名は未確定:** Lite / Standard / Enhancedのような機能段階は検討対象とするが、名称、段階数、固定profile化は決定していない。


## 2026-11-05 — 記憶の永続性と適応型検索方針

**上位方針 — CONFIRMED:** 「記憶は永続、検索能力はハードウェアに合わせて進化する」。ここでの永続性はCanonical Memoryの所有・継続性を指し、DEC-004の自然忘却・明示的忘却・削除規則を無効にするものではない。既存のCanonical Memory、Profile、provenance、権限、保護・忘却状態を保ち、Hardware変更時にユーザーの記憶や習慣を再学習させない。必要なHardware依存index、cache、Context adapterのみを再生成する。

**0.3の検索選択方針 — CONFIRMED:** Memory Engineは質問の性質とCPU、RAM、利用可能なGPU/NPU、ストレージ、電力条件、現在の負荷および実測性能を考慮し、必要十分な処理能力を選択する。通常の単純な問い合わせは高性能端末でも軽量なlexical経路を優先する。検索候補には全文・N-gram等の低負荷検索、任意のローカルEmbedding/Hybrid検索、必要な場合の高度なrerankingを含められる。高負荷処理や索引整備は資源と前景応答性に余裕がある場合に行い、負荷逼迫・機能不在・失敗時は安全なlexical経路へ退避できる。

**権限・Context境界 — CONFIRMED:** 検索前に、現在のPrincipal/Profile/Scope、Permission、Source ACL・機密分類、明示的忘却とSource無効化を適用し、アクセス可能な候補だけを検索対象にする。検索後は許可された候補から関連性を選び、Context予算内に収める。この選択・最適化はDEC-003/004/005を変更せず、検索品質や高速化を理由に権限・忘却・provenanceを弱めない。

**構成可能性 — CONFIRMED:** 固定3段階のLite / Standard / Enhancedを基本設計にしない。能力ごとの有効化・組合せと状況に応じた選択を基本案とする。各能力の境界やUI/APIは別途決める。

**実装詳細 — PROVISIONAL:** 仮称「Memory Resource Governor」、命名、API、CPU/GPU/NPU・負荷・電力等の測定方法と選択規則、各処理の有効化条件、切替待機時間、移動平均・ヒステリシス、閾値、再索引スケジュール、Context budget、合格数値は未決定。これらはImplementation Specと実測評価で定め、今回の合意から数値・実装を推定しない。

**適用境界:** 本決定は製品設計方針であり、Memory本体やRuntimeコードの実装を開始しない。0.1 M30、0.2、0.3の既存activation gatesとIntegration Owner境界を維持する。Private仕様・RoadmapをPublicリポジトリへ転載しない。

## ベンチマークの扱い

詳細なテストマトリクス、計測指標、成功/失敗の報告様式は [MEMORY-BENCH-01](benchmarks/MEMORY-BENCH-01.md) とする。ホストPoCでPASSしたことをNagi target runtime PASSと呼ばない。合成データの検索精度は実際の日本語・英語や長期使用時のユーザー体験を保証しない。

## 例外・禁止・安全境界

- モデル固有のKV Cache、推論Prompt全文、学習済み重みを永続Memoryの正本にしない。
- 候補方式が高スコアを得ても、ユーザーによる明示指定、機密分類、Source ACL、現行Permission、明示的忘却を上書きしない。
- 閲覧履歴・Activity・Waybackの権限をMemoryが自動取得しない。原本が削除/ゴミ箱へ移動された場合はDEC-005に従う。忘却済み情報は旧索引/Backup/Waybackから黙って再学習しない。
- 業務/私的な実データをベンチの既定fixtureにしない。外部クラウド送信・外部Embeddingサービスはベンチに必須としない。
- 性能向上のためにKernelへMemory policyを実装しない。AI無効時もOSの基本機能を使える状態を維持する。
- PublicリポジトリへPrivate仕様・未公開ロードマップを転載しない。公開可能な実装契約が必要な場合は別途情報を整理して公開判断する。

## 未確定・PROVISIONAL

- SQLite/FTS5・別のFTS・独自Storeを含む物理DB、索引方式、ベクトルindex、Embedding model、Tokenizer、量子化、圧縮アルゴリズムの採否。
- 重み付け関数と各係数、Decay/Archive/Delete閾値、コンパクション間隔、キャッシュ容量、検索候補数、コンテキストToken budget、品質・速度・RAMの合格数値。
- Memoryの論理層数・構造（4層構造案を含む）の採否。
- Lite / Standard / Enhanced等のprofile名称・段階数、各段階の検索・索引・モデル構成、resource signalの重み、automatic switching/fallback algorithmと数値threshold。
- 日本語と英語の形態素処理、同義語・言い換えによる検索品質、アプリ/Sourceごとの更新・削除・再索引方式。
- Raspberry Pi 5、ARM64等の物理HWでの達成性能・電力測定条件。0.2参照環境QEMU x86-64 4vCPU/8GB RAMと実機測定は混同しない。

## Roadmapとの関連

- **0.1:** 仕様検討と隔離されたhost-only benchmark準備。0.1 M30 Acceptanceを変更しない。
- **0.2:** Search/Identity/IPC/Settings/Jobs/Storage/Model RuntimeのForward Compatibility確認。既存Gateを維持。
- **0.3:** 正式なImplementation Specとactivation後、MEMORY-01 → MEMORY-02 / LEARN-01 → CONTEXT-01等を依存関係に従い実装・統合する。Canonical MemoryとHardware依存の再生成可能な派生物を分離し、世代間移行とindex再構築を含めてHardware-Adaptive動作を評価する。Workstream候補の具体的な順序は正式版で決定する。

> **2026-11-16 Data Persistence後続決定 — Canonical Memoryと派生Index／Cache:** [DEC-090](DEC-090-common-persistence-contract-distributed-authority.md)で、モデル非依存Canonical Memoryの所有・忘却／由来・Profile境界を保存正本として守り、派生Index／Cacheを再生成できる構造を維持する。今回の決定は共通永続化契約＋用途別分散正本であり、Memory Engineの具体DB／schema・暗号・保持数値・正式0.3実装開始を追加確定しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-04 | 軽量Memory Engineを0.3の独立した重要テーマとし、事前のアルゴリズム候補比較・ベンチマークを進めることに合意。候補の採用、実測値・閾値は未決定。 |
| 2026-11-04 | 0.3 Memory EngineをHardware-Adaptiveとし、モデル非依存Canonical Memoryと再生成可能なHardware依存派生物を分離する方針を合意。移行・index再構築と実測評価を加える。段階名称・数、方式、切替条件・数値閾値はPROVISIONALのまま。 |
| 2026-11-05 | 「記憶は永続、検索能力はハードウェアに合わせて進化する」を上位方針として合意。能力別に組合せ可能な適応型検索、権限先行フィルタとContext予算を確認。Governor命名・API・切替アルゴリズム等はPROVISIONAL、既存Gate維持。 |


**詳細設計案:** [Memory Resource Governor — detailed design proposal](MEMORY-RESOURCE-GOVERNOR-DESIGN.md)。契約・検証候補であり、閾値・技術選択・実装許可を確定しない。

## 2026-11-16 — Nagi Powerとの役割分担

[DEC-067](DEC-067-nagi-power-deterministic-resource-management.md)で、AI非必須のOS全体の電力・熱・負荷・背景資源管理をNagi Power（仮称）として採用した。Memory側は本書の検索経路選択とCanonical Memory保護を維持し、索引整備等もOSの通常資源契約に従う。OS全体の制御とMemory内の適応処理を重複した権限主体にせず接続する方針であり、Memory Resource Governor案のAPI・閾値・技術を確定するものではない。

変更履歴: 2026-11-16 — DEC-067との役割分担を追記。既存Memory方針・0.3実装範囲・Gateは変更しない。

## 2026-11-25 — 長期蓄積に対する容量・読出し性能の設計要件

**CONFIRMED（ユーザー要件）:** 長期利用でMemoryが蓄積することによりストレージを圧迫したり、記憶読出しや通常OS操作が顕著に遅くなったりする設計を避ける。Memoryの容量と検索性能のスケーリングを重要な評価軸とし、前景応答性を維持する。DEC-004の保護・自然忘却・明示的忘却、DEC-003の分類・所有・Permission、DEC-005の原本／閲覧記憶境界、DEC-057のPersona／Secret方針（関連PR #249、統合時に照合）を性能や容量の都合で弱めない。

**PROVISIONAL — 設計・検証候補（数値や方式の採用決定ではない）**

- Canonical Memoryを短い構造化された知識・事実・習慣として管理し、原本や全文ログを無制限に複製しない。参照・重複整理・出典保持・圧縮は安全な所有／分類境界内で検証する。検索用Index、Embedding、Cache、Browsing Archive、監査等の容量を区別し、合計の実ディスク使用量、更新／圧縮時の一時領域、バックアップ等の別管理領域も見える形で測定する。
- 端末全体の空き容量とProfile等の使用量に応じた容量予算・通知・可視化、派生Index／Cacheの再生成可能な整理、低重要度の未保護データの既存Retentionに基づく整理を検討する。保護データで予算を満たせない場合は、無断削除ではなく新規の任意保存の抑制・ユーザー選択等を検討する。上限の具体値・設定UI・優先順は未確定。
- 現行PermissionとPersona／Secretの適用境界を検索前に評価し、許可された範囲のmetadata／全文索引等で候補を限定し、必要時のみsemantic検索・reranking・追加取得を行う。通常の依頼で全記憶を読み込む設計や全履歴をLLMへ投入する設計を避ける。検索・Contextの候補数／時間／RAM／Token予算、Active／Archiveの検索経路、更新の差分反映、背景の整理・Index更新を予算化する案を比較する。前景優先、失敗・圧迫時の安全なfallbackは既存方針を維持。
- MEMORY-BENCH-01の1k／10k／100kを維持し、1M件級の大規模・長期利用相当の合成データ、短文以外の実用的な長さ分布、日本語／英語、Persona／Secret、複数Profile、Archive、削除・更新・再起動・復元・容量逼迫、同時前景負荷を追加する案。データ件数と実バイト、インデックス増分・書込増幅・一時領域、cold/warm p50/p95/p99、Context構築とLLM推論の分離、RAM、正答率、漏えいゼロ、長期間の増加率を測定する。数値合格条件と機器は実測後のImplementation Specで決定する。
- 2026-11-04のhost-only A/B smokeは100k件でDB 25.68 MiB、B p95検索＋Context 0.1501 msを記録した（同一DB/cache/process、短い合成データ、容易なanchor検索）。物理compaction、C、実用多言語意味検索、LLM推論、実機/QEMUは未測定。製品容量・実機速度・長期スケーリングの保証や採用判断には用いない。

具体的な容量上限、技術、保持／削除条件、性能閾値、提供版はPROVISIONAL。既存の先行検証と正式Runtime実装の境界を維持する。本追記は新たな作業開始許可・受入済み・Gate変更ではない。

変更履歴: 2026-11-25 — 長期記憶蓄積に伴うストレージ圧迫・検索性能劣化の回避を設計要件とし、容量予算・検索予算・大規模検証案をPROVISIONALで追加。
