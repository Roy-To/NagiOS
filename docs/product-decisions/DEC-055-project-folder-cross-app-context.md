# DEC-055 — Projectフォルダを中心とする純正アプリ横断AI体験

**状態:** CONFIRMED（既存Project指定・Resource参照／横断Task・ProjectとWorkspaceの分離・継続、PX-03の統合作業ビューB＋将来の文脈理解型作業継続Cという高水準方針）／PROVISIONAL（名称・Project管理の詳細UI／関連付け・一覧・検索・復元導線／データモデル・技術・性能・提供版）  
**決定日:** 2026-11-17（PX-03追加。2026-11-16までの既存決定は維持）  
**対象:** Nagi Files／純正アプリ／Memory／Task／Workspaceの連携。実装版とActivation時期は未決定。  
**関連:** [DEC-003](DEC-003-memory-classification.md)、[DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-005](DEC-005-files-albert-memory-lifecycle.md)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-012](DEC-012-workspace-naming-organization.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-023](DEC-023-work-data-centric-os.md)、[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-040（builder開発Project）](DEC-040-builder-app-development-project-retention.md)、[純正アプリ実装仕様](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[0.3 Revised Roadmap](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[0.5–1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

## 目的

Nagiのアプリケーション横断型AI体験を、利用者にとって直感的な**プロジェクトフォルダ**から始められるようにする。ユーザーは企画・仕事ごとのファイルや参照をまとめ、Nagiに「このProjectの販売予測と市場調査をもとに企画提案を更新して」のように依頼できる。Nagiは許可されたResourceを横断し、元資料と成果物の関係・出典・変更履歴を維持する。

## 合意事項（CONFIRMED）

1. **普通のフォルダから始められるProject:** 利用者は既存の通常フォルダをProjectとして指定できる。Nagi専用の新規ファイル形式や特殊な保存場所を全ファイルに強制しない。Projectは日常のフォルダ操作を損なわない付加的な仕事の単位である。
2. **Project単位の純正アプリ横断:** 利用者はWriter文書、Sheets表、Slides資料、画像、Albertの参照、Notes等をProjectの作業対象としてまとめ、許可された範囲で複数純正アプリのContextと型付きActionを利用した作業をNagiに依頼できる。初期の実証は純正アプリを中心に検討し、既存DEC-023／025と共通Public契約による将来の第三者アプリ参加を妨げない。
3. **実ファイルと参照の両方を扱う:** 利用者がフォルダへ入れたファイルに加え、元の保存先を保持したまま他所のResourceをProjectの関連資料として登録できる。参照登録は無条件の複製、移動、内容の取り込みを意味しない。資料実体の正本は元のアプリ／Provider／保存先に残せる。Projectの利用に新しい専用ファイル形式への変換を強制しない。
4. **関連資料の変更追従:** Sheetsの表からSlidesのグラフを作るなど、Project内の資料間の出典・参照関係を扱い、元資料の変更を正規のEvent／Provider契約により検知できた場合には影響する成果物の更新案を示す。既定は**提案**であって、自動同期・無承認の編集ではない。事前承認済みのTaskの対象・Action・Scope内に限り、DEC-025に従い実行時再検証の上で限定的な自動更新があり得る。
5. **Projectごとの文脈と継続:** プロジェクトの目的、関連資料、明示的な方針、作業の経緯など、許可され保持できる情報をProjectの文脈として扱い、後日に「前の企画の続きを」と依頼した場合の候補発見・作業継続を助ける。Project単位の文脈やMemoryはDEC-003／004の分類、所有・Scope分離、由来、忘却を維持し、ユーザーの明示的な指定を優先する。
6. **ProjectとWorkspaceは別概念:** **Projectは「何について作業するか」（長く使う資料・関連性・文脈）、Workspaceは「どう作業するか」（ウィンドウ配置・アプリ状態・復元用チェックポイント）。** 一つのProjectを複数のWorkspaceから扱える。Projectの保存をWorkspace snapshotや原本バックアップと同一視しない。Workspace復元で停止中のAI Taskを自動再開しない。
7. **通常のGUIとユーザー主導を維持:** Projectを指定しなくても、従来のFiles／各アプリ操作は利用できる。Projectの命名、資料の関連付けと採用する更新案はユーザーの明示選択を尊重する。AIの推定をユーザーが示した関連・修正より優先しない。

## PX-03 — Project中心の統合作業ビューBと将来の高度な文脈理解・作業継続C（2026-11-17）

### CONFIRMED — B案を基本とする

**Project中心の統合作業体験**を採用する。ユーザーがProjectを開いたとき、そのProjectの目的や許可された資料・外部Resourceへの参照、関連Taskとその実際の状態、関連する複数Workspace、直近の作業・再開候補へ、分かりやすく一元的にアクセスできる入口を設ける。Projectに関連するWorkspaceをユーザーが選び、従来のWorkspace操作・復元へ自然につながる体験とする。

これは**表示・発見・操作導線を統合する高水準方針**であって、ProjectとWorkspaceの正本データ、Workspace snapshot、ファイル原本、Task Log／Activity、長期Memoryを一つの保存対象へ統合するものではない。Projectを作成・指定しなくても通常のFiles・各アプリ・Workspaceを利用できる。Project画面の具体名称・配置・一覧・操作手順やProjectとWorkspaceの対応付けの仕組みは確定しない。

### CONFIRMED — 将来C案へ発展させる方向

B案のユーザー主導の入口を保ち、許可されたProject文脈、資料間の出典・参照関係、Taskの状態、Workspaceの保存履歴・作業履歴を踏まえて、**関連資料や次に再開するWorkspace／作業候補をより適切に発見し、根拠と実際の利用可能性を示す高度な文脈理解・作業継続体験**へ将来発展させる。[DEC-065](DEC-065-nagi-handoff-work-continuation.md)のNagi Handoffなど、別端末で対応する文書・意味的な作業状態を継続する既存方針と両立・連携できる方向とする。ただし連携の具体方法、適用端末・Version、自動同期・完全な復元は今回採用しない。

### 変更しない既存CONFIRMED／安全境界

- **[本DEC](DEC-055-project-folder-cross-app-context.md)の基本契約:** 既存の通常フォルダをProjectに指定可能であり、元の場所にあるResourceも複製を強制せず参照登録できる。純正アプリ横断のTaskとProjectの許可された文脈保持、明示的な利用者指定優先を維持する。**Project＝何について作業するか／Workspace＝どう作業するか**を分離し、1 Projectを複数Workspaceから利用できる。
- **[DEC-012](DEC-012-workspace-naming-organization.md):** Workspaceの自動命名・固定／最近／過去の分類、利用者の命名・順序優先、自動削除禁止、Permissionで絞られた自然言語検索・理由付き候補と本人による最終選択を継承する。統合ビューは検索結果の根拠や復元可能性を推測で補わない。
- **[DEC-014](DEC-014-workspace-state-checkpoints.md):** 自動保存・名前付きチェックポイント、保存履歴と復元プレビュー、**既定では新しいWorkspaceへ復元**、元Workspaceへの復元を明示選択したときの復元前保護・競合処理・必要な確認を維持する。Workspace状態保存はファイル本文バックアップや長期Memoryとは別であり、Projectを開く／Workspacesを列挙する／検索するだけで復元・Task実行を開始しない。
- **[DEC-065](DEC-065-nagi-handoff-work-continuation.md):** 端末間の意味的な作業継続は、実ファイルの移送、無条件のCloud同期、元端末のPermissionの引継ぎを許可しない。必要なSourceの実体取得・転送や競合解消はその都度の正規の認可・選択を維持し、HandoffとNagi Shareの単発ファイル転送を混同しない。
- **Permission／Privacy／Memory:** Projectへの関連付けや一元表示から本文無断取得、全ファイル走査、情報区分／Owner／Profileの越境、長期Memoryへの自動昇格、外部送信・共有・Action許可の拡大を行わない。Source／Revision失効、競合、部分復元・復元不能・不明は正直に示す。明示停止・中断・復旧待ちのAI Taskや副作用Actionを無断再開・再実行しない。
- **Gate:** これは体験の製品方向を記録するものであり、Workspace機能の実装完成、Project／Handoffの動作成功、0.2／0.3／0.4+／1.0のRelease・Activation・Acceptance Gate変更、実装開始の許可ではない。

### PX-03の未確定事項（PROVISIONAL／OPEN）

- 統合作業ビューの名称・画面構成・関連情報の配置、通常フォルダのProject指定／解除UI、Task・資料・外部Resource・Workspaceの一覧構成・絞り込み・並び順・表示件数。
- Resource・Task・Workspaceの関連付け／参照の追加・解除／手動訂正の具体操作、1対多以外の関連付けモデルの採否、Projectの重複・入れ子・Archive／削除等の振る舞い。
- Project中心の検索と関連候補のランキング／根拠、履歴プレビュー、Workspace・チェックポイントへの移動／復元／再開導線、未対応Resourceや失効時の細かな表示、別端末Handoffとの画面・状態の接続方法。
- Metadata／schema・Index・データ統合方式、分析・学習・性能目標、対象アプリ／端末・技術・テスト／Acceptance・提供Version。既存のResource認可・Workspace履歴・Memory保持／忘却方針は未決事項に戻さない。

## 権限・安全性と既存決定との整合（CONFIRMED）

- フォルダのProject指定、ファイルの格納、Resourceの参照登録、同一Project所属、Memoryの類似性から**新しい閲覧・編集・外部送信・共有・権限委譲の許可は生じない**。Context取得・参照先の解決・Action実行の都度、現在のUser／Profile／Permission／Capability／Policy、承認済みTask Scope、情報分類、対象Resource／Revision、Provenanceを検証する。権限がなければその部分は扱わず、制限を説明する（DEC-020／023／025）。
- Projectを理由に、フォルダ内の全文や個人Memoryの無差別スキャン、第三者への自動共有、未承認の関連資料変更を許可しない。参照切れ・Revision競合・結果不明・部分成功を成功と表示しない。外部送信と破壊的な操作は既存の独立した承認規則に従う。
- Project所属・参照登録だけで本文へ無断アクセスしない。Cloud AIを含む外部送信は[DEC-002](DEC-002-cloud-ai-data-sharing.md)の既存の事前許可・送信時検査を維持し、Project指定を包括的な送信同意と扱わない。
- Project別Memoryは、User／Profile、Task、Agent、Team、Organizationの所有・共有Scopeを暗黙に混合しない。派生要約・索引・Context cacheにも元資料の機密分類・由来とDEC-003／004／005の削除・忘却を反映する。明示的に忘却した情報を古い履歴から無断再学習しない。DEC-014のWorkspace履歴・検索用メタデータ・節目要約は長期Memoryとは別管理であり、Projectへの関連付けだけで長期Memoryへ自動昇格しない。
- 作業継続は、保持を許された文脈を用いて後日の作業を助ける製品仕様である。Projectを開くことやWorkspaceの復元だけで、明示停止・中断・復旧待ちのTaskや副作用Actionを自動再開・再実行する許可にはならない。
- この利用者の仕事・資料のためのProjectは、DEC-040で定める`builder`製アプリの**開発Project保護・保存契約とは別**。実装上の名称やUIが類似しても、開発ソースの保持／削除／配布権限を移さない。
- 本決定はDEC-023の作業・データ中心思想とDEC-025の分散正本・関連更新提案を具体化するもの。既存release／activation／acceptance gateや実装済み機能の評価を変更しない。

## 未決定事項（PROVISIONAL）

- 「Nagi Project」という機能名称、Filesでのマーク／作成方法、通常フォルダとの見た目、Project専用ビュー、Projectの複数所属、入れ子、Project移動・改名・削除・Archive時の扱い。
- フォルダと参照のMetadata／schema／ID／manifest、元データの移動・権限変更・削除時の参照維持、Index、Cross-App Graph、ファイル形式、既存ファイルへの互換方式。
- Projectに保存・参照する文脈・Memoryの具体的な構造、ローカル保存先・保存方式、保持期間の具体値、既存の忘却Policyを適用する実装、検索・学習の条件とUI。DEC-003／004の確定済みの所有・機密分類・忘却の原則は未確定事項へ戻さない。Projectに追加しただけで内容を自動学習・取り込みする方式は決めない。
- 対応アプリの最初の組み合わせ、写真アプリの実装範囲、他社アプリ参加の条件、第三者SDK、最初のEnd-to-End実証フロー、パフォーマンス指標。
- Project管理操作への既存権限の適用詳細、複数利用者・端末移行／同期・競合・バックアップ・復旧、通知UI、提供バージョン、実装方式、具体的な受入条件。これらの設計でProject所属による権限増加を認めない。

## 実装時期とロードマップ

0.2の共通Resource／Document／Object／Public Action等を土台とし、0.3の単一Personal Agent／Memory／アプリ横断Task、将来の純正アプリ連携に接続する構想。対象機能を0.2・0.3・0.4・1.0の新しい必須Gateとして割り当てない。現時点で動作・試験完了したことも意味しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | 既存の通常フォルダをProjectとして指定し、純正アプリの資料・参照をProject単位にまとめて横断AI作業、変更追従提案、文脈／Memory、作業継続を行う方向を確認。ProjectはWorkspaceと別で、1 Projectを複数Workspaceから扱える。参照・権限の境界を維持。名称・実装方式・版割当ては未決定。 |
| 2026-11-16 | 採用済み仕様の再確認に基づき、他所のResource参照登録と専用形式非強制を明確化。本文取得・外部送信・Task再開の独立した権限境界、Memoryの所有・分類・忘却とWorkspace履歴の別管理を補足。ローカル保存方式等の詳細は引き続きPROVISIONAL。 |
| 2026-11-17 | PX-03においてProject中心の統合作業ビューBを基本とし、将来は許可された資料・Task・Workspace／作業履歴の関係から根拠付きの再発見と継続を支援するCへ発展する高水準方針をCONFIRMED。Project／Workspaceの分離、既存Resource参照・Workspace保存／本人選択による復元・Handoffと現行Permissionを維持。画面・関連付け・検索／復元導線・技術・性能・提供版はPROVISIONAL。 |
