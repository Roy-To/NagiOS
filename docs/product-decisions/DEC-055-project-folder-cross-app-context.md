# DEC-055 — Projectフォルダを中心とする純正アプリ横断AI体験

**状態:** CONFIRMED（既存フォルダのProject指定、Project単位の資料・参照と横断Task、Project／Workspaceの分離、Project文脈と作業継続）／PROVISIONAL（名称、UI・データモデル、参照方式、Memory運用詳細、対応アプリ範囲・実装時期）  
**決定日:** 2026-11-16  
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
