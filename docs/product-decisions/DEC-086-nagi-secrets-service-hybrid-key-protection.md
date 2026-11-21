# DEC-086 — Nagi Secrets Serviceとハードウェア・ソフトウェア両立型の鍵保護

**状態:** CONFIRMED（C案：Nagi独自のSecrets Service・Identity／Permission統合、対応機器でTPM／TEE等を優先、非対応機器でも安全なローカル保管と回復経路、鍵の用途／User／Profile分離、認可経由の利用・秘密情報非露出・更新／失効・クラウド自動預託なし）／PROVISIONAL（具体暗号・KDF・鍵種ごとの回復／TPM対応・実装・提供版）
**最終合意日:** 2026-11-16
**適用範囲:** Nagi OS Foundation Checklist #10「Secrets / Key Management」（検討束B2）。Linuxカーネル＋Nagi専用ユーザーランド上の秘密情報・鍵の生成／保管／利用／更新／失効／回復の高水準方針
**関連:** [DEC-014 Workspace移行／暗号化Export](DEC-014-workspace-state-checkpoints.md)、[DEC-066 Device Connectivity／Trust](DEC-066-device-connectivity-lan-first.md)、[DEC-068 Nagi Sleep](DEC-068-nagi-sleep-resume-recovery.md)、[DEC-079 System Image](DEC-079-system-image-readonly-ab-data-separation.md)、[DEC-083 Identity](DEC-083-nagi-identity-linux-credentials-binding.md)、[DEC-084 Permission／Sandbox](DEC-084-nagi-permission-linux-sandbox-enforcement.md)、[DEC-085 IPC／Broker](DEC-085-linux-transport-nagi-service-broker.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[Roadmaps](roadmaps/README.md)

## 目的とユーザー体験

パスワード、API認証情報、アプリ／サービス用Secret、デバイス／ストレージ／パッケージ等に用いる暗号鍵を、アプリやAIへ無条件に露出させず、NagiのIdentity／Permissionに結び付く信頼された管理経路で扱う。TPM／TEE等がある端末のハードウェア保護を生かしつつ、対応しない端末でもNagi OSをローカルで利用できる構成とする。

## 確定した製品方針（CONFIRMED）

1. **C案：Nagi独自Secrets Serviceによる統括を採用する。** Nagiが秘密情報・鍵の用途、所有・利用範囲、保護状態、利用・更新・失効等の管理責務を担う。NagiのUser／Profile／Session／App／Task、Permission／Capability／Policyと結び付け、Linuxの保護機構や適切な暗号ライブラリを選択活用できる。AI推論を鍵管理・認可の必須経路にはしない。
2. **対応機器ではTPM／TEE等のハードウェア鍵保護を優先する。** 利用可能で要件に適合するハードウェア保護を優先的に活用し、鍵の無用な外部露出を防ぐ構成を目指す。ただし特定チップ・仕様・機種の対応を一律に保証しない。ハードウェアの存在だけで全てのSecretが自動的に安全だと判断しない。
3. **非対応機器では安全なソフトウェア保管とローカル回復手段を用意する。** 暗号化したローカルKey Storeを、必要に応じてユーザーのパスフレーズ等を利用する保護経路で扱う。TPM搭載をNagi OSの一般利用の絶対条件にせず、ハードウェア方式との差と残るリスクを正しく示す。
4. **鍵の用途・所有者・Profileなどを分離する。** OS／Device／User／Profile／App等の用途・権限境界と鍵のライフサイクルを明確にし、ある目的で使える鍵や資格情報を別の目的・他Profileへ無断流用しない。個別鍵の生成・ローテーション・失効と必要な消去を一元的な方針に従って扱えるようにする。具体的な鍵階層・全種類での回復可否は未確定。
5. **AI／アプリに秘密値そのものをむやみに渡さない。** Nagi Secrets Service等の認可済み経路で「認可された操作」を実行できるようにし、可能な場合は秘密鍵・資格情報を呼出元の生データやAI Contextへ直接渡さない。秘密情報へのアクセスが必要な例外も現行Permission／Policyと用途を評価し、AI・アプリがOwner／System権限を勝手に得ることを認めない。
6. **秘密情報のログ露出・自動クラウド預託を防ぐ。** 診断／Activity／Error／AI Context等に鍵・パスワード・Tokenを無用に残さず、クラウドへの鍵預託・秘密情報の外部送信を既定で行わない。別途の明示許可・Privacy／Policyを迂回しない。ユーザーが自分で管理する正規のバックアップ／復旧方法は否定しない。
7. **鍵紛失・失効・回復の限界を説明する。** 回復可能なSecret／暗号化データと回復不能なものを区別し、鍵・パスフレーズ・必要な復旧手段が全て失われた場合に復号できない可能性を事前に説明する。復旧を保証する裏口や、全Secretの自動復元を約束しない。復号成功とOS上のIdentity／Permission／Owner権限を区別する。

> **2026-11-16 System Trust後続決定:** [DEC-087](DEC-087-tiered-system-trust-and-signature-verification.md)で、対応機器のHardware root of trustと非対応機器のSoftware署名検証を使い分ける段階的信頼基盤B案をCONFIRMED。本DECのSecrets Service／鍵保護は信頼の起点・署名検証・署名者の更新／失効の設計に連携するが、秘密保管に成功したことは起動の信頼性や権限付与の保証ではない。製品／開発署名鍵の具体運用・Secure Boot・rollback protectionは未決定。

## 安全境界・既存仕様の尊重

- **DEC-014:** Workspace移行パッケージの既定は、ユーザー指定パスフレーズによるオフラインファーストExport／Importと、ユーザーが別保管する復旧キー等の許容。鍵預託は既定で行わず、復号に成功しても現在のImport先User／Profile／Owner／Permission等の再評価を省かない。この仕様をTPM必須や自動クラウド復旧へ変更しない。
- **DEC-066／068:** Device Trust・端末間鍵交換、Hibernate／Sleepのストレージ暗号化と復帰に必要な鍵は用途・脅威・回復条件が異なる。Workspace Export用パスフレーズをそのまま無条件に使い回したり、hibernateの対応／全機種の鍵復旧を今回確定したりしない。
- **DEC-083／084／085:** Nagi Identity／Sessionを正本とし、Permission／Sandboxで秘密取得経路を強制する。Service BrokerのCaller認証が成功することと、当該Secretへの権限があることは別。Linux UID／rootや同じProfileという理由だけで鍵を列挙・使用させない。
- **DEC-079:** OS A/B更新・User Data分離と矛盾しない鍵の継続・復旧を考慮する。OS imageロールバック、データバックアップ、暗号鍵の回復は異なる契約であり、成功を混同しない。
- **#11との分離:** 本DECはSecret保管・利用の製品方針。Boot／System／Appの署名・Trust Anchor・Secure Boot・Owner override・Developer Mode・Rollback防止の具体的な信頼チェーンはFoundation #11で検討する。本決定から特定のSecure Boot構成や製品署名鍵の運用方法を推定しない。
- **受入・提供時期:** 旧独自kernelの署名・Credential／Trust資産をLinuxで動作確認済みと扱わない。Nagi Linux imageでの秘密保護・権限拒否・復旧の実装試験、対応機器、Release／Activation／Acceptance Gate・担当を今回確定・変更しない。

## 未確定事項（PROVISIONAL／OPEN）

- Key Storeのサービス配置・API／Scope、権限の付与・取消／再認証、秘密情報を返す必要がある例外、監査とログの安全なメタデータ。
- 利用する暗号アルゴリズム・KDF・鍵生成器・鍵階層・鍵ラップ方式・保存先、メモリ中の保護・破棄、更新／失効・鍵漏洩後の復旧。
- TPM／TEEの対応条件、ハードウェア鍵の利用・回復、非対応機器用パスフレーズ／Key Storeの具体保護方式・回復UI。
- 用途別（System／Device／User／Profile／App／Package／Storage／Export／Hibernate／Pairing等）の鍵所有・回復可能範囲・全喪失時の振舞い・バックアップ、端末移行。
- Linux kernel／userlandのSandboxとの連携、実機試験・攻撃モデル／性能、導入順序・提供版。

## 技術設計への引渡し（REVIEW NOTE）

「TPM／TEE搭載」「非搭載」の代表Linux targetに対し、User／Profile／App別の秘密のアクセス許可と拒否、ログ／AI Contextへの露出防止、鍵のローテーション／失効・漏洩／喪失・復旧、機器更新／OS A/B切替時の継続性を別々に試験する。生秘密を外部へ渡すことが避けられる用途では、秘密参照／認可済み代行操作APIを検討する。ハードウェア保護の有無だけで安全性を認定せず、構成と実測に基づき対応範囲を示す。

## Roadmapとの関連

Foundation B2 #10の高水準仕様。0.2のIdentity／Permission／Platform Service、0.3以降のProfile／Memory／Agent利用に接続するが、バージョン・実装済み状況・Linux target受入・既存Release／Activation／Acceptance Gateは変更しない。全35項目の議論を実装開始の新たな一律Gateにも用いない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #10のC案を「CでOK」と承認。Nagi独自Secrets Service、TPM／TEE等の対応機器ではハードウェア保護を優先、非対応機器でも暗号化ローカル保管と回復経路、Identity／Permission連携、AIへの秘密値非露出、鍵の用途／所有者分離・更新／失効・ログ秘匿・クラウド自動預託なしを基本方針とし、具体的暗号／KDF・TPM・回復技術・適用版を未確定とした。 |

## 関連決定の参照注記（2026-11-21）

- [DEC-117](DEC-117-user-data-encryption-and-lost-device.md) — User／Profileごとの鍵の分離をMemoryとユーザーデータの暗号化に適用する。MemoryとProfileは常時暗号化し、回復キーは端末外での保管を案内する。
