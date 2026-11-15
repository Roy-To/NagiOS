# 最初に読む — Nagi共同開発の始め方

作成日: 2026-11-15 / 対象: Nagi Developer Platform、PT-02〜06 / 状態: 配布・開始準備用。

## この一式で行うこと

創設者はNagi全体の方向性と最終判断を担当し、共同開発者は割当領域内の設計・実装・テストを自主的に進めます。担当案は、アプリ雛形、API差分、UI構造解析、検証ツール連携、移植評価です。PT-01、Linux起動、本体UI、共有SDKそのものを引き取る指示ではありません。

## あなたが創設者なら

1. このフォルダ一式またはZIPを友人へ渡す。
2. [01_FOUNDER_HANDOFF.md](01_FOUNDER_HANDOFF.md)の「送付文」を送る。
3. 同ファイルの「一括開始承認文」は、NDP-01で実際のpath・環境・既存担当との接続を確認した後、内容に同意して送る。すでに別途承認・登録が済んでいる場合は、その記録を渡せば重複承認は不要。

公開済みの割当案は現時点でPROVISIONALです。このキットを渡しただけで、その状態を勝手に確定へ書き換えません。最初の承認はまとめて行い、以後の日々の設計・実装を逐次承認待ちにしない構成です。

## あなたが受け取った共同開発者なら

1. [02_SETUP_AND_LAUNCH.md](02_SETUP_AND_LAUNCH.md)に従い、コードを読み書きできるClaude CodeまたはCodexでNagiOSの作業フォルダを開く。
2. 最初はAIを1セッションだけ起動し、[prompts/00_BOOTSTRAP.md](prompts/00_BOOTSTRAP.md)の指示文を貼る。
3. AIに最新仕様・既存担当・開始条件を照合させる。未確定事項は一度にまとめる。権限や契約が既に決まっていれば確認質問を繰り返さない。
4. 開始承認と必要な登録が済んだ範囲から実装へ進む。[05_BACKLOG_AND_PARALLEL.md](05_BACKLOG_AND_PARALLEL.md)のキューを使用する。
5. 日々は担当MDとClaude/Codex開始文だけを渡す。全ファイルを毎回チャットへ貼り付ける必要はない。

## 最初に貼る短い指示

```text
このNagiOS作業フォルダで、docs/development/co-developer-starter-kit/00_START_HERE.md と prompts/00_BOOTSTRAP.md を読んで開始してください。後者のpromptsはスターターキット内の相対パスです。最新の公開仕様と開始承認を実際に照合し、未確定事項は一度にまとめてください。条件が満たされた担当は計画だけで止めず、実装・テスト・修正まで進めてください。満たされていない担当は読み取り調査までにし、承認済みの別担当は止めないでください。
```

フォルダが別の場所にある場合は、その実在する場所をAIに指定するか、`prompts/00_BOOTSTRAP.md`の本文を直接貼ってください。存在しないパスを読んだことにさせないでください。

## ファイルの使い分け

| 読む人・場面 | ファイル |
|---|---|
| 創設者から送る | 01_FOUNDER_HANDOFF.md |
| 環境・アプリの操作 | 02_SETUP_AND_LAUNCH.md |
| 全AIに共通 | 03_COMMON_RULES.md、04_SCOPE_AND_AUTHORITY.md |
| 作業順・並行運転 | 05_BACKLOG_AND_PARALLEL.md |
| レビュー・統合・再開 | 06_REVIEW_INTEGRATION_RESUME.md |
| 正本・根拠・限界 | 07_SOURCES_AND_LIMITATIONS.md |
| 初回／実装／レビュー／再開 | prompts/以下の6ファイル |
| 個別の依頼 | tasks/NDP-01〜NDP-12 |
| 最小限の記録 | templates/BOARD.md、START_AUTHORIZATION.md |

## 何を完成と呼ぶか

実装指示書があることと実装が動くことは別です。hostテスト成功はNagiイメージ内の起動・権限・保存の成功ではありません。成果はcommit、実行コマンドと結果、未検証事項を短く報告します。
