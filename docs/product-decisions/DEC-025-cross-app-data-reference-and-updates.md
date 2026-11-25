# DEC-025 — アプリ横断の共通参照と関連データ更新提案

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（既存方式A：共通参照・更新提案・承認済みTask内の限定更新、PX-12の案内型転送B＋将来の高度ワークフローCという高水準体験との整合）／PROVISIONAL（参照／Transfer契約・対応アプリ・詳細UI・fallback制御・性能・実装版）  
**最終合意日:** 2026-11-17（PX-12との整合を追記。方式Aの既存合意を維持）  
**対象:** Nagiの作業・データ中心体験におけるアプリ間Resource参照・変更追従  
**関連:** DEC-001、DEC-003〜005、DEC-008、DEC-014、DEC-017、DEC-020、DEC-023、Public 0.2 Platform Foundation仕様、Private 0.3改訂ロードマップ

## 目的

異なるアプリの資料を、利用者の目的・Task・Workspaceのもとで関係付ける。ある資料が変更された場合に、関係する成果物をユーザーが適切に更新できるよう支援する。アプリを横断するデータの正本をOSへ強制的に集約しない。

> **2026-11-16 Storage／Filesystem後続決定:** [DEC-088](DEC-088-linux-filesystem-nagi-storage-api.md)で、実際の保存下層はLinux標準Filesystemを用途別に利用し、Nagi Storage／Files APIで上位契約を保持するB案をCONFIRMED。元アプリ・Providerが保持する分散正本、Resource／Revision／Provenanceの現在確認、ファイルのpath／inodeと論理IDの分離、変更提案を標準とする本DECの境界は不変。Filesystem snapshotが自動的な関連成果物更新やWaybackを意味するわけではない。具体FS・mapping・実装版は未確定。

## 確定した仕様

### 共通参照モデル（思想⑦・選択A）

- **共通参照・分散正本:** NagiはResource／Document／Object、Revision、Provenance等の共通契約を基に、アプリをまたぐ許可されたデータの意味・関係・参照を扱う。実体の保存・編集の正本は元アプリ、Provider、利用者の保存先に残せる。
- **必要時の内容取得:** 関連する識別子・metadata・出典を用い、実体や詳細は必要なときに正規Public API経由で取得する。Resource IDや推定された関係だけでは閲覧・操作の権限を得ない。権限・情報分類・所有範囲・Revision・参照先の実在性を現在時点で確認する。
- **従来操作・第三者:** 個別アプリの通常GUIとAIを使わない機能を維持し、純正・第三者は同じ公開APIと段階的な統合を利用する。全アプリへの単一DB・保存形式・AI対応を強制しない。
- **既存の検索とMemory:** DEC-005で許可したAlbert本文のローカル保存・索引等は保持する。派生Context／Index等について既存の分類・削除・忘却・Retentionを維持する。推定関係と明示・検証済み関係は区別する。

### 関連データの変更検知と更新提案（思想⑧・選択A）

- **標準は検知と提案:** Nagiは正規の公開Event／Provider契約で把握でき、現在参照を許された元データの変更を検知した場合、関連する資料・成果物への影響を確認し、必要であれば更新案をユーザーへ提案する。**変更を検知しただけでは、関連成果物を自動編集・同期しない。**
- **事前承認済みTaskの例外:** ユーザーが変更対象、操作、目的、権限範囲を含むTaskを明示的に承認している場合、その範囲内の関連資料は新たな都度承認なしで自動更新できる。実行時の現在のPermission／Capability／Policy、Task Scope、対象Resource／Revisionを正規の型付きPublic Action／Service APIで再検証する。
- **変更の安全性:** 元データや更新先のRevision不一致、競合、出典不明、推定関係の不確実さ、処理不能・権限不足、Task範囲外の変更は成功扱いせず、更新を保留して提案・確認に戻す。ユーザーが明示した手動変更を勝手に上書きしない。
- **透明性:** 提案では、可能な範囲でどの元データが変わり、どの成果物が影響を受け、何を更新するかを示す。実行したActionと成功・部分成功・失敗はActivity／Taskへ正確に記録する。外部送信や不可逆操作は、別の既存承認・Policyの条件に従う。
- **イベントと負荷:** Eventは自動編集や無制限なLLM推論の許可にはならない。必要に応じて非LLMの更新検知・参照整合確認を優先し、DEC-008の控えめな通知とDEC-020のイベント駆動・資源効率方針を尊重する。

