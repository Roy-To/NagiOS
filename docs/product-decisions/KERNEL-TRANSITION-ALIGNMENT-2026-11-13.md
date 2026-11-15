# Linuxカーネル方針変更 — 仕様・ロードマップ整合表

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**確認日:** 2026-11-13（JST）  
**種別:** REVIEW NOTE / 文書整合。新しい製品決定・release activation・runtime受入ではない。  
**決定正本:** Private `Roy-To/NagiOS-internal` main `9e0b185e2cf3fffe371437bf015e7035c672798b`（本改訂の親基準）  
**実装確認基準:** Public `Roy-To/NagiOS` main `48b4eeb12a4a6674badd0a530149570ff4c30d30`  
**編集担当:** 本依頼の文書担当。対象は本表・Private仕様補足・索引・日付付き注記のみ。既存のコード担当・Workstreamの割当を変更しない。

## 1. 決定・承認・実装を分ける

| 区分 | 確認した内容 | 根拠 |
|---|---|---|
| CONFIRMED — 製品基盤 | 独自Nagi KernelからLinux kernelへ転換。Linux採用自体を再び単なる候補としない | [DEC-042](DEC-042-linux-kernel-transition.md) |
| CONFIRMED — 製品構成方針 | Nagi専用ユーザーランド。既存distributionを製品OS基盤とせず、個別OSSは選択的に利用可能。独立OSの目標は維持 | [DEC-043](DEC-043-nagi-userland-independent-os.md) |
| CONFIRMED — 限定開発承認 | Private移行設計・実装・検証は承認済み。割当済みの対象では旧独自kernel完成や旧kernel固有Milestoneを一律の着手前提にしない | [DEC-044](DEC-044-linux-migration-development-authorization.md)、[移行ADR](../decisions/2026-11-12-linux-migration-development-scope.md) |
| PROVISIONAL — 恒久構成 | kernel revision、init、libc、IPC/security enforcement、display backend、package形式、互換方式、適用release・切替時期・ライセンス詳細 | 可逆的な開発選択はDEC-044の委任範囲で根拠・固定入力・変更境界を記録して選べる。恒久的製品採用・互換保証とは分ける |
| 未確認 — 実開始・統合 | DEC-044/ADRの保存時点では実装branch/path/担当/環境はNOT ASSIGNED。本調査でLinux imageの起動、同一本体adapter、通常GUIからの保存・再読込の受入証拠を確認していない | 承認・設定保存・指示・引受・実開始・成果・正式受入を同一視しない。全環境で未着手と断定するものではない |

DEC-042/043の初回「方針記録だけでは実装開始を意味しない」は履歴である。後続DEC-044を無視して全移行開発を停止しない。一方、開発承認を既存release Gateの全面解除に拡張しない。

## 2. 実装リポジトリから確認できた状態

以下は指定refのファイルを読んだ結果であり、今回build・test・VM実行を行ったという意味ではない。

