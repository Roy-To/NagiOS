# Nagi OS Foundation Checklist

**台帳採用:** CONFIRMED — [DEC-069](DEC-069-foundation-checklist-management.md)（2026-11-16）
**正本:** 公開 `Roy-To/NagiOS` / `docs/product-decisions/`
**初回調査基準:** main `029bec16c0e7befa6856c4bd43805d1662bf0f58`、2026-11-16確認の公開PR。実装・targetの新規試験は行っていない。
**優先度:** 全行のP0/P1/P2は **REVIEW NOTE / PROVISIONAL（assistant提案）**。ユーザー確定の製品優先順位・release必須条件ではない。

## 1. 扱い方と状態の読み方

この台帳は、元の35項目を省略せず管理する索引・進捗台帳である。6分類群と、横断的な7つの検討束は別の軸。個別技術、製品要求詳細、実装開始、版、担当、性能数値を一括確定した記録ではない。

- **製品決定:** `CONFIRMED`＝根拠DECで明示確定した範囲のみ、`PROVISIONAL`＝既存の暫定案、`OPEN`＝選択・要件が未決。1項目に複数が併存する。実装仕様の契約があるだけで、そのLinux版の技術をCONFIRMEDにしない。
- **技術設計:** 既存契約あり／旧基盤設計あり／候補・移行設計待ちを記す。製品方針と可逆的な開発用選択、恒久的技術採用を区別する。
- **実装・受入:** ソースの存在、host foundation、旧独自カーネルguest、未マージPR、Linux同一本体adapter、Linux image、実機を別々に記す。`PASS`は対象commit・環境・実行結果・対象Acceptanceを結び付けられる範囲だけ。文書・候補選定・過去のhost PASSからtarget PASSへ昇格させない。
- **未確認**は、今回の公開ref・証拠で確認できないという意味。他環境も含めた未着手の断定ではない。詳細欄の「Linux／実機未確認」はその項目の移行後の受入が未確認であることを表す。
- **依存・責任候補:** IDは技術上の関連候補であり、全件完了待ちのGateではない。workstream名は接続先候補。既存owner／branch／allowed paths／checkpointを優先し、担当割当てを新設しない。
- 仕様確定でAI実装分業を加速できるが、Linux／実機の統合、セキュリティ、電源・driver、測定、受入はAIの自動判断だけで完了しない。全35項目確定待ちという新しい開発停止条件も作らない。

### Linux移行で古い文言との差異

| 既存文言・契約 | 現在の解釈・台帳への反映 |
|---|---|
| 0.1の「not Linux-based」、Nagi Loader → Nagi Kernel → nagi-init、独自syscall／channel／VMO | 旧基盤の仕様と実績として保持。[DEC-042][D42]／[DEC-043][D43]はLinux kernel + Nagi専用userlandを確定。boot、ABI、IPC、driver、保存等のLinux接続は別設計・証拠が必要 |
| 既存distribution採否が未確定とするDEC-042初回欄 | 後続DEC-043により、既存distributionを製品OS基盤として流用しない方針は確定。個別OSSは選択的利用可能。libc／init等の構成は未確定 |
| 「NOT ASSIGNED」「build／VM等を開放しない」という初回snapshot | [DEC-044][D44]、後続[DEC-047][D47]の割当・操作・統合先、[DEC-049][D49]の開発手続きが優先。公開refで成果を確認できないことを開発承認の不在としない |
| Privateへの保存、Publicへの転載禁止という旧文言 | [DEC-050][D50]により公開リポジトリが正本。旧privateへ新規記録しない。秘密・個人情報等の非掲載規則は維持 |
| 0.2のQEMU／x86-64／VirtIO前提、Wi-Fi／Bluetooth／suspend/resume等の非必須 | 既存releaseの受入範囲として保持。将来機能採用を既存Gateへの追加にしない。Linux恒久targetや実機対応の証明にも使わない |
| 実機参照仕様と将来の端末候補 | [DEC-052][D52]の将来候補はPC＝Raspberry Pi（型番未指定）、tablet＝PineTab2、phone＝未選定。0.2の最初の実機参照機を選定したx86-64 UEFI mini-PCとする方針、generic ARM64→Piの境界は維持。候補選定は正式対応・購入・起動成功ではない |
| 0.3／0.4+／0.5–1.0 roadmap | [索引][RM]の位置付けを維持。0.3仕様化条件、0.4 Entry Gate、0.5–1.0の暫定版割当てを変更しない。DEC-013の正式Memory EngineをLinux移行だけで後ろ倒しにしない |

既存[Linux影響表][IMPACT]と[0.2移行補足][ADD]は2026-11-13のsnapshotを含む。後続DEC-047／049／050と併読し、その当時の未確認状態を現在の全開発へ拡張しない。

### 採用済み製品DECと仕様正本（2026-11-16統合更新）

以下は公開PRに保存されたユーザー決定と、統合する仕様正本への参照。2026-11-16の初回台帳では未マージPRとして区別していたが、PR #107〜#129の統合作業で参照を更新する。仕様統合は実装・target受入の証拠ではない。衝突する古い「未採用」「接続方式未定」は後続DECと時系列で読む。各PRのマージ状態・CIはGitHubで確認する。

