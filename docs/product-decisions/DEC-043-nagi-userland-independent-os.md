# DEC-043 — Linuxカーネル + Nagi専用ユーザーランドと独立OS方針

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（製品方針）／PROVISIONAL（具体構成・移植設計・適用版）  
**最終合意日:** 2026-11-12  
**対象:** Linux転換後のNagi OSの製品・設計境界。現行0.1実装の変更・開始許可ではない  
**関連:** [DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-023](DEC-023-work-data-centric-os.md)、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)、[roadmaps](roadmaps/README.md)

> **2026-11-13 アプリ環境の後続決定:** [DEC-017](DEC-017-third-party-developer-ecosystem.md)で一般向けアプリ環境をNagi Native Apps専用とする標準方針、他OS向けアプリのNagi対応版リリースをSDK／UI／AI支援で容易にする目標を確定。Linux／iOS／Android等のバイナリ無変更互換レイヤーは標準搭載しない。以下の「互換方式は未確定」は旧時点の履歴、ならびにlibc／ABI、実際の移植方式、例外的互換性、Package形式等の技術詳細の未確定を示す。既存release Gateや移行開始許可は変更しない。

> **2026-11-16 Boot System後続決定:** [DEC-078](DEC-078-boot-platform-entry-and-common-nagi-startup.md)で、UEFI対応PCではUEFIを基本とし、Raspberry Pi／タブレット等は機器に適したBoot方式を許容、Linux起動後のNagi処理を共通化する方針をCONFIRMEDとした。具体bootloader・handoff・起動時の信頼／復旧方式・対応機種・適用版は引き続き未確定。本文中のBoot方式未決定という記述は初回時点の履歴であり、この高水準方針に限りDEC-078を優先する。

## 決定の位置付けと前回作業との境界

2026-11-12の追加確定指示を記録する。前回の[DEC-042 — 独自Nagi KernelからLinuxカーネルへ変更](DEC-042-linux-kernel-transition.md)を置換・重複作成する目的ではなく、転換後のユーザーランドと独立性を明確化する補足決定である。前回記録の未確定欄に「ディストリビューション／ユーザーランド」が残る場合、本DECで明示的に確定した方針の範囲だけ更新される。具体的な基本ユーザーランド構成は引き続き未確定。

根拠は、参照会話「独自カーネルを比較検証」におけるユーザー発言と、本タスクの明示的な追加確定指示。assistantの提案や未取得の回答本文を確定事項の根拠にしない。

## 確定した仕様（CONFIRMED）

1. **Linuxカーネル + Nagi専用ユーザーランドを採用する。** Linuxへの転換後も、Nagi独自のOS体験、サービス・Platform契約とアプリ環境を構築する。
2. **既存LinuxディストリビューションをOS基盤として流用する案は採用しない。** 既存ディストリビューションの完成したroot filesystem、サービス構成、パッケージリポジトリ／依存体系、デスクトップ環境を製品の土台として引き継ぎ、その上の外観だけをNagi化する方針ではない。
3. **OSSコンポーネントの選択的再利用は可能。** 上記は、すべてのライブラリ、ツール、プロトコルやOSSを自作する指示ではない。Linux以外の個別技術もNagiの設計境界・セキュリティ・保守性との適合を評価して選択できる。個別コンポーネントの採用決定は本DECでは行わない。採用時には依存・出所・固定revision／patch・ライセンス条件を個別に確認する。
4. **Android / ChromeOS（参照会話ではChromium OS）に対抗できる独立したNagi OSを目標とする。** これは製品目標であり、機能・性能の優越、互換性、対応機種、市場成果や納期を保証するものではない。Android / ChromeOS自体をOS基盤として流用する承認でもない。
5. Nagi-first、通常GUI、AI非依存の基本操作、作業・データ中心の体験、第三者アプリの段階的統合を継承する。既存DECのPermission／Capability／Policy、非特権AI、型付きPublic Actionの境界をLinux採用で緩めない。

「Nagi専用」は、Nagiが責務と公開契約を設計・管理する意味であり、全構成要素が新規の独自実装という意味ではない。開発ホスト上のディストリビューションやビルドツール利用は、製品OSの基盤採用とは区別する。成果物が既存ディストリビューションのrootfs・サービス・配布体系に依存するかを評価し、ビルド環境の名称だけで採否を判断しない。

