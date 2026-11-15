# NDP-03 — 最小アプリ雛形generator pilot

状態: 担当確定（Core Developer、2026-11-15）/ NOT STARTED。PT-02。NDP-02の利用可能な契約と、host-only実装・検証の開始承認が必要。

## 目的

開発者が選んだ出力先へ、現行SDKに合う小さなアプリ雛形を安全に生成する。全面自動移植や新SDKの作成ではない。

## 編集範囲案

`tools/nagi-developer-platform/generator/**`と対応テスト。実際のpathは開始記録を優先する。SDK、Package validator、共有Cargo設定、PT-01は直接編集しない。

## 実装

NDP-02で決まった入力を検証し、生成予定を表示できる経路と、明示された新規出力先へ生成する経路を分ける。既存ファイル、上位へ逃げるpath、symlink経由の対象外書込みを拒否する。生成に失敗したら部分成果を成功とせず、自分が新規作成した範囲を識別して報告する。ユーザーの既存ファイルを自動削除しない。

SDK参照は確認済み方式だけを使う。未提供API、実在しない権限、秘密、個人絶対パスをテンプレートへ埋め込まない。生成しただけで依存取得・build・実行・installを始めない。これらはPT-05の明示されたStageへ渡す。

## focused test案

正常入力と決定的出力、無効アプリID、未対応SDK、出力先衝突、相対path逸脱、symlink、書込み失敗、再実行、入力不変を確認する。fixtureだけの別generatorではなく、実際の生成コードを呼ぶ。

生成Manifestを既存validatorで検査し、許可された場合だけ最小host build/testを行う。実行不能なら理由を明示し、成功にしない。

## 最初のcommitと完了境界

最初は1種類の有効な最小雛形と拒否系テストを完成させる。その後必要な入力を増やす。完了は選択した範囲のgeneratorとhost検証であり、Nagi target起動・署名・配布の完了ではない。

NDP-09へ生成物と検証入口を、NDP-11へ比較可能な小さな例を渡す。詳細な採用記録を増やさずcommitと結果を残す。

共通: [規則](../03_COMMON_RULES.md)、[レビュー](../06_REVIEW_INTEGRATION_RESUME.md)。
