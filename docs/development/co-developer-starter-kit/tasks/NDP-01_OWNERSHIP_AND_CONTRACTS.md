# NDP-01 — 担当・契約・開始条件の照合

状態: PROPOSED / NOT STARTED。共通の読取調整タスク。詳細手順は本キットの提案で、正本は公開割当案のNDP-01です。

## 目的

既存担当を邪魔せず、PT-02〜06の実装が具体的に開始できる条件を一度に整理する。全体設計を作り直す仕事ではない。

## 入力

`AGENTS.md`、product-decisionsのREADME/OPERATING_RULES、DEC-045/046/047/049/050、`docs/development/CO_DEVELOPER_ALLOCATION_PROPOSAL.md`、`.dev/workstreams.json`と関係State、PT-01担当記録、PR #91の現在のhead、SDK/Manifest/Packageの実コード、受取側の許可済み環境。

## 実施

1. 現在のrepository、ref、branch/status/diff、明示された開始承認を確認する。既存未commit差分を保持する。
2. PT-01の `tools/nagi-porting/**` 案とPR #91のCLI scannerを照合する。どちらが採用済みか、どのreport interfaceをPT-03が使うかを確認し、独自に採用決定しない。
3. `sdk/rust/Cargo.toml`、SDK README/contract、Manifest schema/validator、`tools/nagi-pkg`、`samples/hello-nagi`を読む。外部から利用可能なSDK参照と未提供APIを区別する。
4. 新規path候補、owner、branch/base、host-only実行条件、必要な登録と統合受取先を短く整理する。公開にないローカル作業は確認不能と記録する。
5. `templates/START_AUTHORIZATION.md`相当の短い開始案を用意する。既存の承認は参照し、未確定項目だけをFounder/ownerへ一括で返す。

## 最初の成果

担当・正規接続・不足をまとめた小さな表と、開始可能なtask群。ファイル保存が未許可なら回答内の案でよい。準備用の大量JSONや別repo、runnerは作らない。

## 完了条件案

- PT-01とSDKを二重実装しないための責務が説明できる。
- 確定済みと要判断を分け、開始条件を満たすtaskだけを挙げられる。
- 現物がない環境・承認・契約を確認済みにしていない。
- 参照refと次の具体的アクションがある。読み取り調査をコード実装PASSとしない。

## 次

承認・登録が済んだNDP-04/05/06/10などを起動する。PT-01接続が未決なら関連実装だけを保留し、承認済みの別taskを進める。

共通: [規則](../03_COMMON_RULES.md)、[権限](../04_SCOPE_AND_AUTHORITY.md)、[キュー](../05_BACKLOG_AND_PARALLEL.md)。