## ユーザーランドの設計境界（REVIEW NOTE / 具体方式はPROVISIONAL）

以下は確定方針を設計へ展開するための責務整理であり、新しいAPI・プロセス分割・採用技術の確定仕様ではない。

| 層 | Nagi側で設計・維持する責務 | 移植時に評価する境界・未確定事項 |
|---|---|---|
| Linux kernel / boot境界 | Linuxを実行・メモリ・デバイス等の低層基盤として用い、Nagiユーザーランドへ起動を引き渡す | kernel版・設定・patch、bootloader、rootfs構築、driverと権限強制機構。既存loaderをそのままLinux用と見なさない |
| init / lifecycle | 起動順序、サービス起動・監督、依存・health・有限再起動、終了／復旧の契約 | PID 1、init実装、service manager、process分離、既存nagi-init／libnagiの移植範囲 |
| services | Identity／Session、Permission／Policy、Resource／Storage、通信・音声・Package等のOSサービス契約 | Linux機構とNagi権限モデルの対応、IPC・認証・失効・仲介、特権サービスの最小化。Linux capabilityとNagi Capabilityを同一視しない |
| desktop / shell | Nagi Home／Desktop、従来GUI、Workspace、入力・表示、アプリ起動とユーザー操作 | compositor、display/input backend、toolkit、font、アクセシビリティ。Wayland等を本DECで採用しない |
| platform | 型付きPublic Action／Context／Event、Resource／Revision、Activity、Checkpoint、SDKの公開契約 | OS依存backend／adapter、IDL・ABIの版管理、呼出しごとの認可と副作用経路。UIやAIからraw Linux APIで迂回しない |
| application / AI runtime | アプリの起動・隔離・終了、manifest／権限、配布・更新、非特権AI・browser内容の隔離 | native ABI、libc、Rust std、PAL／relibc、sandbox、package形式、既存アプリ互換、Servo・model runtime移植。無変更実行を保証しない |

Linuxのsyscallやdriverを利用する方式は従来独自カーネルの実装方式と同じではない。高水準製品契約をユーザーランドに保つことと、Linux自体の既存機能を禁止することを混同しない。実際に権限を強制できるかは、Linux機構・サービス・app sandboxを含むThreat Modelとtarget上の検証が必要。具体的機構の選定や同等性の成立は未検証。

## 既存実装の移植方針（REVIEW NOTE / 実行計画はPROVISIONAL）

| 既存資産 | 整理方針 | 必要な確認 |
|---|---|---|
| OS非依存のmodel、schema、状態機械、UIロジック、SDK／Public Action契約 | 再利用候補として保護し、portable coreとbackendを分離して評価 | host testの再利用可能範囲、依存・ABI差分。既存test成功をLinux target PASSへ移さない |
| nagi-init、libnagi、desktop、service registry、Permission、Package、AI／browser adapter | Linux依存部分とNagi固有Policyを棚卸しし、移植・adapter化・再設計を個別判断 | process／IPC／handle／VMO、storage／network、display／input、認可・取消・復旧の対応表 |
| 独自kernel、UEFI loader、VirtIO等の低層実装 | 現行0.1資産・受入証拠として保持し、Linux版で置換・参照する範囲を後続ADRで定義 | 削除・機械的移植・PASS移行を行わない。Linux側のdriver・bootとの重複責務を調査 |
| nagi-pal、nagi-posix、relibc、Rust target／std、.xappと署名・更新経路 | 現行release契約として保持し、新基盤で必要な部分／置換候補を評価 | libc、syscall ABI、source／binary互換、署名・trust・rollback・data migration。形式の継続／変更は別途判断 |
| test、benchmark、QEMU evidence、workstream registry | 過去の証拠と新基盤で再実施する受入を区別 | 実行対象・commit・環境を明示し、既存MilestoneやGateを自動的にPASS／無効へ変更しない |

後続の検討順序は、資産と依存の棚卸し → 新しいkernel／userland境界とThreat ModelのADR → 再現可能なboot／init／service／appの最小経路設計 → 分離した移植検証 → 対象release仕様・Gateの明示的改訂と移行判断、とする案。これは移行の開始許可・日程・新たなGateの制定ではない。

## 現行仕様・ADR・roadmapとの整合

