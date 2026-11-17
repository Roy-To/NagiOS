# DEC-111 — 50項目のImplementation Readiness棚卸しと判断待ちの限定運用

**状態:** CONFIRMED（50項目を横断して未決詳細を仕分け、実装を妨げる判断だけ先に扱う運用方針）／REVIEW NOTE・PROVISIONAL（下記各項目の分類・順序・必要時期。新たな製品機能の採用ではない）  
**合意日:** 2026-11-17  
**対象:** 既存の[Foundation 35項目](NAGI_OS_FOUNDATION_CHECKLIST.md)と[Product & Experience PX-01〜15](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)  
**関連:** [DEC-069](DEC-069-foundation-checklist-management.md)、[DEC-044](DEC-044-linux-migration-development-authorization.md)、[DEC-049](DEC-049-development-process-speed-priority.md)、[DEC-103](DEC-103-testing-release-evidence-baseline-ai-assisted-staged.md)、[0.3 Roadmap](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[0.5〜1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

## 目的と運用上の合意（CONFIRMED）

ユーザーの指示により、Foundation 35＋PX 15＝**50項目の高水準方針検討を経て、未決詳細を一律に先決するのではなく、実装上の支障が出る判断だけを先に扱う工程へ移る**。新たな「全50件の詳細仕様確定Gate」や詳細承認の追加、作業停止、第三の巨大Checklistを設けない。

- **既存CONFIRMEDはそのまま正本DECに置く。** 本書は未決事項の経路分類だけを行い、具体的な製品挙動・版・技術採用・数値・権限を確定しない。FoundationとPX台帳を再複製せず、個別の「次に決める」「PROVISIONAL」を直接参照する。
- **実装の着手は既存承認の範囲で続ける。** Linux kernel＋Nagi userland、Native AppsとPermission・Memory／Data分離などの確定方針を維持し、可逆的技術比較を担当へ委任する（DEC-044／049）。現行owner・shared paths・Release／Activation／Acceptance Gateを変更しない。
- **阻害の定義を絞る。** （a）既存の契約と矛盾してコードを安全に進められない、（b）ユーザーの権限／Memory／削除／外部送信等の製品選択なしに機能を有効化できない、（c）試験・実測なしに恒久方式／正式対応を確定できない、のいずれかに該当するか、該当する実行工程を確認する。それ以外は担当が既存方針内で比較・試作・修正を進める。
- **実作業BLOCKEDと混同しない。** ここでいう分類は2026-11-17の公開Checklist・DECからの机上棚卸しで、実作業が停止している証拠でも、workstreamへの担当割当でもない。実際の停止は対象branch／PR／current codeと既存受入で確認する。未マージPRはmainの実装済みとしない。
- **不明な点は安全側へ。** 権限・Privacy等を緩めず、未決の自動削除や不可逆操作を勝手に有効化しない。比較結果がないA/Bの恒久採用、未測定の8GB性能、未実機の正式対応を宣言しない。
- **実装担当への伝達は軽くする。** 個別の正式契約・ADR・実装Evidenceは既存の保存先に記し、DEC-049の速度優先運用に沿って重複したJSON・受領記録・承認手続を追加しない。

## 50項目の一次仕分け（REVIEW NOTE / PROVISIONAL）

同じ項目に製品未決・技術未決・証拠不足が併存し得る。以下は**今次工程で最初に管理する主分類**のみ。各DECの製品採用状態、Foundationの既存P0/P1/P2、PXのPARTIALLY_RECORDED状態を改変しない。

| 主分類 | 件数 | 意味 |
|---|---:|---|
| E | 23 | 現行の権限・作業境界で技術比較／最小実装を進められる。恒久採用やTarget PASSは試験を要する。 |
| V | 5 | 比較や実機実測が先。製品の正式採用・性能保証は必要な試験後に改めて判断する。 |
| U | 8 | 実際にユーザー影響が生じる前に明示判断を要する。現在のLinux移行全体を止める意味ではない。 |
| R | 14 | 方針が既に決まっており、詳細UXや拡張技術は適用段階に合わせて具体化する。 |
| **合計** | **50** | Foundation 35、PX 15を欠落・重複なく分類 |

| ID | 既存項目 | 主分類 | 未確定事項の扱い（要約） |
|---|---|:---:|---|
| [F01](NAGI_OS_FOUNDATION_CHECKLIST.md#f01) | Linuxカーネル管理 | E | LTS系列の具体revision/config/固定build入力 |
| [F02](NAGI_OS_FOUNDATION_CHECKLIST.md#f02) | Boot System | E | 開発targetのboot入力・handoffと失敗検知 |
| [F03](NAGI_OS_FOUNDATION_CHECKLIST.md#f03) | System Image | E | 最小rootfs、System/User Data分離、更新slot |
| [F04](NAGI_OS_FOUNDATION_CHECKLIST.md#f04) | Init / Service Manager | V | A第一候補とBのLinux比較後に正式方式を決める |
| [F05](NAGI_OS_FOUNDATION_CHECKLIST.md#f05) | 基本ユーザーランド | E | libc／Rust runtime、ABI、最小OSS構成 |
| [F06](NAGI_OS_FOUNDATION_CHECKLIST.md#f06) | 電源ライフサイクル | R | Sleep・失敗時状態機械は対象段階で具体化 |
| [F07](NAGI_OS_FOUNDATION_CHECKLIST.md#f07) | Identity / Session | E | Linux資格情報とNagi Sessionのbinding・失効 |
| [F08](NAGI_OS_FOUNDATION_CHECKLIST.md#f08) | Permission / Sandbox | E | Sandboxの迂回拒否と権限取消の実装契約 |
| [F09](NAGI_OS_FOUNDATION_CHECKLIST.md#f09) | IPC / Service Broker | E | Linux transport／peer認証／Typed API |
| [F10](NAGI_OS_FOUNDATION_CHECKLIST.md#f10) | Secrets / Key Management | E | 鍵階層・保管、認可・回復経路の実装 |
| [F11](NAGI_OS_FOUNDATION_CHECKLIST.md#f11) | システム信頼基盤 | E | 署名チェーン・開発鍵／製品鍵と検証 |
| [F12](NAGI_OS_FOUNDATION_CHECKLIST.md#f12) | Storage / Filesystem | E | Linux FSとstable Resource ID・耐久性 |
| [F13](NAGI_OS_FOUNDATION_CHECKLIST.md#f13) | Volume / Mount | R | 外部Volume・safe ejectの具体化は後続 |
| [F14](NAGI_OS_FOUNDATION_CHECKLIST.md#f14) | Data Persistence | E | durable commit・結果不明・Schema Migration |
| [F15](NAGI_OS_FOUNDATION_CHECKLIST.md#f15) | Crash Recovery | E | Recovery Coordinator・再実行防止・Journal |
| [F16](NAGI_OS_FOUNDATION_CHECKLIST.md#f16) | Backup / Restore | U | 正式Backup対象／除外・部分復元・忘却境界 |
| [F17](NAGI_OS_FOUNDATION_CHECKLIST.md#f17) | Device Manager | E | 最初のdevice列挙・認可／hotplug |
| [F18](NAGI_OS_FOUNDATION_CHECKLIST.md#f18) | Network Devices | E | Ethernet・Wi-Fi初期接続／firmware契約 |
| [F19](NAGI_OS_FOUNDATION_CHECKLIST.md#f19) | Display / GPU | E | Linux描画backend・最低限のGUI・fallback |
| [F20](NAGI_OS_FOUNDATION_CHECKLIST.md#f20) | Input System | E | keyboard／mouse・focus／IME・入力認可 |
| [F21](NAGI_OS_FOUNDATION_CHECKLIST.md#f21) | Audio System | R | 音声機能の対象段階でbackendと受入を具体化 |
| [F22](NAGI_OS_FOUNDATION_CHECKLIST.md#f22) | 周辺機器 | R | 対象USB／HID以外の機器詳細は段階対応 |
| [F23](NAGI_OS_FOUNDATION_CHECKLIST.md#f23) | Nagi Power | V | 電力／熱のbaseline実測後に最適化判断 |
| [F24](NAGI_OS_FOUNDATION_CHECKLIST.md#f24) | Network Service | E | Linux DNS／TLS・Offline・通信認可 |
| [F25](NAGI_OS_FOUNDATION_CHECKLIST.md#f25) | Resource Manager / Scheduler | V | Linux標準baselineと独自研究の実測比較 |
| [F26](NAGI_OS_FOUNDATION_CHECKLIST.md#f26) | App Lifecycle | E | Nagi App SupervisorとLinux process／sandbox |
| [F27](NAGI_OS_FOUNDATION_CHECKLIST.md#f27) | Settings / Notification | E | 設定Scope・永続化・基本通知サービス |
| [F28](NAGI_OS_FOUNDATION_CHECKLIST.md#f28) | Time / Locale / Accessibility | E | clock権威・Locale・最小keyboard a11y契約 |
| [F29](NAGI_OS_FOUNDATION_CHECKLIST.md#f29) | Diagnostics / Logging | E | Linux boot／crash診断と秘匿・証拠 |
| [F30](NAGI_OS_FOUNDATION_CHECKLIST.md#f30) | Installer / First Setup | R | 実ディスク導入と初回UIは後続の受入段階 |
| [F31](NAGI_OS_FOUNDATION_CHECKLIST.md#f31) | System Update | V | Linux Imageの更新・rollback・電源断検証 |
| [F32](NAGI_OS_FOUNDATION_CHECKLIST.md#f32) | Device Connectivity | R | Share／Handoffの製品段階で最小Protocol詳細 |
| [F33](NAGI_OS_FOUNDATION_CHECKLIST.md#f33) | Hardware Support | V | 開発targetと正式対応端末を分離し実機で判定 |
| [F34](NAGI_OS_FOUNDATION_CHECKLIST.md#f34) | Testing / Release | E | 既存Gateを保つ最小Linux統合受入と証拠 |
| [F35](NAGI_OS_FOUNDATION_CHECKLIST.md#f35) | Build / Supply Chain | E | source pin・SBOM・再現build・署名検証 |
| [PX-01](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | Nagi Home / Desktop | R | 詳細画面・導線は確定B方針内でprototype可能 |
| [PX-02](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | Intent-first UI | R | カード・提示条件などの詳細は段階具体化 |
| [PX-03](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | Project / Workspace | R | 統合ビューと検索・復元導線の詳細は後続 |
| [PX-04](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | 音声・テキスト対話 | R | Wake Word等を前提にせず既存B方針で先行 |
| [PX-05](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | AIの自律実行・承認 | U | 実際に有効化するAction別リスク／人間専用区分 |
| [PX-06](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | 長時間Task・並行作業 | R | Queue／UI／並列度の詳細は測定と実装で調整 |
| [PX-07](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | 先回り提案 | R | 頻度・抑制・却下の具体UIは後続 |
| [PX-08](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | Memoryの自動学習範囲 | U | 重要Memoryの確認条件・未回答時挙動 |
| [PX-09](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | Memoryの保持・忘却 | U | 自動削除・保護・例外の実運用条件 |
| [PX-10](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | 対話スタイルの個人化 | R | 詳細設定と適応評価は後続 |
| [PX-11](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | 純正アプリ標準搭載 | U | 1.0での必須／任意／後続アプリと最低機能 |
| [PX-12](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | アプリ横断・Smart Paste | U | 初期のアプリ組合せ、コピー／参照の範囲 |
| [PX-13](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | Nagiによるアプリ制作 | R | 高度IDEは後順位、既存IDE／Cloud橋渡しを先行 |
| [PX-14](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | 第三者アプリ開発・配布 | U | 1.0公開SDKの対応言語・Package／配布範囲 |
| [PX-15](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) | 1.0製品完成基準 | U | 代表作業・正式対応範囲・測定可能な合否基準 |

## 「先に決める」条件付き判断の順（REVIEW NOTE / PROVISIONAL）

**現時点の仕様正本だけからは、Linux最小起動／通常アプリの試験着手に新たなユーザー製品判断を必須とする根拠は特定されない。** 実際のコードやブランチがBLOCKEDでないと断定するものではない。最初はF01〜05、F07〜09、F12／14／15、F19／20、F26、F29／34／35等のLinux同一本体の縦断実証を優先し、既存仕様・担当の範囲で進める。

| 条件・時期 | 対応ID | 判断すべきこと | 先行作業（判断を待たずにできること） |
|---|---|---|---|
| Init方式の**同条件Linux比較後** | F04 | A（最小独自PID 1＋独自Service Manager）／B（OSS軽量Init＋Nagi Manager）の正式製品採否 | 両案の起動・子process回収・異常系・保守負担を測る。DEC-080のA第一候補は最終採用ではない |
| Personal Agentが実際の副作用あるActionを扱う前 | PX-05 | Action別リスク区分、必須画面確認、人間専用・不明時の扱い | 非特権AI・現行Permission／Policy／Task Scope／Recoveryを前提に型付きAction・拒否試験を先行 |
| 重要なMemoryの自動学習を運用へ載せる前 | PX-08 | 重要度・確認タイミング・未回答・推測の扱い | 事実／推測とSource／Profile／機密境界の保存契約・テストを先行 |
| 自然忘却による**自動削除**を実行する前 | PX-09 | 保護・保持・削除条件と例外、利用者の修正・忘却、派生Indexの扱い | Active→Decay→Archiveの状態遷移・復旧／明示忘却の安全性を先行検証。未決の数値を正式規則にしない |
| 最初のアプリ横断UXを正式なAcceptanceに含める前 | PX-12 | 初期対象アプリ組合せ、コピー／参照／埋込みの提供範囲とユーザー選択 | Public Action・Resource／Revision・Smart Pasteの既存契約に沿ったsample実装 |
| 全体Backup／Restoreを製品機能として保証する前 | F16 | 対象・除外、鍵と回復可能範囲、削除・忘却済み情報を再導入しない条件 | Workspace／ProfileとService別保存正本の構造を先に接続する。Waybackは完全Backupと同義ではない |
| 1.0の機能・アプリ要件を凍結する前 | PX-11、PX-15（F33・F34を参照） | 代表作業・必須アプリと機能、正式対応Targetの範囲・公開時の受入条件 | Linux Image／QEMUから対象実機まで段階的検証。対応機種／測定閾値は実測証拠で確定 |
| 1.0公開開発者環境・配布の正式範囲を宣言する前 | PX-14 | 公開SDKの言語・toolchain、Package・署名・配布の保証範囲 | 既存のRust SDK等・Porting Toolkitのhost検証、Linux target上の実sampleを先行 |

**次に人間へ質問する条件:** 対象ID、試験・実装から生じた対立する選択肢、影響するユーザー体験・安全境界、A/B/Cの利点・欠点、まだ未測定のことを**1件ずつ**提示する。指定のない時期・閾値・採用結果を自動確定しない。上表の順序は絶対的なgateではなく、実際にその機能を進めるタイミングに応じて前後する。

## 検証証拠と進行の更新方法

1. 既存の[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[PX Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)の各詳細欄を現在のDEC・実装ブランチ・Acceptanceに照らして確認する。本書に別個のprogress JSONやstateを量産しない。
2. 技術判断は実装可能な可逆的選択と恒久採用を区別して担当がADR／実装契約へ記録し、Host／旧Guest／Linux Image／実機の実行結果と紐づける（DEC-044／049／103）。
3. 0.3 Implementation Specの正式な作成・有効化は[0.3 Roadmap §26](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md#26-gate-for-03-implementation-spec)の既存条件を維持する。50項目の棚卸しでGateを新設／撤回しない。
4. 共同開発者を含む担当の技術選択は既存の委任・権限内に限る。製品方針の追加選択、恒久採用判断、正式な対応Target宣言は必要な人間判断へ戻す。
5. 具体的な合意が出たら対応DEC・README索引・必要な既存Checklistを更新し、公開GitHub commitを検証した場合だけ保存済みと報告する。

## 変更履歴

| 日付 | 変更 | 状態 |
|---|---|---|
| 2026-11-17 | 50項目から実装時の判断条件を抽出する運用へ移行。50項目の一次主分類と8つの条件付き相談入口を追加 | 運用方針CONFIRMED。分類／順番・実装への影響評価はREVIEW NOTE／PROVISIONAL。新たな製品仕様やGateは未確定 |