| 記録 | 確定範囲と未決境界 | 影響ID |
|---|---|---|
| [DEC-064 / PR #125][PR125]（[仕様正本][P64]） | Shareの単発ファイル転送、受信許可、暗号化・認証、ローカル／LLM非必須。方式・機種・版は暫定 | 10, 17, 18, 24, 32 |
| [DEC-065 / PR #126][PR126]（[仕様正本][P65]） | Handoffの意味的作業継続、UI再構築、実体と参照の分離、現行認可。全状態移行・無断Task再実行を認めない | 7, 14, 15, 26, 32 |
| [DEC-066 / PR #127][PR127]（[仕様正本][P66]） | 初期は同一Wi-Fi／LAN、Connectivity／Trust／Transportの共通抽象、初期Cloud運営非必須。mDNS／Bluetooth／Wi-Fi Direct／relay等の採否は未定 | 8, 9, 10, 17, 18, 24, 32 |
| [DEC-067 / PR #128][PR128]（[仕様正本][P67]） | Nagi PowerのLLM非依存・決定論的電力／熱／資源管理、前景優先、安全限界内の指定。Linux interface・閾値・効果・端末は未定 | 6, 23, 25 |
| [DEC-068 / PR #129][PR129]（[仕様正本][P68]） | Sleep／復帰／ロック、必要な状態保存、接続再検証、Task再実行禁止、実機検証後の段階的hibernate。sleep方式・swap・鍵・時期は未定 | 6, 7, 10, 14, 15, 17, 23, 25 |
| [PR #120][PR120]、[PR #124][PR124] | IMEの入力応答優先とAI学習、8GB級実用AIと適応・手動モデル切替の並行記録。製品方針の記録であり、実測性能達成ではない | 20, 25, 28, 33, 34 |

## 2. 35項目の概要索引

番号・名称・優先度は元の提示を保持する。詳細の状態を一つの「未着手」へ丸めない。**各行の推奨優先度は全てREVIEW NOTE / PROVISIONAL。**

### 第1群 カーネル・起動・基本ユーザーランド

| ID | 名称 | 推奨優先 | 初回評価の要点 |
|---|---|---|---|
| [1](#f01) | Linuxカーネル管理 | P0 | Linux採用確定／版・保守未定 |
| [2](#f02) | Boot System | P0 | 旧UEFI設計・実装／Linux boot未確認 |
| [3](#f03) | System Image | P0 | 旧image／slot基盤／Linux rootfs未定 |
| [4](#f04) | Init / Service Manager | P0 | 旧init／監視契約／Linux PID1未定 |
| [5](#f05) | 基本ユーザーランド | P0 | Nagi専用方針確定／libc等未定 |
| [6](#f06) | 電源ライフサイクル | P1 | Sleep決定はPR／実機未確認 |

### 第2群 セキュリティ・ユーザー・権限管理

| ID | 名称 | 推奨優先 | 初回評価の要点 |
|---|---|---|---|
| [7](#f07) | Identity / Session | P0 | 既存認証・session契約／Linux強制未確認 |
| [8](#f08) | Permission / Sandbox | P0 | 権限境界確定／Linux隔離方式未定 |
| [9](#f09) | IPC / Service Broker | P0 | typed契約・host資産／Linux transport未定 |
| [10](#f10) | Secrets / Key Management | P0 | Workspace鍵方針あり／全OS鍵設計未定 |
| [11](#f11) | システム信頼基盤 | P0 | 旧署名・trust基盤／製品鍵等未完 |

### 第3群 ストレージ・ファイルシステム・データ保護

| ID | 名称 | 推奨優先 | 初回評価の要点 |
|---|---|---|---|
| [12](#f12) | Storage / Filesystem | P0 | 旧VFS／ext2資産／Linux FS未定 |
| [13](#f13) | Volume / Mount | P1 | 旧storage layout／汎用mount設計未定 |
| [14](#f14) | Data Persistence | P0 | 永続状態・所有境界確定／Linux耐久未確認 |
| [15](#f15) | Crash Recovery | P0 | 選択式復旧確定／旧基盤残件あり |
| [16](#f16) | Backup / Restore | P1 | B基本・Wayback連携・C追加方向確定／詳細未定 |

### 第4群 デバイス・ハードウェア管理

| ID | 名称 | 推奨優先 | 初回評価の要点 |
|---|---|---|---|
| [17](#f17) | Device Manager | P1 | Devices／Trust契約あり／ローカル列挙未統合 |
| [18](#f18) | Network Devices | P1 | 旧VirtIO net／Wi-Fi・BT未確認 |
| [19](#f19) | Display / GPU | P0 | 旧compositor／GPU資産／Linux backend未定 |
| [20](#f20) | Input System | P0 | 旧input／IME資産／Linux経路未確認 |
| [21](#f21) | Audio System | P1 | 旧audio基盤／Linux backend未定 |
| [22](#f22) | 周辺機器 | P2 | USB storage・基本HID標準、camera／printer／scanner段階対応。具体機種・範囲未定 |
| [23](#f23) | Nagi Power | P1 | 決定論的管理採用はPR／実測未確認 |

### 第5群 ネットワーク・OS共通サービス

| ID | 名称 | 推奨優先 | 初回評価の要点 |
|---|---|---|---|
| [24](#f24) | Network Service | P1 | 旧net／TLS／Permission資産／Linux未確認 |
| [25](#f25) | Resource Manager / Scheduler | P0 | 前景優先確定／host Jobs等／Linux制御未定 |
| [26](#f26) | App Lifecycle | P0 | manifest host契約／runtime統合別 |
| [27](#f27) | Settings / Notification | P1 | scoped契約・host foundation／runtime別 |
| [28](#f28) | Time / Locale / Accessibility | P1 | en-US／ja-JP基盤あり／時刻・a11y残件 |
| [29](#f29) | Diagnostics / Logging | P0 | 旧診断・host構造化契約／Linux接続別 |

### 第6群 インストール・更新・端末間連携・品質保証

| ID | 名称 | 推奨優先 | 初回評価の要点 |
|---|---|---|---|
| [30](#f30) | Installer / First Setup | P1 | 旧owner初期化／製品installer未定 |
| [31](#f31) | System Update | P0 | 旧A/B・package host資産／Linux更新未確認 |
| [32](#f32) | Device Connectivity | P1 | Export／Continuity契約＋PR決定／実通信別 |
| [33](#f33) | Hardware Support | P0 | QEMU基準と実機候補を分離／実機未確認 |
| [34](#f34) | Testing / Release | P0 | CI／受入契約あり／Linux release未確認 |
| [35](#f35) | Build / Supply Chain | P0 | pin／provenance／SBOM資産／Linux成果別 |

## 3. 各項目の管理記録

各記録は、検討対象、製品決定、技術設計、実装・target受入、依存・責任候補、根拠、次の決定を持つ。優先度は上の概要表が正。全記録の初回更新日は2026-11-16、証拠基準は冒頭のmain／固定PR本文。参照先のhost結果は既存記録の引用で、今回再実行した試験ではない。

<a id="f01"></a>
### 1 Linuxカーネル管理

- **検討対象:** revision／更新保守、config、patch／module、脆弱性対応、driverとライセンス入力。
- **製品決定:** CONFIRMED＝Linux採用 [DEC-042][D42]、製品標準はLTS系列・開発／先行検証は最新Stableを許容 [DEC-077](DEC-077-linux-kernel-lts-stable-policy.md)。PROVISIONAL＝具体kernel revision・構成・移行時期。OPEN＝LTS更新運用・保守責任・Stable採用／取り込み条件。
- **技術設計:** 移行ADRで限定開発の委任範囲あり。恒久kernel revision／configは未確定。
- **実装・target受入:** mainの`kernel/`は旧独自実装。公開Linux image・実機受入は未確認。DEC-047の統合先指定は成果証拠ではない。
- **依存・責任候補:** 2, 3, 33, 35。`os-core`／Linux移行lane／Integration Owner（新規割当なし）。
- **根拠:** [移行ADR][ADR-L]、[DEC-044][D44]／[047][D47]、[0.1 §4][S01]、[影響表][IMPACT]。
- **次に決める:** LTS標準／Stable先行利用の方針を前提に、具体的系列・開発用固定入力、config／patchの管理単位・脆弱性更新責任・正式系列切替条件を具体化する。

<a id="f02"></a>
### 2 Boot System

- **検討対象:** firmware→loader→kernel→userland、UEFI／他arch、boot failure、回復起動と信頼の連鎖。
- **製品決定:** CONFIRMED＝Linux + Nagi userland [DEC-043][D43]、UEFI対応PCはUEFI基本・Raspberry Pi／タブレット等は機器別方式を許容・Linux起動後のNagi処理を共通化 [DEC-078](DEC-078-boot-platform-entry-and-common-nagi-startup.md)。PROVISIONAL＝機種別詳細・移行構成。OPEN＝Linux bootloader／handoff・回復／信頼の具体方式。
- **技術設計:** 旧UEFI loader／ABI／trust設計あり。Linux boot経路は置換・adapter判断が必要。
- **実装・target受入:** `loader/`と旧起動経路あり。旧guestの起動証拠をLinux起動へ流用しない。Linux／実機未確認。
- **依存・責任候補:** 1, 3, 4, 11, 33。`os-core`、boot担当候補、Integration Owner。
- **根拠:** [0.1 §5・6][S01]、[0.2 §6][S02]、[boot trust設計][BOOT]、[移行補足][ADD]。
- **次に決める:** PCのUEFI基本と機器別方式を前提に、一つの開発targetのboot入力・Linux handoff・失敗検知／復旧・信頼境界を定義し、旧loader再利用範囲をADRにする。

<a id="f03"></a>
### 3 System Image

- **検討対象:** rootfs、readonly／mutable分離、system／user data境界、image生成・配布・回復領域。
- **製品決定:** CONFIRMED＝既存distributionの完成rootfs等を製品基盤にしない [DEC-043][D43]、OS本体の通常時read-only、OS更新後の保持を目指すUser Data分離、A/B System更新・Recoveryを基本、AIモデルの論理的分離 [DEC-079](DEC-079-system-image-readonly-ab-data-separation.md)。OPEN＝image形式・構築技術・partition構成・filesystem・更新／復旧の具体方式。
- **技術設計:** 旧image builderとA/B設計あり。Linux rootfsとNagiサービスの搭載契約は未確定。
- **実装・target受入:** `tools/nagi-cli/src/image.rs`は旧loader／kernel／init入力。Linux image生成・boot・更新後再起動は未確認。
- **依存・責任候補:** 1, 2, 5, 11, 12, 31, 35。`os-core`／`developer-tooling`／更新担当候補。
- **根拠:** [0.1 §17・21・79][S01]、[0.2 §16・18][S02]、[boot trust][BOOT]、[影響表][IMPACT]。
- **次に決める:** read-only System／分離User Data／モデル論理分離／A/B復旧の確定方針を前提に、最小Linux image内容、生成入力、書込み境界、schema互換、slot切替／復旧と受入条件を具体化する。

<a id="f04"></a>
### 4 Init / Service Manager

- **検討対象:** PID1、起動順序、依存解決、ready／health、再起動上限、shutdown、service権限。
- **製品決定:** CONFIRMED＝Nagiがuserland責務を持つ [DEC-043][D43]、AI非依存の基本操作 [DEC-009][D09]、A（Nagi独自の最小PID 1＋独自Service Manager）を第一候補にB（既存OSS軽量Init＋Nagi Service Manager）との短期実装比較後に最終選択する手順 [DEC-080](DEC-080-init-service-manager-ab-evaluation.md)。PROVISIONAL＝A/B正式採用・試験詳細。OPEN＝最終方式の選択。
- **技術設計:** 0.1 supervision、0.2 Supervisor契約あり。Linux PID1／process／service registrationの具体接続は未定。
- **実装・target受入:** `user/nagi-init/`は旧guest用。manifest検証やhost監視モデルはLinux PID1動作を証明しない。Linux未確認。
- **依存・責任候補:** 2, 5, 7, 8, 9, 26, 29。`os-core`／app supervisor候補。
- **根拠:** [0.1 §68][S01]、[0.2 §13.2・M13][S02]、[移行補足][ADD]。
- **次に決める:** A第一候補／B比較方針に従い、限定的な同条件のLinux実装比較で起動・プロセス回収・失敗／再起動・工数／保守性を確認し、A/Bの正式採用を最終決定する。最小service graphとPID 1・Service Manager・app supervisorの責務分離は具体化待ち。

<a id="f05"></a>
### 5 基本ユーザーランド

- **検討対象:** libc、runtime、shell／基礎tool、filesystem layout、ABI、必要なOSSとNagi所有境界。
- **製品決定:** CONFIRMED＝Linux + Nagi専用userland／Nagi Native Apps [DEC-043][D43] [DEC-017][D17]、Nagi固有サービス・API・アプリ環境は独自設計、libc・汎用ライブラリ・基礎ツールはOSSを積極的に選択活用、既存Linux distributionのユーザーランド一式は製品基盤に流用しない [DEC-081](DEC-081-nagi-owned-userland-selective-oss-components.md)。PROVISIONAL＝具体libc／ABI・OSS／互換方式。
- **技術設計:** PAL／relibc／spawn等の旧契約と実装あり。Linux ABIの対応表が必要。全面POSIX／既存binary互換は確定していない。
- **実装・target受入:** `user/libnagi/`、`user/nagi-pal/`等は既存資産。Linux向けbuildだけで同じ権限・サービス契約の成立とはしない。
- **依存・責任候補:** 1, 3, 4, 8, 9, 26, 35。`os-core`／`sdk`／`developer-tooling`。
- **根拠:** [0.1 §6・60–65][S01]、[0.2 §26・M21][S02]、[移行補足][ADD]。
- **次に決める:** Nagi独自の契約／選択的OSS活用の確定方針に従い、最小component一覧・libc／Rust runtime・ABI・rootfs配置と実際の採用OSSを比較し、可逆的開発構成と正式採用を分けて判断する。

<a id="f06"></a>
### 6 電源ライフサイクル

- **検討対象:** shutdown／reboot／sleep／hibernate／resume、ロック・保存・job・deviceの協調と失敗処理。
- **製品決定:** CONFIRMED＝復元とTask実行の分離 [DEC-014][D14]／[022][D22]、[DEC-068][P68]のSleep方針、[DEC-067](DEC-067-nagi-power-deterministic-resource-management.md)の継続的Power管理、およびNagiによる電源遷移の一元管理／サービス協調／失敗時の中止・再試行・警告／データと安全性の優先 [DEC-082](DEC-082-power-lifecycle-coordination.md)。PROVISIONAL＝状態機械・処理順序・具体強制停止条件・端末・時期。
- **技術設計:** Linux標準機構＋決定論的NagiサービスによるSleepに加え、全電源遷移を統括する論理的Lifecycle Coordinator責務を確定。独立プロセスか既存Service Managerへ統合するか、状態機械・s2idle／deep／swap／hibernate等の実装技術はOPEN。
- **実装・target受入:** 通常RAM保持resume、障害後の再構成、hibernateを別評価。Linux image・実機の電源遷移は未確認。
- **依存・責任候補:** 4, 7, 10, 14, 15, 17, 23, 25, 33。Power／Sleep候補＋各サービスowner。
- **根拠:** [PR #129][PR129]、[0.2 §5.2・13・14・48][S02]、[DEC-020][D20]。
- **次に決める:** 保存・ロック・停止・再検証の状態機械、各サービスの協調契約、失敗ごとの中止／再試行／警告・強制終了条件を具体化する。既存0.2のsuspend非必須は変えない。

<a id="f07"></a>
### 7 Identity / Session

- **検討対象:** local user／profile／Owner／Guest、認証、session lease、ロック、起動後復旧とdevice identity。
- **製品決定:** CONFIRMED＝AI／Roleが権限を増やさない [DEC-009][D09]／[020][D20]、復旧時の現行権限再評価 [DEC-022][D22]、NagiのUserId／ProfileId／SessionIdを正本としLinux UID／GIDは低レイヤー保護に利用、ログイン／ロック等をNagi管理、Profile別データ／Memory分離とOwner自動昇格禁止 [DEC-083](DEC-083-nagi-identity-linux-credentials-binding.md)。PROVISIONAL＝具体的UID／GID mapping・認証方式・Session失効・実装版。
- **技術設計:** 0.2 stable IDs・local session契約あり。UID／PIDだけでPrincipal／Profile／Taskを代替しない。
- **実装・target受入:** 旧認証・session資産とhost foundationあり。[PR #48][PR48]のDesktop統合は未マージ。Linux session／ロック受入未確認。
- **依存・責任候補:** 4, 8, 9, 10, 14。`ident-01`／`capability-permissions`／session担当候補。
- **根拠:** [0.1 §22–24][S01]、[0.2 §8–10・M08][S02]、[registry][REG]。
- **次に決める:** Nagi論理Identityを正本にする方針に従ってLinux kernel credentialsとの具体的binding、expiry／logout／lock／resume時のSession失効・再認証条件、Guest／cross-profile拒否の実装とLinux target受入を設計する。

<a id="f08"></a>
### 8 Permission / Sandbox

- **検討対象:** app／service／AI隔離、データ・device・network権限、最小権限・取消・狭窄・Owner操作。
- **製品決定:** CONFIRMED＝非特権AI、正規API、現行認可と限定GUI fallback [DEC-020][D20]／[027][D27]、Nagi Permission／Capabilityの判断とLinux Sandboxの実効隔離を組み合わせて直接アクセス迂回を防ぐB案、最小権限・取消／失効・狭窄・AI／アプリの特権非自動付与 [DEC-084](DEC-084-nagi-permission-linux-sandbox-enforcement.md)。PROVISIONAL＝具体的Linux強制技術・配置／受入条件。
- **技術設計:** Nagi Capability／PermissionとLinux capabilitiesは別概念。namespace／seccomp／LSM等の採否は本台帳では決めない。
- **実装・target受入:** `crates/nagi-capability/`等のhost／旧kernel資産あり。broker外のLinux syscall・device nodeへの迂回拒否は未確認。
- **依存・責任候補:** 1, 7, 9, 10, 11, 26。`capability-permissions`／`os-core`／security担当候補。
- **根拠:** [0.1 §8・23・40・57][S01]、[0.2 §10・46][S02]、[移行ADR][ADR-L]。
- **次に決める:** Nagi判断＋Linux強制隔離方針に沿い、Threat Model／迂回経路、実装するLinux機構、caller／profile／resource別拒否・失効・取消、broker／syscall／deviceへの直接アクセス拒否のLinux target受入を具体化する。

<a id="f09"></a>
### 9 IPC / Service Broker

- **検討対象:** transport、service discovery／identity、IDL／versioning、caller binding、timeout／cancel、large data。
- **製品決定:** CONFIRMED＝typed Public Action／Serviceと決定論的実行 [DEC-020][D20]／[027][D27]、B案＝Nagi独自の型付きIPC／Service API・Brokerを保ち低レイヤーにLinux標準IPCを活用、Transport独立の契約とNagi Identity／Permissionを強制する基本方針 [DEC-085](DEC-085-linux-transport-nagi-service-broker.md)。PROVISIONAL＝具体transport（Unix Domain Socket候補）／wire／配置・受入。
- **技術設計:** 0.2のtransport-independent契約を継承し、Linux標準IPC上にNagi Broker／型付きService APIを実現する方向。UDS等は比較候補で未採用。旧Kernel Channel・VMOをLinuxで無変更利用できるとしない。
- **実装・target受入:** `crates/nagi-action-ipc/`／`crates/nagi-search-ipc/`、`sdk/rust/src/app_contract/ipc.rs`等の契約と旧IDL／channel実装あり。Linux peer認証・権限狭窄・再起動時binding未確認。
- **依存・責任候補:** 4, 7, 8, 29。`system-service-ipc`／`platform-apis`／`os-core`。
- **根拠:** [0.1 §9–11][S01]、[0.2 §11・M10][S02]、[影響表][IMPACT]。
- **次に決める:** 採用するLinux local transport（Unix Domain Socket等）、実CallerとNagi Sessionのbinding、fd／shared memory・bounded message、cancel／replay／version不一致・Service再起動時の契約とLinux target受入。

<a id="f10"></a>
### 10 Secrets / Key Management

- **検討対象:** credentials、device／package／storage鍵、生成・保管・利用・rotation・revocation・回復・消去。
- **製品決定:** CONFIRMED＝Workspace Exportのpassphrase基本・別保管回復手段・復号と認可の分離 [DEC-014][D14]、C案＝Nagi独自Secrets ServiceでOS秘密情報・鍵を用途／User／Profile別に管理し、TPM／TEE等の対応機器ではハードウェア保護を優先、非対応機器は暗号化ローカル保管／回復経路を利用、Identity／Permissionによる認可・AIへの秘密値非露出・更新／失効・無断ログ／クラウド預託なし [DEC-086](DEC-086-nagi-secrets-service-hybrid-key-protection.md)。PROVISIONAL＝暗号方式・KDF・具体Key Store／TPM／用途別回復可能性・適用版。
- **技術設計:** ハイブリッドKey Storeの論理管理契約を採用方針とするが、TPM／TEE／software保護の実装とLinux adapterは未選定。旧署名・credential・trust資産は用途限定で、Export鍵契約をhibernate／device pairingへ無条件転用しない。
- **実装・target受入:** 旧boot／package検証資産あり。製品鍵運用・全OS秘密保管・Linux／実機の鍵保護は未確認。
- **依存・責任候補:** 7, 8, 11, 12, 16, 31, 32。security／identity／update候補。
- **根拠:** [boot trust][BOOT]、[0.2 §16・46][S02]、[Connectivity DEC-066][P66]／[Sleep DEC-068][P68]（PR）。
- **次に決める:** 確定したC案に従い、鍵の用途別所有・鍵階層、ハードウェア／ソフトウェア保管と回復可能範囲・全喪失時挙動、Secrets APIの認可・ローテーション／失効、Linux targetでの秘密露出／アクセス拒否受入を具体化する。

<a id="f11"></a>
### 11 システム信頼基盤

- **検討対象:** boot／system／app署名、trust anchors、Owner／Developer Mode、失効・rollback・鍵更新。
- **製品決定:** CONFIRMED＝Owner選択と認可境界を維持 [DEC-017][D17]／[020][D20]、Linux移行で緩和しない [DEC-043][D43]、B案＝対応機器はハードウェア信頼機構を活用し、非対応機器も署名検証する段階的信頼基盤、限定Developer Mode／Owner認可、製品・開発署名鍵の分離と更新／失効・安全な復旧を設計対象 [DEC-087](DEC-087-tiered-system-trust-and-signature-verification.md)。PROVISIONAL＝具体Secure Boot／Trust Anchor・Rollback・機器／版。
- **技術設計:** 旧boot trust／署名slot／package chain設計あり。ハードウェア信頼の起点とソフトウェア署名検証を段階的に組み合わせる方向。特定のLinux Secure Boot／鍵／署名の採用は未決定。署名検証、暗号化、データ認可を混同しない。
- **実装・target受入:** 旧基盤の検証経路あり。production鍵、鍵保管、rollback counter等の残件を保持。Linux trust chain・実機未確認。
- **依存・責任候補:** 1, 2, 3, 8, 10, 31, 35。security／boot／update候補。
- **根拠:** [boot trustと残件][BOOT]、[0.1 §21・24・56–58][S01]、[0.2 §16][S02]。
- **次に決める:** #11のB案に沿ったThreat Model、機器別Trust Anchor／検証チェーン、開発鍵／製品鍵の具体運用・失効、rollback防止・失敗Recovery、Developer Mode／Owner overrideの条件とLinux target受入。

<a id="f12"></a>
### 12 Storage / Filesystem

- **検討対象:** FS選択、block層、data integrity、暗号化、容量不足、Resource IDとpath／inodeの関係。
- **製品決定:** CONFIRMED＝作業・データ中心、分散正本 [DEC-023][D23]／[025][D25]、B案＝System／User Data／Model Store／一時データの用途別にLinux標準Filesystemを選択活用、Nagi独自Storage／Files APIと論理Resource IDを維持し、Filesystem snapshotとWayback／Backup／Workspace復元を分離 [DEC-088](DEC-088-linux-filesystem-nagi-storage-api.md)。PROVISIONAL＝具体FS（EROFS／ext4／Btrfs等は候補）・mount／暗号化・永続化／ID対応・Linux受入／提供版。
- **技術設計:** Linux標準Filesystemを用途別に選定し、Nagi Storage／Files契約・stable Object／Resource IDと実際のpath／inodeを分ける方針。旧ext2／VFS／stable identity契約を参照するが、ext2採用実績をLinux製品FS決定にはしない。
- **実装・target受入:** `user/libnagi/src/storage.rs`等の旧storage資産あり。公開statusにstorage失敗診断もあり、Linux耐久性・実media未確認。
- **依存・責任候補:** 1, 3, 8, 10, 13, 14, 15。`os-core`／Files・storage候補。
- **根拠:** [0.1 §17・18][S01]、[0.2 §23・24][S02]、[implementation status][STATUS]。
- **次に決める:** #12のB案に従うSystem／User Data／Model Store／Tempの具体Linux FS選定、flush／atomicity・容量不足／電源断時の結果契約、stable Resource IDとpath／inode対応・暗号化、fault injection・Linux target受入。

<a id="f13"></a>
### 13 Volume / Mount

- **検討対象:** volume発見・識別、mount namespace、removable media、権限、safe eject、切断・故障。
- **製品決定:** CONFIRMED＝既存Permission／所有境界の維持 [DEC-020][D20]、B案＝Nagi Volume ManagerがVolume接続の検出・論理識別・権限・安全な取り外し・切断時状態を統括し、Linux block／mount／hotplugを利用 [DEC-089](DEC-089-nagi-volume-manager-linux-mount-hotplug.md)。PROVISIONAL＝対応Volume／FS・自動mount条件・Volume ID／eject手順・Linux受入／版。
- **技術設計:** Linux block／mount／hotplugとNagiの認可済みVolume Manager／Files APIを分離。旧storage layoutはあるが、Linux汎用mount／hotplug／外部mediaの包括設計として扱わない。
- **実装・target受入:** 旧固定storage初期化あり。removable・multi-volume・Linux mount／safe ejectの受入未確認。
- **依存・責任候補:** 8, 12, 14, 17, 22。storage／Device Manager候補。
- **根拠:** [0.1 §16–18][S01]、[0.2 §10・23][S02]、[影響表][IMPACT]。
- **次に決める:** 最初のVolume種別・対応FS、自動mount／許可の条件、Volume IDとNagi Resource IDの関係、safe ejectのflush／使用中handle・unmountと突然切断／再接続時の結果不明・拒否試験。

<a id="f14"></a>
### 14 Data Persistence

- **検討対象:** user／profile／app／Workspace／Task／Memoryの正本、schema migration、transaction、flushと保持・削除。
- **製品決定:** CONFIRMED＝model-independent Canonical Memory [DEC-013][D13]、Workspace状態・本文・Taskの分離 [DEC-014][D14]、復旧時再評価 [DEC-022][D22]、B案＝Nagi共通Persistence契約と用途／App／Provider別の分散正本、保存完了／失敗／部分成功／結果不明の区別、Version／安全なSchema Migration、User／Profile所有・忘却とCanonical Memory／派生Index分離 [DEC-090](DEC-090-common-persistence-contract-distributed-authority.md)。PROVISIONAL＝DB／形式・durable commit詳細・移行方式／保持数値・Linux受入／版。
- **技術設計:** versioned IDs／Revision／所有／永続化契約と複数のportable coreを継承し、保存／移行／復旧の共通契約と元アプリ・Provider別正本を分離する。全データ共通DBの強制採用はしない。具体DB・journal／transaction・Linux durabilityは未決。
- **実装・target受入:** host永続化モデル・旧VFS／app保存あり。全app・Linuxでのcrash durability・migration成立は未確認。
- **依存・責任候補:** 7, 8, 10, 12, 15, 26, 27。storage／`platform-apis`／app各owner。
- **根拠:** [0.2 §23–24・45][S02]、[純正仕様 §87–92][FP]、[status][STATUS]。
- **次に決める:** #14 B案に沿う用途／App／Providerごとの保存正本一覧、durable commitのflush／transaction保証・結果不明照合、schema互換／失敗Migration・忘却済みMemoryの再導入防止、Linux target受入。

<a id="f15"></a>
### 15 Crash Recovery

- **検討対象:** crash／電源断／service restart、journal／checkpoint、結果不明、二重実行防止、回復範囲。
- **製品決定:** CONFIRMED＝状態保持・再開／やり直し／中止の選択、無断再実行禁止 [DEC-022][D22]、Workspace保護 [DEC-014][D14]、B案＝Nagi共通Recovery Coordinatorによる整合性／保存証拠・復旧状態の管理＋Service別正本の復旧、結果不明Actionの無断再実行禁止・現行権限／Revision再検証・通常Sleepと障害復旧の区別 [DEC-091](DEC-091-nagi-recovery-coordinator-service-owned-recovery.md)。PROVISIONAL＝形式・Journal／Checkpoint・状態機械・Linux受入／版。
- **技術設計:** Nagi Recovery Coordinatorと各Serviceの所有データ別の責務を分離。Wayback／Activity／Revision／idempotencyの既存契約を維持し、System rollback・障害後resume・通常sleep復帰を分ける。
- **実装・target受入:** 旧VFS／Search回復資産と残件あり。[PR #42][PR42]／[#43][PR43]は未マージ。Linux power-loss・結果不明照合・実mediaは未確認。
- **依存・責任候補:** 4, 9, 12, 14, 26, 29, 31。`wayback-activity-ledger`／Files／Jobs候補。
- **根拠:** [0.2 §14・24・46][S02]、[純正仕様 §90][FP]、[status][STATUS]、[Sleep PR][PR129]。
- **次に決める:** Recovery Coordinatorの配置とdurable state machine、Journal／Checkpoint・処理途中の切断点、結果不明の照合／安全な再試行・復旧不能表示とLinux targetでのAcceptance。

<a id="f16"></a>
### 16 Backup / Restore

- **検討対象:** 本文／設定／Profile／Memory／app／Workspaceのbackup境界、復元・鍵・忘却・世代・外部保存。
- **製品決定:** CONFIRMED＝B方式（ユーザー環境全体のバックアップ）を基本にWaybackと分担・連携、必要時のC方式を追加選択肢とする方向 [DEC-105](DEC-105-user-environment-backup-and-wayback.md)。既存Workspaceの暗号化Export／Importと選択式移行 [DEC-014][D14]を維持。PROVISIONAL／OPEN＝対象別収録・除外・復元契約、Cの具体内容・条件、技術・提供版。Workspace保存を本文backupとしない。
- **技術設計:** 対象別の保存・回復契約あり。全体snapshot／差分backup／復元媒体は未採用。
- **実装・target受入:** 既存checkpoint・hostモデルは再利用候補。全OS backup／bare-metal restore／Linux鍵回復は未確認。
- **依存・責任候補:** 7, 8, 10, 12, 14, 15, 32。storage／Workspace／security候補。
- **根拠:** [DEC-013][D13]／[014][D14]、[0.2 §24][S02]、[純正仕様 §88–92][FP]。
- **次に決める:** Bの具体的なbackup対象と非対象、Wayback連携、Cの内容・適用条件、削除／忘却済み情報の復活防止、鍵喪失・部分復元・世代保持の境界。

<a id="f17"></a>
### 17 Device Manager

- **検討対象:** USB／Bluetooth／Wi-Fi／周辺機器の列挙・hotplug・driver／firmware・権限・診断、信頼端末管理との境界。
- **製品決定:** CONFIRMED＝ローカル機器管理と他のNagi端末管理を内部責務として分離、Nagi Settings／Devices UIは統合、Linux既存機器基盤を活用しNagi側で機器状態・Permission・Eventを管理する方式B [DEC-106](DEC-106-device-manager-local-remote-unified-ui.md)。純正Devices／Trust [純正仕様 §65][FP]、通常GUI／非特権AI [DEC-009][D09]を継承。OPEN＝具体stack・機器／bus・実装方式。全機器対応は未保証。
- **技術設計:** Linux側のローカル機器認識と他のNagi端末のTrust（untrusted／pending／trusted／revoked）を内部で区別し、管理UIに統合。udev等のstackや詳細なhotplug状態機械は未選定。
- **実装・target受入:** 旧PCI／VirtIO等のdriver資産あり。統合ローカルdevice service・USB／BT／Wi-FiのLinux実機受入未確認。
- **依存・責任候補:** 1, 4, 8, 9, 13, 18–22, 29, 33。device／`os-core`／Devices app候補。
- **根拠:** [0.1 §16][S01]、[純正仕様 §27–30・65][FP]、[Connectivity][P66]／[Sleep][P68]（PR）。
- **次に決める:** 最初のbus・周辺機器、driver・firmwareの選択、hotplug・切断・取消の状態機械、UIとLinux実機受入。

<a id="f18"></a>
### 18 Network Devices

- **検討対象:** Ethernet／Wi-Fi／Bluetooth controller、firmware、認証情報、radio状態、再接続・省電力。
- **製品決定:** CONFIRMED＝Ethernet・Wi-FiをNagi OSの基本機能、Bluetoothは段階的対応とする方式B [DEC-107](DEC-107-network-devices-ethernet-wifi-bluetooth-staged.md)。端末間連携は初期LANターゲット [DEC-066][P66]を維持。OPEN＝Wi-Fi／BT stack・対応chipset・pairing／対象用途・適用版。全機器対応保証ではない。
- **技術設計:** 旧VirtIO Networkと共通network契約は再利用候補。Linux既存driver活用とNagi管理層／Permission境界（DEC-106）を基本とし、無線制御・Bluetooth pairing・再接続の具体方式は未選定。
- **実装・target受入:** 旧仮想NIC資産あり。Wi-Fi／BT実機、firmware配布、sleep後再接続は未確認。
- **依存・責任候補:** 1, 8, 10, 17, 23, 24, 32, 33。device／network候補。
- **根拠:** [0.1 §16・35][S01]、[0.2 §5.2][S02]、[PR #127][PR127]。
- **次に決める:** 初期対象NIC／Wi-Fi chipset、credential／firmwareと接続許可、Bluetooth最初の用途、接続不能・sleep復帰後の再評価とLinux実機受入。

<a id="f19"></a>
### 19 Display / GPU

- **検討対象:** modesetting／compositor／renderer、multi-display／hotplug、GPU隔離、fallback、描画・compute資源。
- **製品決定:** CONFIRMED＝Nagi Home／Desktop・Workspace・ウィンドウ管理方針・端末適応UIはNagi独自、Linux／OSSの描画・GPU・Display基盤は選択的に活用するB改良案 [DEC-108](DEC-108-display-gpu-nagi-shell-oss-graphics.md)。PC／Tablet／Mobileの共通契約を保持し端末別backend差を吸収。GUI維持、Workspaceの複数画面切替・指定画面維持 [DEC-009][D09]を継承。OPEN＝Wayland／Smithay等の具体採否・Linux display backend・対応hardware／版。
- **技術設計:** 上位Shell／Workspace／端末適応をNagi設計とし、画面合成やGPU／Displayの基礎層ではLinux／OSS資産を評価・再利用する。旧compositor／VirtIO GPU／Mesa／Servo契約は参照資産。DRM／KMS／Wayland／Smithay等の採否、CPU描画fallback・rendererは未決定。
- **実装・target受入:** 旧guest描画・desktop資産あり。Linux compositor接続・実GPU・複数実画面は未確認。
- **依存・責任候補:** 1, 5, 8, 17, 20, 25, 33。desktop／graphics／`ui-design-system`候補。
- **根拠:** [0.1 §25–31][S01]、[純正仕様 §27–30][FP]、[影響表][IMPACT]。
- **次に決める:** OSS描画基盤／Compositorの候補比較、最小display経路とCPU描画退避、surface／buffer権限、multi-display・hotplug・端末別backendのLinux実機受入。

<a id="f20"></a>
### 20 Input System

- **検討対象:** keyboard／mouse／touch／IME、focus・shortcut、入力応答、権限・trusted input、切断。
- **製品決定:** CONFIRMED＝Linux既存入力基盤を活用し、Keyboard／Mouse／TouchをNagi Input Service／共通入力契約で扱う方式B [DEC-109](DEC-109-input-system-cross-device-linux-foundation.md)。PC／Tablet／Mobileの適応入力を目標とし、GUI／keyboard／mouse継承 [DEC-009][D09]、IME入力応答優先・AI学習安全境界 [DEC-060](DEC-060-responsive-ime-ai-learning.md)を維持。OPEN＝Linux input stack・IME／seat／focus・機器別対応・適用版。
- **技術設計:** 低レイヤーは適切なLinux driver／OSSを再利用し、Nagi共通Input Serviceで端末別入力を統合。旧input／composition／localization資産あり。具体evdev／libinput／seat／focus／shortcut／IME接続は未選定。入力イベント取得と権限付きAction実行は別契約。
- **実装・target受入:** 旧guest input／IME経路あり。全日本語IME完成・touch・Linux実機入力を既存資産から推定しない。
- **依存・責任候補:** 7, 8, 17, 19, 28。input／IME／desktop／localization候補。
- **根拠:** [0.1 §26・32][S01]、[純正仕様 §78][FP]、[language architecture][LANG]、[status][STATUS]。
- **次に決める:** Linux input backend、device→seat→focus→appの認可・入力契約、IME composition／秘密入力・shortcutの境界、touch・software keyboard等の初期範囲、遅延と実機受入。

<a id="f21"></a>
### 21 Audio System

- **検討対象:** playback／capture／mixing／routing、device切替、latency／underrun、mic権限・音声優先度。
- **製品決定:** CONFIRMED＝Linux／OSS音声基盤を活用し、Nagi Audio Serviceで再生／録音・mixing／routingの統合管理、機器切替・優先度・マイク権限・Nagi Voice連携を担う条件付きB [DEC-110](DEC-110-audio-system-oss-nagi-service-measured-optimization.md)。低遅延・省メモリ・省電力を実機測定し、要件未達の部分だけ独自最適化／部分置換を検討。AIなしの通常操作・非特権AI [DEC-009][D09]／[020][D20]を維持。OPEN＝具体backend・機器・指標・適用版。
- **技術設計:** Linux audio driverやALSA／PipeWire等の既存OSSは候補であり、個別stack・採用方式は未決定。Nagi独自Audio ServiceでOS側のPolicy／Permission・アプリ再生／録音・Voice経路を管理する方向。旧VirtIO Sound／audio serviceとvoiceの分離契約は既存資産。
- **実装・target受入:** 旧audio／voice資産あり。Linux imageでの継続再生・mic認可・実codec／BT audio未確認。
- **依存・責任候補:** 8, 9, 17, 18, 23, 25, 33。audio／voice／device候補。
- **根拠:** [0.1 §37–38][S01]、[0.2 §47–48][S02]、[0.3 roadmap][R03]。
- **次に決める:** 最小再生／録音・mixing・routing、マイク認可・Voice優先度、device切替とsleep復帰、Linux／実機での遅延・RAM・電力・underrun測定と基準、OSS未達時の対象限定最適化。

<a id="f22"></a>
### 22 周辺機器

- **検討対象:** printer／scanner／camera／USB storage／HID等の対象、driver／firmware、protocolと権限・UI。
- **製品決定:** CONFIRMED＝USBストレージ（USBメモリ・外付SSD）と基本HIDを標準対象、カメラ・プリンター・スキャナーは段階対応とする方式B [DEC-092](DEC-092-peripherals-usb-storage-hid-staged-support.md)。Linux既存driver／OSSを活用しNagi Device Manager／Permission／Privacy／通常UIと連携。全Linux対応機器の動作保証ではない。PROVISIONAL＝具体機種・対応範囲・API・protocol・実装版・実機受入。
- **技術設計:** DEC-106のローカル機器管理とDEC-109の共通Input契約に接続する。機器認識と実際の利用許可を区別し、DEC-107のBluetooth段階対応を維持。個別stack・driver／APIは未選定。
- **実装・target受入:** 旧参照VM向けdevice経路以外の包括対応を確認していない。実printer／camera／USB等は未確認。方針記録をLinux実機受入済みとしない。
- **依存・責任候補:** 8, 13, 17, 18, 20, 21, 33。device／各機能owner候補。
- **根拠:** [DEC-092](DEC-092-peripherals-usb-storage-hid-staged-support.md)、[0.1 §16][S01]、[0.2 §5.1–5.2][S02]、[実機候補DEC-052][D52]。
- **次に決める:** 標準対象内の具体機種・対応機能と認識／閲覧／操作の段階、driver／protocol、切断・失敗時UIと機密データ・権限境界、実機受入条件。

<a id="f23"></a>
### 23 Nagi Power

- **検討対象:** 電池／AC、thermal／CPU／GPU負荷、power profile、充電制御候補、signal欠落時の挙動。
- **製品決定:** CONFIRMED（main）＝前景優先・AIの通常資源契約 [DEC-020][D20]。CONFIRMED（製品方針）＝[DEC-067][P67]の決定論的OS全体管理。PROVISIONAL＝技術・名称正式化・数値・端末。
- **技術設計:** Sleep遷移は6、継続的制御は23、配分は25。cpufreq／thermal／powercap／cgroup／eBPF等は候補、採用済みでない。
- **実装・target受入:** policy記録とMemory Governor案あり。Linux電力制御・実sensor・省電力効果は未確認。
- **依存・責任候補:** 1, 6, 17, 25, 29, 33。Power／resource／Model Runtime候補。
- **根拠:** [PR #128][PR128]、[DEC-013][D13]、[Governor案][GOV]、[0.2 §48][S02]。
- **次に決める:** 観測信号と制御責任、故障・信号遅延時の方針、baseline・測定条件と後続合格基準。

<a id="f24"></a>
### 24 Network Service

- **検討対象:** IP／DNS／DHCP／routing／TLS／proxy、offline表示、接続権限と宛先、service API。
- **製品決定:** CONFIRMED＝offline-firstと外部送信Scope [DEC-002][D02]、正規Service経路 [DEC-020][D20]。OPEN＝Linux network管理構成。
- **技術設計:** 旧`nagi-net`／TLS／socket broker資産あり。network serviceとradio device管理18、端末信頼32を分離する。
- **実装・target受入:** 旧guest net／browser接続資産あり。Linux imageのDNS／TLS／権限拒否・復帰後再接続は未確認。
- **依存・責任候補:** 7, 8, 9, 10, 18, 29。network／`platform-apis`候補。
- **根拠:** [0.1 §35–36][S01]、[0.2 §10・11][S02]、[Connectivity PR][PR127]。
- **次に決める:** service APIとLinux backend、宛先／profile別認可、offline・timeout・certificate更新の責任。

<a id="f25"></a>
### 25 Resource Manager / Scheduler

- **検討対象:** CPU／RAM／GPU／NPU／I/O／電力budget、OOM、fairness、queue／throttle／pause／cancel。
- **製品決定:** CONFIRMED＝前景応答性、明示優先変更、安全限界とResource待機の条件付き継続 [DEC-020][D20]、適応型Memory [DEC-013][D13]。PROVISIONAL＝閾値・algorithms。
- **技術設計:** OS scheduler、Jobs、Model Runtime、Memory検索選択は責務が異なる。独立AI schedulerの重複採用を避ける。Linux強制手段は未定。
- **実装・target受入:** 旧scheduler／OOM・host Jobs／Model資産あり。Linux統合の応答性・飢餓・負荷・電力測定未確認。
- **依存・責任候補:** 1, 4, 8, 14, 23, 26, 29。`job-01`／`model-runtime`／`os-core`候補。
- **根拠:** [0.1 §13・15・43–45][S01]、[0.2 §14・47–48][S02]、[Governor案][GOV]、[Power PR][PR128]。
- **次に決める:** 予算所有者・要求API・強制と観測、Pause／Cancel／crashと資源待機の状態分離、測定baseline。

<a id="f26"></a>
### 26 App Lifecycle

- **検討対象:** manifest／package／launch／ready／stop／crash／restart／upgrade／uninstall、session・data・background jobs。
- **製品決定:** CONFIRMED＝Nagi Native Apps、Public API共通・段階統合 [DEC-017][D17]、builder更新・導入・削除の境界（関連DEC参照）。PROVISIONAL＝実装・版。
- **技術設計:** 0.2 M12 manifest契約とM13 Supervisorを分離。package検証成功を安全なruntime起動としない。
- **実装・target受入:** app-sdk／manifest host foundationあり。supervisor runtimeは別状態。Linux process／sandbox／lifecycle統合は未確認。
- **依存・責任候補:** 4, 5, 7, 8, 9, 14, 25, 31。`app-sdk-contract`／`app-lifecycle-manifest`／`app-lifecycle-supervisor`候補。
- **根拠:** [0.2 §13・16・26・M12/M13/M21][S02]、[registry][REG]、[純正仕様][FP]。
- **次に決める:** processとlogical app identity、ready／crash／bounded restart、更新時のdata／Permission再検証と実sample受入。

<a id="f27"></a>
### 27 Settings / Notification

- **検討対象:** system／user／profile／app設定、schema／migration、購読、通知の帰属・quiet／focus・redaction・action。
- **製品決定:** CONFIRMED＝非特権AI・正規API [DEC-020][D20]。OPEN＝Linux service接続・設定UI詳細。具体既定値は一括決定しない。
- **技術設計:** 0.2 scoped Settings、通知記述子は権限を持たずactivation時再検証、bounded persistence契約あり。
- **実装・target受入:** notification core等のhost foundationあり。Settings runtimeは仕様snapshotでNOT_STARTED、現行stateを都度確認。Linux通知表示・設定永続化は未確認。
- **依存・責任候補:** 7, 8, 9, 14, 26, 28。`settings-configuration`／`notify-01`／desktop候補。
- **根拠:** [0.2 §12・15・M11/M14][S02]、[registry][REG]、[notification core](../../crates/nagi-notification-core/README.md)。
- **次に決める:** scopeの上書き規則と変更認可、通知UI／lockscreen秘匿、action実行時の権限境界。

<a id="f28"></a>
### 28 Time / Locale / Accessibility

- **検討対象:** monotonic／wall clock／timezone／時刻同期、locale、入力言語、a11y tree／keyboard／screen reader等。
- **製品決定:** CONFIRMED＝GUI利用維持・限定Accessibility fallback [DEC-009][D09]／[027][D27]。既存契約＝英語内部識別子、en-US／ja-JP同格。OPEN＝時刻同期・a11y機能範囲と方式。
- **技術設計:** language architectureとlocalization契約あり。locale／input／AI会話言語を分離。期限認可を巻き戻せるwall clockだけに依存させない設計が必要。
- **実装・target受入:** localization資産・host契約あり。Linux clock／RTC／timezone、実screen reader、全UI a11y受入は未確認。
- **依存・責任候補:** 7, 9, 14, 19, 20, 24, 27。`localization-i18n`／UI／clock／a11y候補。
- **根拠:** [language architecture][LANG]、[0.2 §14・22・M06][S02]、[純正仕様 §98][FP]、[IME PR][PR120]。
- **次に決める:** time authorityとoffline時刻ずれ、timezone保存、a11yの最初の操作・読み上げ・権限契約。

<a id="f29"></a>
### 29 Diagnostics / Logging

- **検討対象:** boot／crash／service health、structured log／trace／correlation、redaction・保持・export・測定負荷。
- **製品決定:** CONFIRMED＝結果の正直な表示・検証可能性 [DEC-022][D22]／[026][D26]。OPEN＝Linux収集backend・運用保持値。
- **技術設計:** 0.2診断・fingerprint契約あり。debug logとActivity、privileged diagnosticsと通常閲覧を区別する。
- **実装・target受入:** 旧serial／crash診断とhost構造化契約あり。Linux image crash収集・実機・秘密除去の統合受入は未確認。
- **依存・責任候補:** 4, 7, 8, 9, 14, 35。`diagnostics`／`ci-acceptance`候補。
- **根拠:** [0.1 §66–69][S01]、[0.2 §17・M04][S02]、[status][STATUS]、[registry][REG]。
- **次に決める:** 最小boot／service証拠、sink failureと容量境界、誰が何をexportできるか・redaction検証。

<a id="f30"></a>
### 30 Installer / First Setup

- **検討対象:** boot媒体、disk選択・破壊操作、partition／暗号化、Owner初期化、locale、offline setup、recovery。
- **製品決定:** CONFIRMED＝offline-first、Ownerと通常操作の維持 [DEC-009][D09]／[017][D17]。OPEN＝製品installer・初回設定フローと対応媒体。
- **技術設計:** image builder／local identity／package installはあるが、OS installerとは別の機能。
- **実装・target受入:** `tools/nagi-installer/`のhost契約と旧guest owner bootstrap資産あり。host installer契約を物理diskへのinstallとしない。Linux disk installation・初回設定・実機復旧媒体は未確認。
- **依存・責任候補:** 2, 3, 7, 10, 12, 13, 28, 31, 33。installer／identity／storage候補。
- **根拠:** [0.1 §22・79–80][S01]、[0.2 §9・16][S02]、[boot trust][BOOT]。
- **次に決める:** 最初のinstall target、データ破壊前確認・暗号化鍵回復、offlineで完了する最小setup。

<a id="f31"></a>
### 31 System Update

- **検討対象:** system／app／model更新の分離、署名・互換、原子性、A/B／rollback、停止・再起動・data migration。
- **製品決定:** CONFIRMED＝builderアプリ更新は検証・確認後の正式更新、data保護 [DEC-035][D35]／[017][D17]。OPEN＝Linux system update方式・配信運用。
- **技術設計:** 旧A/B・trustと0.2 install/update契約あり。system updateとapp更新は共通primitiveでもpolicy domainが異なる。
- **実装・target受入:** 旧slot／boot検証・host package planning等あり。network配信・製品鍵・update UI等の残件あり。Linux image更新・電源断復旧未確認。
- **依存・責任候補:** 2, 3, 10, 11, 12, 14, 15, 26, 35。`update-installation`／boot／release候補。
- **根拠:** [boot trust][BOOT]、[0.1 §21・56–58][S01]、[0.2 §16・M15][S02]。
- **次に決める:** image／package更新単位、互換とrollback不能なdata変換、信頼鍵・配信・中断時のdurable commit条件。

<a id="f32"></a>
### 32 Device Connectivity

- **検討対象:** discovery／Trust／Transport、pairing・暗号化・revocation、Share／Handoff、offline・競合・将来remote。
- **製品決定:** CONFIRMED（main）＝選択式暗号化Workspace Export／Import [DEC-014][D14]。CONFIRMED（製品方針）＝Share [064][P64]／Handoff [065][P65]／初期LANと共通抽象 [066][P66]。PROVISIONAL＝protocol・名称・版・対象。
- **技術設計:** 既存Device／Surface／Continuity契約を再利用。Share≠Handoff≠継続同期、trusted≠data操作許可。Clipboard同期は未採用。
- **実装・target受入:** pure contract／appモデルは再利用候補。実端末発見・暗号化転送・Handoff／remote通信のLinux受入未確認。
- **依存・責任候補:** 7, 8, 9, 10, 14, 17, 18, 24, 26。Devices／Connectivity／Workspace候補。
- **根拠:** [純正仕様 §27–30・54・65・71・97][FP]、[PR #125][PR125]／[#126][PR126]／[#127][PR127]。
- **次に決める:** LAN上の最小pairing／transport契約、実体不足・失効・取消・重複／中断の扱い。Cloud／relayの採否はOPEN。

<a id="f33"></a>
### 33 Hardware Support

- **検討対象:** architecture／VM／実機、firmware／driver matrix、必須・任意機能、候補と認定、試験環境。
- **製品決定:** CONFIRMED＝[DEC-052][D52]の候補選定のみ。PROVISIONAL＝正式対応・時期・版。OPEN＝恒久Linux reference target・必須device matrix。
- **技術設計:** 既存QEMU x86-64参照仕様、0.2最初の実機参照機を選定したx86-64 UEFI mini-PCとする方針、generic ARM64→Piの境界を保持。別軸の将来候補はRaspberry Pi型番未指定、PineTab2、phone未選定。
- **実装・target受入:** 旧QEMU実績あり。[PR #57][PR57]のmultiarch spikeは未マージ。Linux実機boot／GPU／radio／sleepは未確認。
- **依存・責任候補:** 1, 2, 17–23, 34, 35。`os-core`／porting／`ci-acceptance`候補。
- **根拠:** [0.1 §2][S01]、[0.2 §5][S02]、[実機候補][D52]、[porting PR][PR57]。
- **次に決める:** 開発targetと製品対応保証の区別、最小HW matrix、VM・実機ごとの受入と未対応表示。

<a id="f34"></a>
### 34 Testing / Release

- **検討対象:** host／adapter／image／実機の試験階層、fault／negative／security、性能測定、release gateと証拠。
- **製品決定:** CONFIRMED＝既存Gate維持 [DEC-042][D42]／[044][D44]／[047][D47]、実行していない試験を通ったとしない [DEC-049][D49]。OPEN＝Linux版追加Acceptance。
- **技術設計:** 0.1 M30、0.2 activation／M21、0.3仕様化・activation、0.4 Entry Gateを保持。全35項目完了を新Gateにしない。
- **実装・target受入:** 既存CI／QEMU試験あり。`test-plat-01`の新platform harnessはstateでIN_PROGRESS、実装・Acceptance成功は未記録。旧statusはM19以降等にPARTIALを含む。CI単発成功をrelease全体PASSにしない。Linux release・実機未確認。
- **依存・責任候補:** 29, 33, 35と各対象項目。`ci-acceptance`／`test-plat-01`／Integration Owner。
- **根拠:** [0.1 §75–92][S01]、[0.2 §31–32・46–48][S02]、[roadmaps][RM]、[status][STATUS]。
- **次に決める:** Linuxの最小統合シナリオと失敗ケース、既存Gateとの差分・証拠形式・最終受入責任。

<a id="f35"></a>
### 35 Build / Supply Chain

- **検討対象:** toolchain／source pin／patch、reproducibility、provenance／SBOM／NOTICE、artifact配布・署名・再利用。
- **製品決定:** CONFIRMED＝OSS選択的利用可 [DEC-043][D43]、公開正本 [DEC-050][D50]。OPEN＝Linux依存構成と配布条件の個別判断。
- **技術設計:** 固定revision・patchと0.2 fingerprint／SBOM契約あり。SBOM出力はlicense適法性の自動認定ではない。
- **実装・target受入:** `sources.lock`、provenance／release tools等のhost資産あり。[PR #99][PR99]のcache修正は未マージ。Linux kernel／rootfs／moduleを含む再現build・配布受入未確認。
- **依存・責任候補:** 1, 3, 5, 10, 11, 31, 34。`developer-tooling`／`build-provenance`／`license-sbom`／release候補。
- **根拠:** [0.1 §71–80][S01]、[0.2 §18–19・M02/M03][S02]、[registry][REG]、[PR #99][PR99]。
- **次に決める:** Linux入力と許可環境の固定、再利用可否・artifact照合、license不明／未署名／改ざん時の明示的扱い。

## 4. 7つの今後の検討束

**REVIEW NOTE / PROVISIONAL。** 元の7名称を保持し、35項目を漏れなく主担当束へ対応付けた整理案。括弧内は横断依存。所有権・実装順序の確定ではない。

| 束 | 名称 | 主に扱うID | 主な依存・接続先 |
|---|---|---|---|
| B1 | Boot/Init/System Image | 1, 2, 3, 4, 5 | B2の権限・信頼、B3のstorage、B7のbuild／診断 |
| B2 | Security/Identity/IPC | 7, 8, 9, 10, 11 | B1の実行境界、B3の鍵・保存、全serviceの拒否試験 |
| B3 | Storage/Persistence/Recovery | 12, 13, 14, 15, 16 | B2の所有・鍵、B6の更新、B5のSleep |
| B4 | Device/Display/Input/Network | 17, 18, 19, 20, 21, 22, 24, 33 | B1のdriver、B2の認可、B6のConnectivity |
| B5 | Power/Sleep/Resource | 6, 23, 25 | B3の保存、B4の復帰・sensor、B6のapp／Jobs |
| B6 | App Runtime/Package/Update | 26, 31, 32 | B2のmanifest／権限・IPC、B3のdata、B4のnetwork |
| B7 | Installer/Diagnostics/Release | 27, 28, 29, 30, 34, 35 | 全束の証拠。Settings／Localeは全サービスへ横断 |

### 最初に扱う5トピックの提案

以下は会話・設計の着手順案であり、全体設計の完成待ちや既存開発停止を要求しない。可逆的な技術選択はDEC-044の委任範囲で進められる。

| 順案 | トピック | 対象ID | 先に揃えるもの／次へ渡すもの |
|---|---|---|---|
| A | 最小Linux boot → userland → serviceと固定build入力 | 1–5, 33, 35 | 既存移行担当・開発targetを確認 → 実際に起動できる最小構成。恒久採用とは分離 |
| B | Identity／Permission／IPCのLinux強制境界 | 7–11 | Aと並行してthreat model → caller binding・迂回拒否・失効の契約 |
| C | 1つの通常appの保存 → 再起動 → 再読込と失敗回復 | 12, 14, 15, 26 | Aのservice、Bの所有・認可 → Linux同一本体のdurabilityと結果不明処理 |
| D | Device／Display／Inputの最小経路とnetwork境界 | 17–20, 24, 33 | Aのdriver、Bのdevice認可 → 一つのtargetで通常GUI操作と接続状態を確認 |
| E | Power／Sleep／Resourceの責務と状態機械 | 6, 23, 25 | Cの保存・復旧、Dの再接続、Bのロック → resource待機と停止／障害復旧を分離 |

Diagnostics／Acceptanceの設計（29, 34）はAから各トピックに組み込み、最後まで後回しにしない。Device Managerの具体化はDで進め、先行するA／Bとのinterfaceを明らかにする。

## 5. 進捗更新ルール

1. **正本を先に更新する。** 新しいユーザー決定は対応DECとREADMEへ。技術変更はADR／実装仕様へ。実装・担当は現行workstream／registryへ。台帳はその内容と証拠を索引する。
2. **項目を丸ごと昇格させない。** CONFIRMEDの範囲、暫定案、OPENを別記。P0等を確定へ変えるには明示的な製品判断を記録する。
3. **証拠を限定する。** 対象ID／commit／環境／command・結果／既存Acceptanceを1–3行と公開可能な参照先で示す（DEC-049）。host-only、旧guest、Linux adapter、image、実機を区別。未実行・未測定・失敗・不明を残す。
4. **進捗の根拠を保持する。** workstreamの古いstate・存在しないstate path・仕様内のsnapshotだけで最新実装を断定しない。branch／PR未マージなら明記。merge後にmain SHAと対応箇所を確認して参照を更新する。
5. **変更時だけ再検証する。** 対象コードが不変なら、環境・限界付きの過去証拠を使える。新しい変更・失敗・懸念に応じた試験を行い、台帳編集だけでruntime test実行を主張しない。
6. **担当は確認してから記入する。** 候補と割当済みownerを分け、既存allowed／shared paths・Integration Ownerを尊重する。台帳の依存図で新しい承認待ちやrelease gateを作らない。
7. **履歴と公開状態を更新する。** 下表へ日付・ID・根拠・何が変わったかを追記し、公開commit／PRを検証する。秘密情報・個人情報・ユーザー名入りローカルpath・会話識別子は保存しない。

### 項目更新用テンプレート

```text
ID／名称／更新日:
推奨優先度: P0/P1/P2（REVIEW NOTE / PROVISIONAL。確定時は決定DEC）
検討対象・決めるべき仕様:
製品決定: CONFIRMED（範囲＋DEC）／PROVISIONAL（案）／OPEN（残る選択）
技術設計: 契約／ADR、旧基盤との差異、開発用選択か恒久採用か
実装: main／branch／PR・commit、portable core／backend／未接続箇所
受入: host／旧guest／Linux adapter／Linux image／実機ごとの結果・未確認
依存・責任候補: ID、interface、既存workstream、候補か割当済みか
関連DEC・実装仕様・ADR／証拠参照:
次に決める事項:
```

## 6. AI実装担当へ渡す実装契約テンプレート

必要な範囲で既存workstreamの記録へ組み込み、別の重複台帳や段階別承認記録を量産しない。全欄を埋めたこと自体は実装許可でも受入でもない。OPENが実装を変える場合は、製品判断が必要な点と委任内で解決できる技術判断を分ける。

```text
契約名／Foundation ID／対象commit・branch:
製品根拠: 決定DECとCONFIRMEDの範囲、残るPROVISIONAL／OPEN
前提: Linux開発target、固定kernel／toolchain／rootfs、既存契約、実行可能な環境
目標: 今回観測可能にする具体動作と入出力／状態遷移
非目標: 対象外の機器・機能・互換性・全面移行・release変更
依存: API／schema／Identity／Permission／storage／service、相手workstream
設計: ADR、型・IDL・version、Linux強制機構、旧実装の再利用／置換箇所
失敗・復旧: 取消・失効・timeout・部分成功・結果不明・二重実行・rollback実在範囲
担当: 既存ownerと割当根拠、branch／allowed paths／共有path調整、統合先、受入責任者
Acceptance:
  - 正常: 操作 → 実際の状態変化 → 再読込／再起動後確認
  - 拒否: 未認証・別profile・失効・Scope外・古いRevision・迂回アクセス
  - 障害: service停止・不正入力・容量不足・I/O失敗・電源断／cancel等、該当範囲
  - 測定: baseline・負荷・計測法・数値が承認済みか未決か（未決値を創作しない）
Linux／実機検証:
  - portable host／Linux同一本体adapter／image内／実機を別結果にする
  - firmware・driver・センサー・電源・周辺機器の対象と未対応
  - 実機が不要な範囲と実機でしか受け入れられない範囲を明記
セキュリティ・データ:
  - threat model、caller認証、権限最小化／取消、鍵・秘密・ログ、data保護と忘却
  - AIは非特権。モデルの判断を認可や受入証拠の代替にしない
証拠: commit＋実行command・結果1–3行、公開可能なlog／CI参照、未実行・限界
完了判断: 対象Acceptanceとの照合、重要残件、次の統合先。release全体PASSは別判断
```

## 7. 根拠一覧と調査の限界

- 製品決定の正本は[README](README.md)と[OPERATING_RULES](OPERATING_RULES.md)。本台帳の新規決定はDEC-069の管理方針だけ。
- 実装仕様：[0.1][S01]（旧kernel本文＋優先注記）、[0.2][S02]（§30はsnapshot）、[Linux移行補足][ADD]、[純正ソフトウェア仕様][FP]。
- 設計／進捗：[移行ADR][ADR-L]、[boot trust][BOOT]、[Linux影響表][IMPACT]、[implementation status][STATUS]、[workstream registry][REG]、[workstream文書][WS]。registry登録は実装完了ではなく、state未掲載なら確認先を残す。
- [0.3 Revised][R03]はArchitecture Draft、[0.4+][R04]はVision proposal、[0.5–1.0][R05]は暫定版計画。各版の既存Gateは維持する。
- 関連公開実装PR：[Files #42][PR42]、[session worker #43][PR43]、[Desktop #48][PR48]、[multiarch #57][PR57]、[host port readiness #91][PR91]、[dependency cache #99][PR99]、[NDP担当・NDP-10承認 #105][PR105]。確認時はOPEN、main実装として混ぜない。#91はhost-only、#105は担当・開始記録であり成果証拠ではない。
- 公開originに`codex/linux*` branchの掲載を確認できなかった。DEC-047の統合先が公開されていないことは、他の許可済み環境で開発されていない証明ではない。私有環境・旧privateの新規情報は調査していない。
- 実装状況の初回評価は公開ソースと既存記録の読取。Linux image、実機、driver、電源、性能の新規測定は未実施。全35項目を一律未着手にも、一律完了にもしていない。

### 2026-11-16 Foundation #12 Storage／Filesystemの用途別Linux採用

[DEC-088](DEC-088-linux-filesystem-nagi-storage-api.md)により、Nagi独自Storage／Files APIと論理Resource／Object／Revision・分散正本を維持し、Linux標準Filesystemを用途別に選択活用するB案をCONFIRMED。Filesystem snapshot／Nagi Wayback／Backup／Workspace復元を混同しない。候補EROFS・ext4・Btrfs等は未採用で、具体FS・暗号化／mount・durability／stable ID対応・Linux target試験・提供版は未確定。#13 Volume／Mountの自動mountや外部媒体対応を今回確定しない。

### 2026-11-16 Foundation #13 Volume／MountのNagi主体管理

[DEC-089](DEC-089-nagi-volume-manager-linux-mount-hotplug.md)で、Linuxのblock／mount／hotplugを活用しNagi Volume Managerが接続検出・論理Volume ID・現行Permission・Files表示・安全な取り外しと突然切断の状態管理を担当するB案をCONFIRMED。自動検出は自動mount／全媒体許可と同義ではない。書込み／unmount失敗を成功表示せず、再接続時に不確かなTask／Actionを勝手に再実行しない。具体初期対応媒体／FS・権限UI・mount／eject方式・Linux実機試験と版は未決定。

### 2026-11-16 Foundation #14 Data Persistenceの共通契約＋分散正本

[DEC-090](DEC-090-common-persistence-contract-distributed-authority.md)で、Nagiが共通の保存完了／Migration／復旧／所有・忘却等のPersistence契約を定めながら、正本をUser／Profile／Memory／Workspace／Task／App／Provider別に保持できるB案をCONFIRMED。Canonical Memoryと索引・Cacheを分離し、Workspace Checkpoint／本文Backup／Task再実行を混同しない。単一DBを強制せず、具体形式・commit／fsync・schema migration・Linux crash durability受入・適用版は未決定。

### 2026-11-16 Foundation #15 Crash Recoveryの統合調整＋Service別復旧

[DEC-091](DEC-091-nagi-recovery-coordinator-service-owned-recovery.md)で、共通Nagi Recovery Coordinatorが障害後の整合性・保存証拠・復旧結果を調整し、各Serviceが自分の保存正本と状態を復旧するB案をCONFIRMED。Workspace／本文Backup／Taskは別、結果不明の副作用Actionと中断Taskを無断再実行せず、User／Profile／Permission／Resource Revisionを再確認。通常Sleep復帰とクラッシュ復旧を区別し、具体Journal／Checkpoint・State Machine・Linux実媒体受入／適用版は未決定。

## 8. 変更履歴

| 日付 | 対象 | 変更・根拠 | 検証範囲 |
|---|---|---|---|
| 2026-11-16 | #15・B3 | [DEC-091](DEC-091-nagi-recovery-coordinator-service-owned-recovery.md) B案確定：Nagi Recovery Coordinator＋Service別安全復旧 | 文書commitのみ。Linux実媒体・電源断／Crash Acceptance未確認 |
| 2026-11-16 | #14・B3 | [DEC-090](DEC-090-common-persistence-contract-distributed-authority.md) B案を確定：共通Persistence契約＋分散正本、Version／復旧・忘却の境界 | 文書commitのみ。Linux crash durability／migration受入未確認 |
| 2026-11-16 | #13・B3 | [DEC-089](DEC-089-nagi-volume-manager-linux-mount-hotplug.md) B案を確定：Linux mount／hotplug＋Nagi Volume Manager、認可・safe eject・切断状態 | 文書commitのみ。Linux target／実媒体受入未実施・未確認 |
| 2026-11-16 | 16 | [DEC-105](DEC-105-user-environment-backup-and-wayback.md)：B基本・Wayback分担連携・必要時C追加の方向を確定。対象・技術・C詳細・版は未定 | 文書・リンク・状態分離を確認。runtime PASSの追加なし |
| 2026-11-16 | 22 | DEC-092で方式BをCONFIRMED。USBストレージ・基本HIDを標準対象、カメラ・プリンター・スキャナーは段階対応。具体機種・範囲・API・protocol・実装版・実機受入は未確定 | 文書・採番・リンクの確認。実装／runtime・実機受入の追加なし |
| 2026-11-16 | 1–35、B1–B7 | DEC-069による初版。35項目・元優先度・6分類群・7検討束を保持。最新mainと公開並行PR、Linux移行差異、候補／host／旧guest／未確認を分類 | 文書・根拠・リンク・番号・機密非掲載の確認。runtime PASSの追加なし |

[D02]: DEC-002-cloud-ai-data-sharing.md
[D09]: DEC-009-nagi-first-home-desktop.md
[D13]: DEC-013-lightweight-memory-engine-benchmark.md
[D14]: DEC-014-workspace-state-checkpoints.md
[D17]: DEC-017-third-party-developer-ecosystem.md
[D20]: DEC-020-ai-os-interaction-path.md
[D22]: DEC-022-builder-interrupted-task-recovery.md
[D23]: DEC-023-work-data-centric-os.md
[D25]: DEC-025-cross-app-data-reference-and-updates.md
[D26]: DEC-026-ai-change-verification-and-recovery.md
[D27]: DEC-027-public-action-and-scoped-gui-fallback.md
[D35]: DEC-035-builder-created-app-maintenance-updates.md
[D42]: DEC-042-linux-kernel-transition.md
[D43]: DEC-043-nagi-userland-independent-os.md
[D44]: DEC-044-linux-migration-development-authorization.md
[D47]: DEC-047-linux-migration-workspaces-and-integration.md
[D49]: DEC-049-development-process-speed-priority.md
[D50]: DEC-050-public-specification-repository.md
[D52]: DEC-052-physical-test-device-candidates.md
[S01]: ../Nagi_OS_0.1_Codex_Implementation_Spec.md
[S02]: ../Nagi_OS_0.2_Codex_Implementation_Spec.md
[ADD]: ../Nagi_OS_0.2_Linux_Migration_Addendum.md
[FP]: ../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md
[IMPACT]: KERNEL-TRANSITION-ALIGNMENT-2026-11-13.md
[ADR-L]: ../decisions/2026-11-12-linux-migration-development-scope.md
[BOOT]: ../architecture/boot-trust-and-system-updates.md
[LANG]: ../architecture/language-architecture.md
[GOV]: MEMORY-RESOURCE-GOVERNOR-DESIGN.md
[RM]: roadmaps/README.md
[R03]: roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md
[R04]: roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md
[R05]: roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md
[REG]: ../../.dev/workstreams.json
[WS]: ../workstreams/
[STATUS]: ../implementation_status.md
[PR42]: https://github.com/Roy-To/NagiOS/pull/42
[PR43]: https://github.com/Roy-To/NagiOS/pull/43
[PR48]: https://github.com/Roy-To/NagiOS/pull/48
[PR57]: https://github.com/Roy-To/NagiOS/pull/57
[PR91]: https://github.com/Roy-To/NagiOS/pull/91
[PR99]: https://github.com/Roy-To/NagiOS/pull/99
[PR105]: https://github.com/Roy-To/NagiOS/pull/105
[PR120]: https://github.com/Roy-To/NagiOS/pull/120
[PR124]: https://github.com/Roy-To/NagiOS/pull/124
[PR125]: https://github.com/Roy-To/NagiOS/pull/125
[PR126]: https://github.com/Roy-To/NagiOS/pull/126
[PR127]: https://github.com/Roy-To/NagiOS/pull/127
[PR128]: https://github.com/Roy-To/NagiOS/pull/128
[PR129]: https://github.com/Roy-To/NagiOS/pull/129
[P64]: DEC-064-nagi-share-nearby-file-transfer.md
[P65]: DEC-065-nagi-handoff-work-continuation.md
[P66]: DEC-066-device-connectivity-lan-first.md
[P67]: DEC-067-nagi-power-deterministic-resource-management.md
[P68]: DEC-068-nagi-sleep-resume-recovery.md

### 2026-11-16 仕様統合に伴う参照更新

[PR #107〜#129の統合対応表](PR-107-129-INTEGRATION-2026-11-16.md)に従い、Share／Handoff／Connectivity／Power／Sleepの参照を仕様正本へ接続した。元の35項目・分類・検討束・優先度・実装／target受入欄は維持し、文書マージを機能実装済みや受入PASSへ昇格させない。

### 2026-11-16 Foundation #1 カーネル系列方針の決定

[DEC-077](DEC-077-linux-kernel-lts-stable-policy.md)に、ユーザー決定として製品の標準Linuxカーネル系列はLTS、開発・先行検証では最新Stableを許容する方針をCONFIRMEDで記録。具体kernel revision／更新責任／patch・config／適用Releaseは未確定。旧Kernelの実装証拠をLinux target受入済みにしない。

### 2026-11-16 Foundation #2 Boot Systemの起動方式方針

[DEC-078](DEC-078-boot-platform-entry-and-common-nagi-startup.md)に、PCのUEFI基本とRaspberry Pi／タブレット等の機器に適したboot方式の許容、Linux起動後のNagi専用処理を共通化する方針をCONFIRMEDとして記録。Linux bootloader、署名／A-B／復旧方式の具体採否と実機起動・target受入は未確定。

### 2026-11-16 Foundation #3 System Imageの基本方針

[DEC-079](DEC-079-system-image-readonly-ab-data-separation.md)で通常時read-only System、OS更新後も保持するUser Data分離、A/B更新と復旧、AIモデルの論理分離をCONFIRMEDとした。旧0.1の6partition/FAT32等の構成をLinuxへ自動採用せず、具体partition・filesystem・容量・Recovery・データschema・Linux target受入・提供版は未確定。

### 2026-11-16 Foundation #4 Init／Service Managerの選定プロセス

[DEC-080](DEC-080-init-service-manager-ab-evaluation.md)に、A（最小Nagi独自PID 1＋独自Service Manager）を第一候補とし、B（既存軽量Init＋Nagi Service Manager）との短期実装比較で最終決定する方針をCONFIRMEDとして保存。方式の恒久採用、検証期間の数値・結果、Linux target受入・適用版は未決定。既存のLinux移行権限／Release Gateを維持する。

### 2026-11-16 Foundation #5 基本ユーザーランド方針

[DEC-081](DEC-081-nagi-owned-userland-selective-oss-components.md)で、Nagi固有サービス／API／アプリ環境の独自設計と、汎用libc・ライブラリ・基礎ツールのOSS選択利用、既存Linuxディストリビューション完成ユーザーランド非採用をCONFIRMED。具体OSS・ABI・POSIX／relibc等は未決定。B1の#1〜#5は方針検討が一巡したが、#4のA/B比較と正式採用、Linux image／実機acceptance・Release条件は完了ではない。

### 2026-11-16 Foundation #6 電源ライフサイクル協調方針

[DEC-082](DEC-082-power-lifecycle-coordination.md)で、Nagiによるshutdown／reboot／sleep／resume／将来のhibernateを含む電源遷移の統括、各サービスの保存・停止・復帰準備と失敗時の中止／再試行／警告、データと安全性の優先をCONFIRMED。DEC-067／068の既存正本・DEC-014／022の復旧とTask非自動実行を維持。状態機械・処理順序・timeout・実装構成／端末・Linux target受入と提供版はPROVISIONAL。

### 2026-11-16 Foundation #7 Identity／SessionとLinux資格情報の分担

[DEC-083](DEC-083-nagi-identity-linux-credentials-binding.md)で、Nagi独自のUserId／ProfileId／SessionIdを論理Identity正本とし、Linux UID／GIDは低レイヤーのアクセス制御に利用するB案をCONFIRMED。ログイン／ロック／復帰の認証・権限はNagiの管理下で、Profile別データ・Memory・Workspace・Task分離とAI／アプリのOwner非昇格、Guestと将来Team対応を維持。具体マッピング・認証技術・Session lease／失効・Linuxでの強制／受入・Releaseは未決定。

### 2026-11-16 Foundation #8 Permission／SandboxのLinux強制隔離

[DEC-084](DEC-084-nagi-permission-linux-sandbox-enforcement.md)で、NagiのPermission／Capability正本＋Linuxの実効的なSandbox／アクセス制御の併用（B案）をCONFIRMED。User／Profile／Session／App／Task／Resourceの現行権限、直接経路の迂回防止、最小権限・取消／失効／狭窄、AI・アプリ非特権、重複承認不要を維持。Linux方式の具体選定、強制隔離の実装・迂回拒否／失効試験・リリース割当ては未決定。

### 2026-11-16 Foundation #9 IPC／Service Broker選定

[DEC-085](DEC-085-linux-transport-nagi-service-broker.md)で、Nagi独自の型付きIPC／Service API・Service BrokerとLinux標準IPCを組み合わせるB案を基本方針としてCONFIRMED。Nagi論理Identity／PermissionをCallerに結び付け、Transport独立の契約を維持する。Unix Domain Socket等の具体採否、受渡し・キャンセル・再起動・失効の詳細、性能・Linux target／実機受入、適用版はPROVISIONAL。Kernel独自IPC開発は初期必須としないが将来の限定的検討を禁止しない。

### 2026-11-16 Foundation #10 Secrets／Key Management

[DEC-086](DEC-086-nagi-secrets-service-hybrid-key-protection.md)で、Nagi独自Secrets ServiceとTPM／TEE等対応機器優先＋非対応機器の暗号化ローカル保管というハイブリッドC案をCONFIRMED。用途・User／Profile分離、Identity／Permissionによる認可、秘密値の不要なAI／App露出・ログ露出・自動Cloud預託の防止、更新・失効と回復限界の明示を含む。具体暗号・KDF・鍵階層／形式・用途別回復・Linux強制／実機検証・Releaseは未決定。B2 #6〜#10の方針検討は一巡したが正式実装・受入ではない。

### 2026-11-16 Foundation #11 段階的な信頼基盤

[DEC-087](DEC-087-tiered-system-trust-and-signature-verification.md)で、対応機器のハードウェア信頼機構を活用し、非対応機器でもソフトウェア署名検証するB案をCONFIRMED。OS／更新／アプリの署名と改ざん検出、Ownerの認可付きDeveloper Mode、製品／開発署名鍵の分離・更新／失効・危険なRollbackの防止と安全な復旧を設計対象とする。具体的Linux trust chain／Secure Boot／鍵運用・anti-rollback・実機対応／受入・版は未決定。B2の#7〜#11が高水準方針として一巡したが、Runtime／Linux実機受入は未完了。
