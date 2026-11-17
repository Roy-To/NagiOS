# DEC-017 — 第三者アプリ開発・参入環境

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

> **2026-11-13 時系列補足:** 下記2026-11-12注記の「ユーザーランド未承認」はDEC-042時点の履歴。[DEC-043](DEC-043-nagi-userland-independent-os.md)によりNagi専用userland／既存distribution基盤非採用／個別OSS再利用可能の製品方針はCONFIRMED。[DEC-044](DEC-044-linux-migration-development-authorization.md)によりPrivate移行開発も対象限定で承認済み。Linux binary/toolkit互換、恒久構成・正式切替時期・既存release Gate変更は引き続き未確定。本書の一般的な開始境界を、承認済み移行開発の一律停止へ拡張しない。旧本文は保存し、[影響表](KERNEL-TRANSITION-ALIGNMENT-2026-11-13.md)で適用範囲を確認する。


> **2026-11-12 方針変更:** 本書の「独自カーネルの強みを活かす」というカーネル選択の前提のみ、[DEC-042](DEC-042-linux-kernel-transition.md)によりSUPERSEDED。Linuxカーネルへの方針転換はCONFIRMED。AI coding優先・段階的公開API統合・開発者／配布目標は維持し、Linuxアプリの無変更互換性、ユーザーランド、移行時期は承認されていない。 旧本文は履歴として保持し、本注記がカーネル選択について優先する。既存実装仕様・ADR・Release / Activation Gateは変更しない。