- Privateの[implementation_status.md](../implementation_status.md)旧本文は2026-09-19のM17 NOT STARTEDを記録する。最新のPublic実装全体を表すものではないので、旧記録を上書きして偽の最新状態を作らない。
- Public [implementation_status.md](https://github.com/Roy-To/NagiOS/blob/48b4eeb12a4a6674badd0a530149570ff4c30d30/docs/implementation_status.md)のCurrent statusはM19 PARTIAL、M20 PARTIAL、M21–M30 PARTIAL。上部には後日の部分進捗・診断記録もある。単一の受入command成功、過去CI成功、M30 image試験成功をrelease全体のM30 PASSへ読み替えない。
- Public [Cargo.toml](https://github.com/Roy-To/NagiOS/blob/48b4eeb12a4a6674badd0a530149570ff4c30d30/Cargo.toml)、[kernel/Cargo.toml](https://github.com/Roy-To/NagiOS/blob/48b4eeb12a4a6674badd0a530149570ff4c30d30/kernel/Cargo.toml)は独自kernel、libnagi、nagi-pal、nagi-posix、nagi-init等を含む。既存資産の移植候補であり、Linux imageへの接続済み証拠ではない。
- [user target](https://github.com/Roy-To/NagiOS/blob/48b4eeb12a4a6674badd0a530149570ff4c30d30/targets/x86_64-unknown-nagi-user.json)は`os=nagi`、`llvm-target=x86_64-unknown-none`。[libnagi](https://github.com/Roy-To/NagiOS/blob/48b4eeb12a4a6674badd0a530149570ff4c30d30/user/libnagi/src/lib.rs)はNagi固有ABIのSYS_*、channel、handle等に依存する。Linux向け再buildだけで契約が成立するとは判断しない。
- [image builder](https://github.com/Roy-To/NagiOS/blob/48b4eeb12a4a6674badd0a530149570ff4c30d30/tools/nagi-cli/src/image.rs)はUEFI loader・kernel・nagi-init user ELFを入力に取る既存経路。[nagi-init manifest](https://github.com/Roy-To/NagiOS/blob/48b4eeb12a4a6674badd0a530149570ff4c30d30/user/nagi-init/Cargo.toml)のguest/acceptance feature群も旧基盤のもの。Linux boot/rootfs/PID1/service起動を別途接続・検証する必要がある。
- 2026-11-13のAPI確認でPublicのPR #42 / #43 / #48（Files、session worker、Desktop統合）、#57（multiarch）、#91（host-only port readiness）、#99（dependency/cache）はOPEN。未統合branchやhost準備をmain完成・Linux移行済みとしない。PrivateのOPEN PR一覧は空。これは調査時snapshotであり、別場所の未公開作業の不存在を証明しない。
- 付属third_partyのLinux対応ファイルの存在はupstreamの対応コードであり、Nagi用Linux imageや移行実装開始の証明ではない。

## 3. 仕様への影響範囲

| 文書・範囲 | 旧基盤への依存／維持する契約 | 今回の整合更新・後続担当への引渡し |
|---|---|---|
| [Private AGENTS](../../AGENTS.md)／[0.1仕様](../Nagi_OS_0.1_Codex_Implementation_Spec.md)冒頭 | 2026-11-12限定優先注記は既に保存済み。旧本文のnot Linux-based等を無言で全置換しない | 限定注記を維持し、本表・0.2補足へ導線を追加。Public AGENTSは変更しない |
| 0.1 §0–6、§13–17、§67–68、§79 | own kernel、hybrid kernel、UEFI handoff、syscall/ABI、scheduler、driver、disk image、supervisor | Linux boot → Nagi userland → Nagi serviceの最小経路で置換・再利用範囲を移行ADRへ具体化。旧低層完成は限定開発の一律前提ではない |
| 0.1 §7–12、§22–24、§40、§57 | handles/VMO/channel、Identity、Permission、AI隔離、app authority | Linux UID/root/capabilitiesをNagi Capabilityと同一視しない。caller identity、権限狭窄、失効、取消、共有memory、資源上限の強制と失敗経路を同一本体で検証する |
| 0.1 §18–21、§56–58 | Object ID/Resource、永続化、Transaction/Wayback、A/B recovery、package/trust | Nagi製品契約とdata保護を継承。Linux storage/update backendと復旧範囲は未確定。利用者data移行・旧Permission復元を自動承認しない |
| 0.1 §25–38、§60–64 | UI/compositor、Servo、network/audio、PAL/relibc、Rust std、spawn | 高水準Nagi契約を維持し、OS依存backendを棚卸し。Linuxの既存syscall利用を旧kernel向けの禁止文から一律禁止しない。Wayland・libc・全面binary/toolkit互換を本改訂で採用しない |
| 0.1 §39–55、§69–78 | 非特権AI、provider-neutral routing、model、localization、privacy、pin・CI | kernel変更によるモデル変更や新機能activationは行わない。host試験・Linux同一本体試験・Nagi Linux image内試験を区別 |
| 0.1 §84–98／ADR-0001・0002・0010等 | 旧Milestone順序、Definition of Done、channel/VMO/hybridの技術契約 | 過去の本文・PASS・残件を保全。旧ADR全体をSUPERSEDEDにせず、移植対象の差分と新target証拠を記録。release Gateは維持 |
| Public 0.2両仕様 §2.1、§5–7、§29–32、§35–37 | 旧起動連鎖・参照target、release activation、owner/shared files、host-v-runtimeと再統合 | [Private 0.2移行補足](../Nagi_OS_0.2_Linux_Migration_Addendum.md)を追加。Public両本文は保持。M30等のrelease条件とDEC-044の限定開発開始を別経路として明記 |
| Public 0.2 §8–27／0.2-M21 | Identity/IPC/Permission、Jobs、Settings、Model、SDK、manifest/package/out-of-tree app | 既存DeliverableとAcceptanceを維持。Linux adapterのABI、認証、保存・復旧の接続証拠が別途必要。SDKを0.3へ延期しない |
| [Private 0.3 Revised](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md) §5、§17–22、§26–34 | Profile/Canonical Memory、Context/Event/Public Action、model portability、Personal Agent、仕様化とactivation境界 | Linux移行との適用注記を追加。§26のIdentity等の条件と§34のrelease境界を維持。旧M30順序文を限定開発の全面停止と読まない |
| [Private 0.4+](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md) §2、§11–12 | 初期単一Nagiのbuilder Role、狭窄、Stop、監査、初回Role受入と将来Agentの別Gate | kernel選択とrelease activationを分離する注記。開発作業の並列化を製品のMulti-Agent実装許可としない |
| [Private 0.5–1.0](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)／[DEC-021](DEC-021-roadmap-0.5-1.0.md) | 1.0品質／開発者・配布目標、版別配置は暫定 | Linux転換の適用版・期日を追加しない。DEC-013の正式Memory Engineを移行を理由に0.5へ延期しない |
| [DEC-017](DEC-017-third-party-developer-ecosystem.md)／[DEC-020](DEC-020-ai-os-interaction-path.md) | own-kernel戦略の前提だけ置換。SDK、AI coding、構造化操作、前景優先は継承 | DEC-017の冒頭に残る「userland未承認」をDEC-043/044の時系列で明確化。新たな互換保証や権限を作らない |

## 4. Gateと担当境界

- **0.1:** M30をPASSへ変更しない。旧kernel・loader・branch・未統合差分・Filesの保全対象・試験証拠を削除しない。
- **0.2既存release runtime:** M30 PASS、明示的0.2 release境界、対象WorkstreamのIntegration Owner checkpointを維持。Public仕様§29と[registry](https://github.com/Roy-To/NagiOS/blob/48b4eeb12a4a6674badd0a530149570ff4c30d30/.dev/workstreams.json)のowner_branch・allowed_paths・forbidden_paths・shared-path調整を勝手に変更しない。
- **Private Linux移行開発:** DEC-044/ADRで承認された別の限定経路。旧kernel-only前提を着手阻害にせず、実在する許可済みsource location・branch/path・担当・入力出力・統合先・試験/資源条件を個別に確認する。NOT ASSIGNEDという保存時snapshotから他担当への新規割当を捏造しない。
- **0.3:** Revised §26のImplementation Spec作成条件（0.2がおおむね50〜70%安定という目安、Identity・Settings・IPC・Model・Permission・Storage・Jobs）と§34の受入・activation境界を維持。全0.2 Workstream PASSを新たに要求しない。移行開発承認からProfile/Memory runtimeの全面開始を推定しない。
- **0.4+:** §2.1の0.3 target受入、再現可能なoffline音声/Action/Permission/取消/復旧証拠、安定した相関・契約、Threat Model、別release境界とIntegration Owner checkpointを維持。後続専門Agentの別Gateも維持。
- **資源・権限:** DEC-044に記録されたSSD/Git・環境新設・install・課金・VM・保全対象・拒否／結果不明操作の条件を継承する。本改訂は文書編集だけで、それらを解除しない。公開・配布・user-data移行・互換保証は別判断。

## 5. 次の実装担当が具体化する内容

以下は新たな一括開始Gateや担当割当ではなく、DEC-044の個別工程の引渡し項目である。全体設計の完了を待たず、許可された独立工程は進められる。

1. 既存の許可済み作業場所、branch、担当path/interface、owner、統合先と試験資源を記録する。Files/共有Git/生成物に重複編集を作らない。
2. 開発用kernel/toolchain/rootfs/boot入力を固定し、採用理由・出所・license確認状況・可逆な境界をADRへ記録する。ここでは具体構成を選定しない。
3. Linux boot → Nagi userland → Nagi serviceの同一target経路を作り、次に既存一appの通常画面操作 → 認可済み実data保存 → 再起動後再読込を接続する。
4. caller認証、拒否、失効、取消、部分失敗、復旧を含むNagi契約を同じ本体/adapterで確認する。host別実装・fixture・従来kernel結果で代用しない。
5. portable logic、Linux上の同一本体/adapter、Nagi Linux image内統合の結果を別記し、commit/diff・入力・環境・制限・未接続部分を添える。
6. 正式製品切替や公開が必要となった時点で、その具体的範囲とGate変更を別途扱う。未確定のrelease番号を既定値にしない。

## 6. 今回の検証範囲

文書差分、相対リンク、保存先・ref、旧本文保持、release/ownership境界、独立監査を検証対象とする。OSコード・設定・IDL・依存pin・Workstream registry・release statusは変更しない。build、host unit test、Linux image起動、QEMU、real runtime・tenant等の外部受入は実施しない。文書の整合検証をruntime PASSとして報告しない。

## 変更履歴

- 2026-11-13: DEC-042/043/044と実装refを照合。Private 0.1注記・0.2補足・0.3以降roadmapの適用関係を整理。新しい製品決定は追加していない。