## 禁止・安全境界

- データ間に関係があるという理由だけで、別アプリへのアクセス、機密情報の露出、データ共有や外部送信、承認外の編集・同期を認めない。
- Task承認がない場合の原則は更新案の提示であって反映ではない。明確に停止・取消されたTaskを関連データ変更のみで再実行しない。
- 仕様の記録は0.1 M30、0.2 M17／M21、0.3 Implementation Spec／Activation、Workstream権限・既存Acceptanceを変更せず、実装開始・実装完了の証拠としない。

## 未確定・暫定案

- 関係の推定・登録・解決のSchema、参照Indexの方式、Eventの種類、差分・影響範囲の検出アルゴリズム、通知頻度とUI。
- 自動更新のTask承認形式、編集の可逆性・競合解消・複数アプリ間の部分成功・再実行抑止・Revision検証の具体的契約。
- 対応アプリ、負荷上限、各Releaseへの割当、実装担当、Acceptanceと性能目標。

### 2026-11-17 — PX-12のSmart Paste利用体験との整合

**CONFIRMED（追加した高水準方針）:** [DEC-056](DEC-056-smart-paste-semantic-transfer.md)のPX-12節で、B＝コピー元／先が対応する方法と制限を案内し、ユーザーが通常コピー・構造化転送・連動参照などの利用可能な方法を選べる体験を基本とし、将来C＝許可されたProject／Task文脈・Provenanceと変更の影響を扱う高度なアプリ横断Workflowへ発展する方向を採用した。

**本DECの方式Aを維持:** 元App／Provider／利用者の保存先に正本を残せる**共通参照・分散正本**、Resource／Revision／ProvenanceとPermission・分類・Ownerの現在確認、**元データの変更を検知しただけでは成果物を更新せず、更新提案を標準とする**ことは既存CONFIRMEDのまま。対象・操作・目的・Scopeが事前承認されたTask内の限定自動更新だけを例外とし、正規Public Action・現在のRevision・権限を実行時に検証する。貼り付けや参照の選択をTask承認や他アプリ更新の包括同意としない。Source失効・Revision競合・権限不足・不明／部分失敗を成功扱いしない。停止Taskの自動再実行も禁止。

**PROVISIONAL／OPEN:** 対応アプリ／Objectの初期組合せ、静的コピーか参照かの既定、更新提案や関係表示の詳細UI、Transfer Schema、Fallbackの具体制御・技術／性能・提供Version・Acceptance。既存のPermission／Privacy／Cloud共有条件・Release／Activation Gateは変更しない。

## Roadmapとの関連

- **0.2:** 既存Resource／Object／Revision／Public Action／Permission等の基盤を維持する。
- **0.3:** 認可済みContextとアプリ横断ActionによるPersonal Agent構想に接続する。
- **0.4+:** 追加Agentも同じ権限・承認・分散正本の境界を尊重する。

## PDF Reader / Editorとの整合（2026-11-16）

[DEC-061](DEC-061-first-party-pdf-reader-editor.md)により、PDFを参照・抽出・引用する連携にも共通参照・分散正本・Revision / Provenanceの原則を適用する。Nagi Project / Smart PasteやPDFの派生資料も権限・機密分類を継承し、変更検知のみでは関連成果物を自動編集しない。承認済みTask内の例外は本書の既存境界を維持する。 具体的な連携契約・適用版はPROVISIONAL。