**状態:** CONFIRMED（既存の開発者体験・段階的統合・Nagi Native Apps専用・移植負担低減と1.0開発者／配布目標、PX-15実用OS B＋将来総合OS Cの高水準完成方針との整合）／PROVISIONAL（個別技術・対応範囲・工数・SDK／配布詳細・具体的1.0受入・提供版）  
**最終合意日:** 2026-11-17（PX-15と1.0開発者／配布目標の整合追記。従前の移植方針を維持）  
**対象:** 0.1の先行準備、0.2 Platform Foundation、0.3 Personal Agent、および1.0に向けた開発者・配布環境  
**関連:** DEC-001、DEC-007、[0.2 Platform Foundation仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)、[純正ソフトウェア実装仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[DEC-036（builder製アプリ共有・公開）](DEC-036-builder-app-sharing-and-publication.md), [DEC-041（アンインストール）](DEC-041-builder-app-uninstall-and-data-deletion.md), [DEC-071（初心者向けApp制作）](DEC-071-nagi-native-app-creation-experience.md)

## 目的とユーザー体験

Nagiの純正アプリを充実させながら、個人・第三者の開発者もNagi向けアプリを作り、公開APIとの統合を段階的に深められる環境を目指す。Nagi専用の言語やIDEを学ぶことを入口の条件にせず、既存PCの開発環境から通常のアプリを作り、必要に応じてNagiの機能を利用できるようにする。

## 確定した仕様

### Nagi Native Apps専用と低負担の移植・追加リリース（2026-11-13確定）

- **一般向けアプリ実行・配布環境はNagi Native Apps専用とする。** 純正・第三者を問わずNagiのアプリ契約（Package、Manifest、SDK／Public API、Permission／Capability）に適合させる。Linuxデスクトップアプリ、iOS／Android等の既存アプリバイナリを無変更で動かす汎用互換レイヤーは標準搭載しない。将来の例外的な互換性導入は別途の明示決定を必要とする。既存の.xapp形式の継続・変更は未確定。
- **移植しやすさを普及の中核要件とする。** iOS／Android／Windows／macOS／Linux等で開発されたアプリについて、権利者が許可したソースコード、共通ロジック、データモデル、UI設計を可能な限り再利用し、Nagi対応版を追加リリースする負担を減らすことを目指す。OS固有API・GUI Frameworkの修正やNagi向け再実装が必要になる場合を認め、既存バイナリやSwiftUI等のUIコードの完全・無変更互換は保証しない。
- **SDK・UI Framework・AI支援による省力化を重視する。** 標準UI部品、入力／画面処理、Nagi Public API、Manifest、移植差分診断、ビルド・試験・パッケージ検証、AIコーディング支援を整える。具体的な移植支援ツール、対応言語／Framework、AI変換の方式・適用バージョンはPROVISIONAL。Nagi専用言語／専用IDEは必須としない。
- **Nagiの段階的統合モデルを守る。** Level 0の通常GUIアプリとして移植してから、必要な範囲でLevel 1〜3のContext、Search、型付きPublic Action、Workspace、Wayback等を採用できる。全アプリへのAI統合は強制せず、AIなしでも基本操作を成立させる。NagiのPermission／Capability／Policy、純正・第三者の共通Public API、非特権AIの境界を緩めない。
- **追加リリース・継続保守の工数削減は製品目標であり保証ではない。** 移植元の依存、GPU／音声、ライブラリ／ライセンス、データ移行、検証コストによって難易度は異なる。具体的な移植例で工数・再利用範囲・未対応点を記録し、支援機構を改善する。数値目標、無変更実行、すべてのアプリの移植可否は未確定。
- **適用境界:** Nagi内部のLinux／OSSコンポーネントの選択的使用やAlbertからのWebサイト利用まで禁止するものではない。DEC-042/043と既存0.2-M21 SDK、0.1〜0.4のGate、Workstream所有、正式公開・配布・インストール承認は変更しない。確定したのは製品方針であり、移植ツール完成や移植受入PASSではない。

### 移植支援の対象プラットフォーム優先順位（2026-11-13）

**CONFIRMED — 現時点の製品・開発支援優先順位**

1. **PC／Desktop：Windows向け・Linux向けアプリからNagi Native Appsへの移植支援を重点対象とする。** どちらも優先的に扱い、既存のソースコード・データモデル・UI設計等の再利用を支援する。優先対象は「移植元」であって、WindowsやLinuxディストリビューションのアプリバイナリをNagi上で無変更実行する互換機能、あるいはNagi OSをWindows上で実行する機能を意味しない。Windows／Linux相互の厳密な先後順位は定めない。
2. **将来のMobile／Tablet：Androidアプリからの移植支援を第一の検討対象とする。** PC／Desktop firstの製品・実装順序を維持する。Androidを対象にすることはAndroid APK・Android Runtime・Google Mobile Services等をNagiで直接利用可能にする約束、モバイル向けNagiの提供・時期の確定、既存Gateの解除ではない。実機対応・モバイル環境が整うまでの対象選定と具体化はPROVISIONALとする。
3. **macOS／iOS：当面、移植支援の重点対象から外して後順位とする。** 将来の対応可能性を閉じず、一般的なPortable Coreの再利用や権利者による個別のNagi移植を禁止しない。macOS上の開発ホスト／IDEを許容する既存方針は維持する。AppleのOS／Framework固有の実行互換や変換ツールの提供を約束しない。
4. **Nagi専用アプリ環境を維持する。** 対象OSの優先順位は、移植しやすさ・開発者参入促進・ドキュメント・サンプル・診断の優先順位であり、Nagi SDK／Public App contract、Level 0〜3、Permission／Capability／Policy、AI非特権を変更しない。

**PROVISIONAL — 技術・成果物・版割当て**

- 具体的な言語、UI Framework、依存技術（例：WindowsのWin32／.NET系、LinuxのGTK／Qt系、AndroidのKotlin／Java／Compose系）への対応可否・優先順位・変換方式、権利／ライセンス条件、実際の移植工数・保守負担は個別評価で決める。これらの名前は対応を保証しない評価対象の例である。
- PC用移植支援ツール、Android向け将来の移植支援の実装時期・受入条件・版割当ては、既存のSDKとrelease gateに従って別途決定する。今回の方針自体は、新しい実装タスクを割り当てたり、Mobile Runtimeを有効化したりしない。
- PC移植支援を具体化した[DEC-045 — Nagi Porting Toolkit](DEC-045-nagi-porting-toolkit.md)を参照。PT-01読み取り専用診断を最初の設計対象とし、PT-01〜06の技術・受入・担当・実装版はPROVISIONALとして分離する。

### 開発環境とSDK

- 開発者は既存のmacOS、Windows、Linux PCと一般的なIDE・ツールチェーンを利用できる。VS CodeとRustは利用例であり、Nagi専用IDEの導入を必須にしない。
- Rust SDKを中心にし、必要な境界では安定したC ABIを提供する方針とする。Nagi固有のプログラミング言語を必須にしない。
- CLIとQEMUを通じ、アプリのビルド、検証、Nagi target上での起動を行える開発体験を目指す。
- 英語と日本語の開発者向け説明を用意し、Hello World、Notes、AI連携アプリの例と、Nagi本体の外に置いたサンプルアプリを提供目標とする。
- アプリ開発体験は、**AIコーディングに最適化することを優先方針**とする。独自カーネルから設計するNagiの強みを活かし、AI時代の先進的なOSを目指す。
- 既存Rust／C／C++アプリをNagi向けに移行しやすくすることを目標に含める。これは移行容易性の目標であり、すべての既存アプリの無変更実行を保証しない。

### 段階的なNagi統合

- アプリは、通常のアプリとしての基本機能から始め、必要なNagi Platform APIを段階的に採用できる。
- AI連携を必要としないユーティリティアプリも成立する。Agentや外部AIが利用できない場合も、アプリ自身の基本機能を利用できる設計を維持する。
- Context、Action、Search、Activity、Checkpoint / Wayback、Workspaceなどの統合は、アプリが宣言した権限と既存のPermission / Capability / Policyに従って有効にする。
- 0〜3の統合レベルの名称・内容は0.2 Platform Foundation仕様の定義を正とし、本決定で再定義しない。浅い統合から始められ、より深い統合を必須にしない。
- 純正アプリと第三者アプリは、原則として同じPublic APIと公開契約を使う。純正アプリは統合の参考実装となり、純正であることだけを理由とする非公開APIや権限上の優遇を持たない。
- OS管理機能などにSystem Capabilityが必要な場合は、その能力を明示的に区別し、通常のアプリ権限と同一視しない。

### `builder`製アプリのNagi連携・Action API生成（DEC-076）

`builder`がアプリを作る際、用途からNagiの自然言語操作に適した**型付きPublic Action APIの候補を自動提案**し、必要なものを承認済み開発Task／Action Scope内で実装・検証する**方式Cを確定**する（[DEC-076](DEC-076-builder-generated-action-api-integration.md)）。ユーザーは従来のGUIからも利用でき、AI連携が不要なアプリにActionを強制しない。Actionの生成・発見・登録はOS Permission／Capabilityや実行承認の付与を意味しない。正規Public Action／Service APIを優先し、限定的なGUI fallbackはDEC-027に従う。0.2の統合レベル0〜3とSDK／Action契約を再定義せず、0.3・0.4のGateや受入を変更しない。具体的Schema／自動生成方式／実装版はPROVISIONAL。

### 既存アプリの移植・再利用

- 既存のRust、C、C++アプリケーションについて、Nagi向けに移植・リメイクできる部分と必要な作業を見つけやすくし、技術的に可能な範囲で既存のportableなコードを再利用しやすくすることを製品目標とする。
- 開発者が依存関係、OS固有API・サービス契約、Nagi側で利用可能な公開契約との不足を把握し、portable coreとOS依存部分を分けて段階的に移行できるガイドや診断手段を目指す。診断は調査の手掛かりであり、互換性認定や移植工数の保証ではない。
- Nagi固有の機能を使う場合は公開SDK/API、宣言済みPermission / Capability / Policyに従う。既存コードの再利用を理由に秘密APIや追加権限を設けない。
- Linux向けバイナリの直接実行、全面的なPOSIX互換、Qt / GTK / Electron / SDL等のフレームワーク互換は約束しない。個別の実証・別途の決定なしに対応済みと扱わない。

### パッケージと配布

- アプリのパッケージを検証し、インストール・起動できる開発および受け渡しの流れを目指す。
- Storeは将来の配布先候補とするが、Storeだけを唯一のインストール・配布経路にしない。

### `builder`製アプリの保守・更新（DEC-035）

`builder`が作成して正式導入されたアプリについて、**不具合・改善点をNagiが提案し、必要なTask／Action Scopeの承認内で隔離修正・検証し、ユーザーによる内容確認と正式更新の承認を得てから、正規アプリ更新経路で適用する**方式Cを確定する（[DEC-035](DEC-035-builder-created-app-maintenance-updates.md)）。新しいPermissionやデータ移行が必要なら別途承認する。可能な限り利用者データ・設定を保護し、更新失敗時には実在する復旧経路のみ示す。修正コードのローカル反映（DEC-018）はインストール済みアプリの自動更新権限を与えない。

本方針はアプリ配布・Package基盤の製品方向を補足するもので、0.2-M21の現行契約／Acceptance、既存のインストール・サイドロード方針、0.4 Entry Gateを変更しない。更新PackageやRollbackの具体的実装・適用版は未確定。

### builder製アプリ開発Projectの長期保存（DEC-040）

正式なbuilder製アプリ開発Projectをローカル基本で長期保持し、改修時に発見・Scope付きで再開できるようにする。Projectのソースとアプリ実行物・利用者データ・Canonical Memoryを分離し、Project移行を将来の製品目標とする。保存だけでGit commit／push、Cloud backup、外部送信、共有を許可しない。形式・保持期間・容量・Git採用・移行形式・適用版は未確定で、[DEC-040](DEC-040-builder-app-development-project-retention.md)が正本。

本方針は開発者体験の将来目標を補足し、0.1／0.2／0.3 Acceptance、0.4 Entry Gate、既存Workstreamを変更しない。

### builder製アプリの初回試用・正式導入（DEC-075）

新規にbuilderが作成したアプリは[DEC-075](DEC-075-builder-app-trial-and-installation.md)に従い、安全に対応できる環境での試用、機能・検証証拠・要求Permissionの説明、ユーザーの採用判断、正規Package検証とOS Permission／Policy確認、正式導入への別途承認を経てから初回インストール・永続登録する。正式導入済みアプリの保守・更新は[DEC-035](DEC-035-builder-created-app-maintenance-updates.md)の別ライフサイクルである。

### `builder`製アプリの個人利用・共有・公開（DEC-036）

`builder`製アプリは個人利用を初期の標準とする。ユーザーが共有・公開を希望した場合に限り、Nagiはソースまたは配布Packageの内容、依存関係、個人情報・秘密情報/APIキー、個人設定・学習Memory、ライセンス、必要Permission等を確認し、必要な分離・検証を行った後、ユーザーが確認・承認した成果物を出力できる。友人等への限定共有とGitHub／Store等での一般公開は別の公開範囲として明示選択する。共有作成の許可は、実際の外部送信・公開を認可しない。受取側は独立に検証しPermissionを再承認する。元ユーザーのMemory、Permission、認証情報は移転しない。

完全な秘密情報除去、ライセンスの自動判定、特定の検査による安全性保証、Store実装を約束しない。技術形式・具体的検査・適用版は未確定であり、本決定は0.2-M21、0.3 Acceptance、0.4 Entry Gate、Workstream ownerを変更せず実装開始も許可しない。詳細は[DEC-036](DEC-036-builder-app-sharing-and-publication.md)を正本とする。

### `builder`製アプリのアンインストール・関連データ削除（DEC-041）

正式導入済みアプリの通常アンインストールではアプリ本体のみを削除し、ユーザーデータ・設定・保護ソースコード・DEC-040の開発Project／履歴を原則保持する。完全削除はNagiが対象、既知の共有参照、影響、回復可否を示し、ユーザーが削除対象を個別に選び承認した場合に限る。共有データ、作業中成果、復旧情報、他ユーザー所有・未確認データを巻き込まず、現在のPermission／Policyと正規OS経路に従う。再インストールや再ビルド、古いPermissionの復元は保証しない。保持期間、UI、実装・版割当ては未確定であり、既存Gateも変更しない。正本は[DEC-041](DEC-041-builder-app-uninstall-and-data-deletion.md)。

## PX-15 — 1.0完成基準との整合（2026-11-17）

[DEC-021](DEC-021-roadmap-0.5-1.0.md)でPX-15の**1.0実用OS B（正式対応範囲で代表的日常作業を安全・安定して完結・継続）＋将来総合OS C（対象Hardware／アプリ・利用形態を広げる）**という高水準方向をCONFIRMEDした。

**本DECの既存CONFIRMEDを維持:** 公開SDK／開発者環境・アプリ配布基盤を1.0までに整える高水準目標、Nagi Native Apps専用、一般的なIDEでの開発・段階的なLevel 0〜3連携、移植負担を抑える開発者支援、Public API／Permission／Policy、正規Package検証・導入／公開の別承認を変更しない。1.0は正式対応する開発／配布経路とTarget上の再現可能な結果を重視するが、Storeアプリを唯一の配布手段としたり、全Porting Toolkit／全Framework／高度な初心者向けIDEを一括必須としたり、HostだけのPASSをTarget実受入と認めたりしない。

**PROVISIONAL／OPEN:** 初期SDK・言語／Framework、具体的な対応Package／署名・配布経路、Developer onboarding・Compatibility／Support matrix、Target Acceptance・品質／性能の数値・正式な1.0 Release判定手順と時期。PX-15の高水準完成方針は0.1〜0.4の既存Gate・Workstream条件の変更、1.0実装／Release PASSの宣言ではない。

## 例外・禁止・安全境界

- 本決定は製品方針と目標配置を記録するもので、実装済み、Workstream開始、Acceptance通過、release activationを意味しない。
- ここに記した0.1、0.2、0.3、1.0の並びはロードマップ上の目標であり、既存の0.1 M30、0.2 M21、Workstream所有権、依存関係、activation / acceptance gateを変更しない。
- 0.1中の先行作業は、既存ルールで認められた範囲のホスト側SDK、契約、テンプレート、サンプル等に限る。既存Gateが満たされる前に、target runtime統合やtarget acceptanceを開始・宣言しない。
- 0.2ではNagi target上でのアプリ実行基盤を整える。既存の0.2-M21が定めるSDK、manifest、package、out-of-tree sampleのDeliverableとAcceptanceを維持する。0.3でのAgent連携はPublic Action契約の利用を深める段階であり、0.2のSDKやAction契約を延期・置換する意味ではない。
- 0.3ではAgentが第三者アプリを含むアプリのPublic Actionを正規経路で利用できることを目標とする。すべての呼出しは現行の権限確認、Policy、Activity等の関連契約を通り、アプリの許可範囲を越えない。
- Linux向けアプリの直接実行互換性は約束しない。必要な場合の互換レイヤーは将来の別途検討とする。
- 未公開の本決定やPrivateロードマップをPublicリポジトリへ転載しない。公開用の仕様・説明を作る場合は、別途公開範囲を判断する。

## 未確定・暫定案

- 既存アプリの優先移植対象、個別のRust/C/C++ toolchainやtarget、各OS APIとNagi公開契約の対応範囲、診断ルール・分類・数値評価、実際の移植工数、Qt / GTK / Electron / SDL等の対応可否。
- macOS、Windows、Linuxそれぞれのサポート対象バージョン、Rust target、依存ツール、署名付きビルド方法。
- C ABIの具体的な範囲、IDL、生成binding、互換性保証期間。
- CLIの正式なコマンド・エラー契約と、QEMU起動・デバッグの詳細。
- 各Public APIレベルの利用可能機能、アプリごとの権限UI、第三者向けCapability承認・失効の詳細。既に版管理された0.2仕様やsecurity contractは引き続き優先する。
- Packageの署名、更新、審査、サイドロード、Store運営の実装詳細。0.2-M21のAcceptanceや既存のside-loading方針を本DECで変更しない。
- 1.0に向けた開発者環境・配布基盤の具体的な構成、日程、完了条件。
- Linux／iOS／Android等の既存アプリ実行互換レイヤーは標準方針として採用しない。例外的な互換性を将来検討する場合は対象API・方式・適用範囲を別途決定する。
- 移植支援の対象言語／UI Framework、SwiftUI／UIKit／Jetpack Compose等との対応・変換可能範囲、AI支援の検証方式、追加リリース工数・再利用率の実測。
- 初心者向けの新規Nagi Native App制作体験を提供する高水準の方向は[DEC-071](DEC-071-nagi-native-app-creation-experience.md)でCONFIRMED。専用GUI IDEとして独立させるか、Porting Toolkitへ組み込むか、別の案内型UIとするか、名称・画面・追加言語は未確定。

### AIコーディング向けの具体化（PROVISIONAL）

以下は設計候補であり、採用する技術や実装契約は未決。

- Action、Context、Search、Activity、Checkpoint／Wayback、Workspace、Permission／Capabilityなど、既存Public APIをAI支援ツールが発見・理解できる機械可読・型付きcatalogやmanifestで表す方法。IDL、schema、生成binding、MCP等の標準は選定しない。
- AI支援ツールが安全に呼び出せるscaffold／build／test／package／validate／verifyのCLI契約、構造化診断、安定したエラー・log・traceの形式。既存のCLI／QEMU目標を置き換えず、任意のshell実行は許可しない。
- Rust／C／C++アプリの移行例、依存・API差分の診断とout-of-tree fixture。対応範囲や無変更で移行できる範囲は決めない。
- 再現可能なhost-side検証とQEMU guest acceptanceのつなぎ方。host-side成功をtarget acceptanceへ読み替えない。
- AI coding agent向けのsandbox、Capability、承認、差分確認と結果報告の具体的な設計。
- AIは既存規則どおりuser-spaceの非信頼主体として扱い、AI向け高水準syscallをカーネルに追加しない。カーネルを最小限の強制可能な権限境界に保つ方向は現行設計と整合するが、強制機構やAPIを新設する判断は本DECでは行わない。

### host-onlyで先に評価できる候補

- 現行Public APIとmanifestを説明するAI向け文書・機械可読catalogの設計。
- Rust／C／C++向け移行fixtureを使った、host-side schema／manifest validationとCLI診断の検討。
- 既存out-of-tree sampleに対する、決定論的なbuild／test／package検証の設計。

いずれも候補であり、新しい実装許可ではない。実装時は最新の0.2 Workstream registry、state、許可path、owner、および明示的なhost-only checkpointを確認する。app-sdk-contract workstreamの記録済みhost-side PASS（last_verified: 2026-09-26）をtarget/runtime受入として扱わない。

## Roadmapとの関連

- **0.1:** 既存GateとWorkstream規則の範囲で、ホスト側SDK・契約・テンプレート・サンプル等の先行準備を行える。target app runtimeの完了を意味しない。
- **0.2:** 0.2-M21等の既存定義に従い、Rust SDKを含むPublic App contract、package検証、out-of-tree sample、およびNagi target上のアプリ実行基盤を進める。既存の実装順序・Acceptanceを変更しない。
- **0.3:** Personal Agentが第三者を含むアプリの公開Actionと正規に連携できることを目標とする。権限・安全境界はDEC-001および各Public contractに従う。
- **1.0:** 公開開発者環境と配布基盤を製品目標とする。具体的な公開範囲、Storeの役割、日程、Acceptanceは別途定める。

## PDF Reader / Editorとの整合（2026-11-16）

[DEC-061](DEC-061-first-party-pdf-reader-editor.md)により、純正アプリ拡充の一環としてPDF Reader兼Editorを採用する。通常の閲覧・編集はAI非必須とし、純正・第三者共通のPublic APIで連携する。Acrobat完全互換、全機能初期実装、具体的な技術・提供版は確定しない。 具体的な連携契約・適用版はPROVISIONAL。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-04 | 純正アプリの拡充と並行して第三者の開発・参入を促す方針、一般的なPC開発環境、段階的なPublic API統合、AI非依存の基本アプリ、0.1〜1.0の目標配置を確認。 |
| 2026-11-04 | 既存Rust/C/C++アプリの移植しやすさとportable code再利用を製品目標として追加。互換対象・診断方式・工数は未確定とし、Linux/POSIX/GUI toolkitの包括互換を約束しない。 |
| 2026-11-04 | AIコーディングに最適化した開発体験と、独自カーネルの強みを活かすAI時代のOSを目指す方針を追加。技術・CLI・機械可読契約等はPROVISIONALに分離。 |
| 2026-11-05 | `builder`製アプリの保守・更新方式CをDEC-035として追加。Nagiの修正提案、Scope内の隔離検証、ユーザー確認後の正式更新、データ保護と可能な範囲の旧版復旧をDEC-017の配布方針に接続した。 |
| 2026-11-05 | `builder`製アプリ共有・公開方式CをDEC-036として追加。個人利用を標準とし、範囲選択・内容確認・明示承認後の出力、限定共有と一般公開の区別、受取側の独立検証を定めた。 |
| 2026-11-05 | builder製アプリのアンインストール・関連データ削除方式CをDEC-041として追加。通常アンインストール時のデータ保持、完全削除の対象別選択・承認と安全境界を定めた。 |
| 2026-11-12 | DEC-042により独自カーネル選択の前提だけを限定置換。Linux採用方針は確定、製品・安全境界は維持、実装詳細と切替時期は未確定。旧本文・履歴は保持。 |
| 2026-11-13 | Nagi Native Apps専用の標準アプリ環境と、iOS／Android等からの低負担移植・追加リリースを促進するSDK／UI／AI開発支援を製品目標として確定。無変更互換性・工数保証・実装版は未確定。 |
| 2026-11-13 | 移植支援対象の優先順位を整理：PCはWindows／Linuxを重点化。将来のモバイルはAndroidを優先検討、macOS／iOSは当面後順位。Nagi Native Apps専用とDesktop-firstは維持し、直接実行互換・成果／時期は保証しない。 |
| 2026-11-16 | DEC-061の純正PDF Reader / Editor方針への参照を追加。既存の公開API・AI非必須・権限境界と整合させ、実装版・既存Gateは変更しない。 |
| 2026-11-17 | PX-15の1.0実用OS B＋将来総合OS Cの高水準完成方針と、既存1.0公開開発者環境・アプリ配布基盤目標を整合。Nagi Native／段階的Public API・既存IDE・移植支援、HostとTargetの試験区別、正規Package／署名・導入・公開の別承認は維持。対応SDK・開発者経路・数値・個別提供版はPROVISIONAL。 |

| 2026-11-15 | DEC-071として、Porting Toolkitの既存アプリ移植に加え、初心者がローカルLLMを使い自然言語から小規模な新規Nagi Native Appを作り、試用・導入・共有／配布する純正体験の方向を確定。製品形態・Toolkit連携・技術・版は未確定。 |
