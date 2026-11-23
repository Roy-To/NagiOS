# DEC-099 — Installer / First Setup：Linux機構を利用するNagi専用GUI方式B

**状態:** CONFIRMED（Foundation #30の方式B：Linuxの既存ディスク／インストール関連機構を活用し、Nagi独自GUIでOSインストールと初回設定を案内する。ローカルLLMによる高度な設定判断を標準構成・前提にせず、AIがなくても基本セットアップできる）／PROVISIONAL（GUI・採用部品・ディスク／暗号化／Owner初期化の実装詳細・対応機器・適用版・実機Acceptance）\
**最終合意日:** 2026-11-16\
**適用候補:** Nagi OS Foundation #30。Release割当て・実装開始条件は未確定\
**関連:** [Foundation Checklist #30](NAGI_OS_FOUNDATION_CHECKLIST.md#f30)、[DEC-043（Linux＋Nagi専用ユーザーランド）](DEC-043-nagi-userland-independent-os.md)、[DEC-079（read-only System／User Data分離・A/B更新）](DEC-079-system-image-readonly-ab-data-separation.md)、[DEC-009（通常GUI／AIがないときの操作）](DEC-009-nagi-first-home-desktop.md)、[DEC-017（Nagi Native Apps）](DEC-017-third-party-developer-ecosystem.md)、[DEC-020（決定論的なService／非特権AI）](DEC-020-ai-os-interaction-path.md)、[DEC-078（Boot System）](DEC-078-boot-platform-entry-and-common-nagi-startup.md)、[0.2実装仕様 §16](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[Roadmaps](roadmaps/README.md)

## 目的とユーザー体験

Nagi OSを導入するユーザーが、端末・保存先を把握し、取り返しの難しい操作を自分で確認しながら初回セットアップを進められる製品導入体験を提供する。LinuxカーネルとNagi専用ユーザーランドの構成を前提とし、OSの導入操作をローカルLLMの能力・推測・可用性に依存させない。

## 確定した製品方針（CONFIRMED：B）

1. **Linuxの成熟した低レイヤーを活用する。** ディスク・partition等の汎用操作にLinuxの既存機構と選択したOSSを利用し、OS独自Installerの画面・進行・検証・エラー表示はNagiのユーザーランドで統合する。具体OSS、partition tool、filesystem、Installerの内部実装はまだ採用しない。
2. **Nagi専用GUIで初回導入と設定を案内する。** 導入先の選択、変更予定の内容、進行／失敗状態、初回Owner・Locale等の基本設定、必要なRecoveryの案内を、ユーザーに分かる形で提供する方向とする。対象画面・操作順序・手順の必須項目は後続設計とする。
3. **AI推論に依存しない。** Installerによる対象確認、危険操作の可否判断、既定の選択、設定適用、成功確認は、再現可能な通常のOS処理・決定論的検証で扱う。ローカルLLMに高性能なハードウェア診断・設定選択・障害修復を要求しない。現段階でCのAI支援Installerを正式製品目標として採用しない。ただし将来の補助説明機能等を永久禁止する決定ではない。
4. **基本のセットアップはOffline-first。** インターネット接続、Cloud account、AI／LLM推論を通常の初回導入成功の絶対前提にしない。追加取得を要する個別機器・配布方式などの具体的な例外と、セットアップ可能な範囲は後続検証で確定する。
5. **破壊操作には明示確認を要する。** 対象ディスク、消去・上書きの影響、User Dataへのリスクを提示し、ユーザーが意図した対象と操作を明示確認する前に、データを破壊する導入処理へ進まない。曖昧なディスク識別や未確認の書き込み先をAIの推測で補完して実行しない。実際の対象照合・警告・中止・Rollback可能範囲は後続設計とする。
6. **SystemとUser Dataの既存の分離方針を維持する。** DEC-079のread-only System／User Data分離、A/B更新・Recovery、AI Modelの論理的分離と整合させる。これはディスク上のpartition数・配置・暗号化や、インストール失敗からの完全復旧・既存データ保存を保証する決定ではない。
7. **OS InstallerとPackage／Appインストールを混同しない。** 0.2 §16のPackageの検証、inventory、atomic change、Recovery等は参照できるが、host上のPackage／Installer契約が実機へのOS書込・暗号化・Owner初期化に成功した証拠ではない。対象機器上での検証と権限・結果の記録を別途求める。

## Bの機能範囲（製品方向、具体UIはPROVISIONAL）

| 領域 | 製品基本方針 | 後続設計で決める内容 |
|---|---|---|
| 起動・導入先 | 対応する媒体からNagi GUI Installerを開始し、ユーザーが対象・影響を確認できる方向 | 対応Boot／USB／Disk媒体、対象端末、ライブ環境方式、ファームウェア互換性 |
| OS／Data配置 | Linuxの低レイヤーとNagi独自の製品設計を組み合わせ、DEC-079の論理的分離を守る | Partition、Filesystem、Encryption、System slot、Data／Modelの実際の配置、容量 |
| Owner／言語等 | Nagiの初回Ownerと基本的なLocale／設定を案内する | Identity初期化／認証方式、Timezone、言語、ネットワーク、Recovery keyの操作UI |
| 失敗・中断 | 未実施・進行中・成功・失敗・不明を区別し、危険な再試行とデータ損失を防ぐ | Durable progress、Commit／Rollback境界、電源断・容量不足・再開／復旧メディア |
| ネットワーク・AI | Offline-first／LLM非必須の基本導入を維持 | 追加Component取得が必要な場合の明示選択・代替、適用対象・対応外表示 |

## 保護・例外

- ユーザーの確認は実際の破壊対象の照合と、現在のOS権限・Policyの代わりにならない。危険な処理の実行と、単なる画面上の選択を区別する。
- Installerの表示・ログで資格情報・回復鍵などの秘密を不要に収集・公開しない。鍵生成・格納・復旧・暗号化仕様の選択は未決定。
- 検証されていないディスク、データ移行、復旧経路を「安全」または「成功」と表示しない。中断後の結果不明や部分的な書き込みは正確に扱う。
- OS本体のインストールとSystem Update、App Package導入／更新、モデルファイル導入は異なる操作・認可・Rollback契約を持ち得る。共通実装部品の再利用を認可の統合と取り違えない。

## 未確定事項（PROVISIONAL／OPEN）

- 最初にサポートする導入先（PC／Raspberry Pi等）、起動媒体／Firmware、他OSとの共存、既存データを保持した導入の可否。
- Installerで利用するLinux tooling／OSS、GUI実装・翻訳／Accessibility、動作に必要な最低構成、記憶媒体の同定と物理ディスクの変更権限。
- Partition構成、暗号化の標準化・鍵Recovery、User Data・Model Storeの配置、機器別Boot／A/BとRecovery媒体の実装。
- 初回Owner／認証・Device Trust／Locale／Timezone／Offline network setupの順序、具体初期値・Profile／Permission。
- コミット前の再確認と不可逆操作の警告、失敗時の保持・中断・再試行・再起動、データ移行・復旧の検証範囲。
- 対応端末の実機検証と合格条件、Release／Activation Gate、開発担当・時期・提供Version。
- CのAI支援Installerは今回の採用対象外。将来別途検討する場合も、危険操作の決定論的な確認・人間の明示承認を前提とする。

## 2026-11-24 — 参考事例の高度なインストール最適化は今回見送り

**CONFIRMED（追加案の扱い）:** Omarchyの事例を参考とする高度なインストール高速化・端末別自動設定の新規提案は、今回のNagi追加開発案としては採用しない。複雑さが大きいため、今回の優先実装・新規要件に含めない。

**既存のCONFIRMEDは維持:** 本決定はDEC-099のLinux機構を利用するNagi専用GUI Installerの基本方針を撤回せず、実機へのOS導入に必要な既存の検証・安全確認を省略しない。Installerの内部実装方式、対応機器、Release割当ては引き続きPROVISIONAL。

## Roadmapとの関係

Linux＋Nagi独自ユーザーランド、Desktop first、System／User Data分離と既存0.2 Install／Update・Release gateの方向に接続する。方式Bを決めたことは物理ディスクへの導入成功や、Linux targetのOwner初期化、Recovery検証、対応端末・提供版の確定ではない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-24 | Omarchy参考の高度なインストール高速化・端末別自動化の追加提案を今回は見送り。既存のNagi専用GUI Installer（DEC-099）の採用済み方向は維持。 |
| 2026-11-16 | Foundation #30についてユーザーが方式Bを承認。Linuxの低レイヤー＋Nagi専用GUI Installerを標準とし、ローカルLLMで高度なInstaller判断を行う方式Cは採らず、Offline-first／AI非必須での基本セットアップ、危険操作の明示確認を維持。OSS・GUI・暗号化・実機・版は未確定。 |

## 関連決定の参照注記（2026-11-21）

- [DEC-117](DEC-117-user-data-encryption-and-lost-device.md) — 初回セットアップで回復キーを表示して端末外での保管を強く促し、パスワード・PIN・回復キーをすべて失うとデータを復元できないことを伝える。