### 2026-11-16 — Projectフォルダを参照の作業単位として利用（CONFIRMED）

[DEC-055](DEC-055-project-folder-cross-app-context.md)で、通常フォルダのProject指定や他の保存先のResource参照を組み合わせ、Project単位の関連資料と横断AI作業を扱う方向を確定した。元データの分散正本、Provenance、変更検知時の**提案を標準**とする本DECのルールを維持し、Project所属のみでアクセス許可・自動更新を付与しない。具体的なGraph／Schema／UIは未決定。

> **2026-11-16 Data Persistence後続決定 — 分散正本と共通保存契約:** [DEC-090](DEC-090-common-persistence-contract-distributed-authority.md)で、Nagiが保存・移行・復旧に関する共通Persistence契約を持ち、App／Provider／利用者の元データ正本は用途別に保持できるB案をCONFIRMED。共通参照と元正本の分離を維持し、Resource／Revision／Provenanceと現行Permissionなしに内容を取得・更新しない。全Appを一つのDBへ統合する決定ではなく、具体DB／Migration・Linux受入は未確定。

## 決定履歴

| 日付 | 決定 |
|---|---|
| 2026-11-05 | 思想⑦ A：共通参照モデル型。元アプリ・Providerの正本を保持しつつ、OSが意味・関連性を扱う方針を確定。 |
| 2026-11-05 | 思想⑧ A：関連データの変更を検知して更新案を提案する方式を標準とする。既に承認済みのTaskの対象・権限範囲内なら自動更新を認める。 |
| 2026-11-16 | DEC-061の純正PDF Reader / Editor方針への参照を追加。既存の公開API・AI非必須・権限境界と整合させ、実装版・既存Gateは変更しない。 |

| 2026-11-16 | DEC-055のProjectフォルダ構想を、共通参照と関連更新提案の利用形態として連携。既存の更新承認・正本分離・Scope維持。 |
| 2026-11-17 | PX-12の案内型Smart Paste B＋将来の高度アプリ横断Workflow Cを、DEC-056に記録した高水準体験として本DECへ整合。元の共通参照方式A・変更検知／更新提案方式A・承認済みTask内だけの限定自動更新を維持。詳細UI・初期対象・Transfer／fallback制御・技術・性能・版はPROVISIONAL。 |

## 2026-11-25 — 共通データ契約と内部保存・転送の分離

**CONFIRMED（高水準方針）:** アプリの内部保存形式・実装と、アプリ間で利用する公開データ契約を分離する既存方針を具体化する。通常のコピー・連携ごとに共通形式のJSONファイルを永続的に大量複製することを前提にせず、RAM消費と処理速度・データ量を考慮して転送方式を選ぶ。機械可読な公開契約と実際の内部通信表現は区別し、内部処理をJSON文字列に固定しない。既存の分散正本・正規Public API・現行Permission・分類・所有Scope・Provenanceと変更検知時の提案標準、承認済みTask内だけの限定自動更新を維持する。

**PROVISIONAL（設計候補）:** JSON Schema等の公開定義、Rust構造体・軽量バイナリ・IPC等の実行時表現、小データの直接転送、大データの参照・遅延取得・stream・共有メモリ・必要時の保護された一時領域、コピー時点のRevision／内容を保持する仕組み（取得不能なら明示し、静的コピーを暗黙に現在版参照へ置換しない）、連動参照の永続化とReference Graphの責務、転送サイズ・RAM・一時領域の予算、型・拡張・fallback・API分類・Schema Version・移行・具体保存方式・提供版。連携用識別子は権限ではなく、Persona／Secret・外部由来の情報を指示としないDEC-113・現行認可・出典の境界を維持する。対応形式や実装開始、既存Gateの変更を確定しない。

変更履歴: 2026-11-25 — 共通契約と内部保存／通信の分離、連携による不要な永続JSON複製を前提としない方向、資源効率を考慮する方針を合意。具体形式・転送方法はPROVISIONAL。