- root READMEの0.1 Developer Preview／QEMU対象は現行開発の説明であり、本DECでLinux版の起動・実装完了へ書き換えない。
- 現行AGENTS.md、0.1 Implementation Spec、ADR-0001（hybrid kernel）、ADR-0010（handles／VMO／IPC）、M6 init／registry、M10 UI、M11 Permission、M12 network、M13 PAL／POSIX、M16 packageは旧基盤に対する実装契約。将来方針との相違を明示し、移行ADR・release仕様の承認なく書き換えない。
- DEC-017／020の独自カーネルを前提とする説明は、前回Linux転換決定の対象。AIコーディング最適化、SDK、portable code再利用、非特権AIとPublic Actionは維持する。
- 前回決定がユーザーランドやディストリビューションをPROVISIONALとした場合、**Nagi専用ユーザーランド採用／既存ディストリビューション基盤非採用／個別OSS再利用可能**の製品方針は今回CONFIRMEDへ進む。具体構成・互換方式・時期まで確定したと解釈しない。
- Private 0.3 RevisedはPersonal Agent、0.4+は将来Role／Team構想、0.5–1.0は暫定版配置。Linux移行を勝手に0.3や1.0の必須完了要件へ追加しない。版割当て・順序・受入条件は既存Gateと別途の移行判断に従う。
- **0.1 Milestone／M30、0.2 M21、0.3仕様・Acceptance、0.4 Entry Gate、Integration Owner checkpoint、Workstream owner・許可pathを変更しない。** 本記録は実装済み・activation・target acceptance・公開許可を意味しない。
- 未公開仕様はPrivateに保持する。Publicへの転載・PR・issue・CI artifactへの掲載には別途の公開判断が必要。

## 未確定事項（PROVISIONAL）

- 基本ユーザーランド構成：init、service manager、libc、基礎utility、rootfs・build／updateシステム、各OSSの採否。
- 互換性方式：Nagi native ABI、Linux binary／POSIX、Android、Web・GUI toolkit等の対象と実現方式。Linux採用だけで既存アプリが動くとしない。
- パッケージ形式：現行.xappを含む継続／変更、system imageとapp更新の分離、署名・trust・distribution・recoveryの詳細。
- 移行タイミング：適用release、開始条件、既存実装との並行期間、切替・rollback・data migrationとtarget acceptance。
- ライセンス詳細：Linuxと各OSSの組合せ、改変・配布方法ごとの義務、Nagi固有部分のライセンス、公開範囲。利用許諾の適合や法的結論を本DECで確定しない。
- kernel版、boot、security enforcement、process／IPC、desktop backend、性能・互換性・ハードウェア対応と測定条件。

## 確認基準

Private main `6e12bab1dcd10879b7d7422931c393e5d8a677be`で前回DEC-042の保存を確認し、その続きとして本補足を作成した。README、OPERATING_RULES、関連DEC、0.3／0.4+／0.5–1.0 roadmapと0.1実装仕様・関連ADRを確認した。root README、実装コード、Gate、Publicは変更対象に含めない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-12 | ユーザーの追加確定指示によりLinuxカーネル + Nagi専用ユーザーランド、既存ディストリビューション基盤非採用、個別OSSの選択的利用可能、Android／ChromeOSに対抗する独立OS目標をCONFIRMEDとして記録。責務・資産移植の整理をREVIEW NOTE、具体構成・互換・Package・移行時期・ライセンス詳細をPROVISIONALに分離。現行実装・GateとPrivate境界を維持。 |


## 後続の限定開発承認（2026-11-12）

本DEC時点の「方針確定だけでは実装開始を意味しない」という境界に対し、後の明示承認を[DEC-044 — Linux移行開発の開始承認](DEC-044-linux-migration-development-authorization.md)に記録した。承認対象はPrivateでのLinux移行設計・実装・検証であり、適用範囲は[移行開発ADR](../decisions/2026-11-12-linux-migration-development-scope.md)とAGENTS / 0.1実装仕様の2026-11-12限定優先注記に従う。

旧本文・過去時点の境界は履歴として保持する。既存のrelease Gate、M30受入、正式切替・公開・配布・ユーザーデータ移行・互換性保証は自動承認しない。具体的構成の可逆的な開発選択と、永久的な製品判断を区別する。許可された場所・範囲・資源での実開始および成果は別途確認する。
