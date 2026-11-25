# MEMDB-RD-01（仮）— Memory専用DBのhost-only研究レーン登録提案

**2026-11-25／PROVISIONAL／未登録・開始承認待ち**

関連: [DEC-013](../product-decisions/DEC-013-lightweight-memory-engine-benchmark.md)、[DEC-090](../product-decisions/DEC-090-common-persistence-contract-distributed-authority.md)、[MEMORY-BENCH-01](../product-decisions/benchmarks/MEMORY-BENCH-01.md)、[Memory Resource Governor設計案](../product-decisions/MEMORY-RESOURCE-GOVERNOR-DESIGN.md)

## 登録と開始条件

| 項目 | 提案・確認事項 |
|---|---|
| IDと担当 | 仮ID `memdb-rd-01`。実装Ownerは未割当。現行のレジストリ上のIntegration Ownerへ登録・範囲審査を依頼するための提案。 |
| 調査基点 | main `1999e58b31c7c93817a420cfc5f8dc40f75fecec`。開始時のbase SHAはIntegration Ownerが最新の統合先を確認して指定する。 |
| 作業分離 | Owner branch候補 `research/memdb-rd-01`、専用worktree、スタンドアロンRust harnessとローカル実行。 |
| 許可パス候補 | `tools/memory-db-research/**` に独立manifest・lockfile、合成fixture生成器、試験、比較ハーネスを収める。記録候補は `.dev/workstreams/memdb-rd-01/**`。生成DB等の実験出力は専用の一時領域に置く。 |
| 対象外 | ルートCargo manifest／lockfile、共有CI・IDL・schema、他レーンのパス、製品runtime・アプリ・ゲスト／Linux imageへの接続。レジストリ編集はIntegration Ownerの手続きに従う。 |
| 依存 | 初回スモークはMEMORY-BENCH-01の合成fixture・query judgmentsとDEC-090に沿う仮契約で完結。Identity・Permission・Source・Storageは合成入力として扱い、Governorの資源条件はホスト計測で記録する。製品連携には実際のIdentity／Permission／Storage／Search／Jobsと復元契約、および0.3 §26の正式仕様・別途実装許可が必要。 |
| 開始判定 | Integration OwnerがID、担当、base SHA、branch／worktree、許可パス、host-only開始範囲と成果の統合先を確定し、登録と開始承認を記録した後、下記の1kスモークまで起動可能とする。既存の一般的なhost研究方針とSearchの担当範囲は、このレーンへの割当を与えていない。 |

根拠は [Workstream運用](../0.2/WORKSTREAMS.md)、[現行レジストリ](../../.dev/workstreams.json)、[AGENTS.md](../../AGENTS.md)。公開順序は [DEC-123](../product-decisions/DEC-123-kernel-unification-and-linux-0.1.md)、既存承認範囲での着手は [DEC-111](../product-decisions/DEC-111-implementation-readiness-triage.md) に従う。

## 試験設計案

1. **共通Storage契約とversioned export。** 同一の合成fixtureから、Profile／Persona、分類、安定ID・Revision、Source・provenance、保護／忘却状態、更新と保存結果を比較できる中立契約を定義する。現行権限は認可元の合成スナップショットから再評価する。版付きexport/importの往復は論理レコード・由来・忘却状態を照合し、派生索引は再構築する。未知の版、部分保存、結果不明を成功扱いしない。形式・API・データ構造は暫定。
2. **1kスモーク（初回）。** seedと時計を固定した1,000件の日本語・英語の合成Memory、同一query judgments・操作順でSQLiteと再生成可能なRust製索引を比較する。SQLiteが正本を保持し、索引を消して再構築・再起動する。Profile／Persona切替、許可と拒否、Secret相当の合成ラベル、権限剥奪・明示的忘却を含め、旧索引やcacheから結果・存在・件数が漏れないことを確認する。データ・ログ・exportは合成のものに限り、実ユーザーデータ、実鍵、実秘密は使わない。
3. **Hybrid候補。** 再生成可能な独自索引をSQLiteの正本に組み合わせ、更新遅延・revision整合・再構築・SQLite経路への復帰を検証する。Governor設計案に沿い、前景と並行した更新・整理は負荷に応じて中断／延期できる条件で測る。
4. **Canonical storage候補。** 上記の比較を経て、独自DBが正本を扱う案を評価する。SQLiteからの移行、合成データでの往復照合とロールバック、旧世代復元後の現行認可と忘却、プロファイル分離を検証する。[DEC-004](../product-decisions/DEC-004-memory-learning-forgetting.md)、[DEC-057](../product-decisions/DEC-057-growing-personal-agent-os.md)、[DEC-105](../product-decisions/DEC-105-user-environment-backup-and-wayback.md)の方針を適用し、旧Backup単体で現行の忘却が確認できなければ対象の利用を保留する。Persona間の分離とSecretの利用・export制限は、索引・cache・復元でも検証する。

## 比較条件と判断材料

- **公平性:** MEMORY-BENCH-01の1k／10k／100k件を使い、1M件は追加候補とする。日本語・英語、短文・長文、長期利用を模した反復更新・削除・再起動を含める。同じfixture、query judgments、更新順と検索品質・認可・忘却基準を使い、同等の耐久性設定と負荷条件を明記する。認可違反や忘却済み情報の復活がある候補は採用評価から外す。
- **性能・資源:** cold／warm別に検索・Context構築・open／復旧のp95／p99、サンプル数、前景遅延と検索品質（Recall@k等）を報告する。正本／索引／cache／一時領域の実ディスク容量、RAM、論理更新量に対する実書込量（書込増幅）、実行環境・版・commit・seedを記録する。p99のサンプル数はMEMORY-BENCH-01の報告規則に従う。
- **障害・復旧:** 隔離した合成ストアに対して、書込み中断／電源断相当、破損、容量不足、移行中断、旧世代からの復元を比較する。確認済み保存・未保存・部分成功・結果不明、整合性、復旧後の認可・忘却、SQLiteへの復帰可否を記録する。ホストの強制終了等と実機電源断の証拠は区別し、未実測を明示する。
- **成果:** 再現手順・合成seed・集計と失敗条件を提出する。独自DBを採用候補に進める判断は、検索品質・安全性・耐久性と移行／復元の検証、同条件で得た性能・容量上の十分な利点に基づく。正式な採用・数値閾値・置換時期・提供版・具体データ構造は後続審査に委ねる。0.3の製品受入は [改訂ロードマップ §28](../product-decisions/roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md#28-acceptance--exit-criteria-for-03) に従う。
