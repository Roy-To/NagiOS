# NDP-09 — 薄い検証オーケストレーター

状態: 担当確定（Core Developer、2026-11-15）/ NOT STARTED。PT-05。NDP-08、独立path、実行Scope、host条件の明示承認が必要。

## 目的

既存toolの呼出しと結果をまとめ、何を実行して何が失敗/未実施なのかを開発者へ返す。検証の合否をLLMに決めさせない。

## 編集範囲案

`tools/nagi-developer-platform/validation/**`と専用テスト。既存CLI、Package validator、CI、本体SDKは直接変更しない。

## 実装

NDP-08の確認済みentryをadapterで呼ぶ。入力ファイルが指定する任意shell文字列を実行しない。承認済み実行ファイルと引数列、cwd、出力先を明示し、許可範囲をチェックする。実行結果、終了コード、timeout、cancel、未実行を保持する。出力量を有限にし、ログから秘密や個人絶対パスを漏らさない。

Stage 1は解析対象のbuild scriptを走らせない。Stage 2は明示許可された入力だけをbuild/testする。Stage 3は初期pilotでは未接続として扱え、何も試さずPASSを返さない。後続stageを進める条件と、失敗時の停止範囲を明示する。

## focused test案

成功/非zero終了、実行ファイルなし、timeout、cancel、壊れた出力、過大ログ、引数への特殊文字、許可外cwd/path、未承認Stage 2/3、前段失敗を検査する。テスト用child processでrunnerの振舞いを調べても、正規tool接続まで実証したとしない。

利用可能な正規Manifest validator等を使う小さな実接続テストも行う。実行条件がない場合はその部分を未検証として分離する。

## 最初のcommit

Stage 1の1つの実toolと、成功/失敗/未実行の結果を先に成立させる。その後Stage 2の承認済み範囲を増やす。独自のPackage生成や秘密鍵管理へ拡張しない。

## 次

NDP-03の生成物と接続し、NDP-11で同じ検証経路を使う。host成功をNagi install/GUI成功にしない。

共通: [規則](../03_COMMON_RULES.md)、[レビュー](../06_REVIEW_INTEGRATION_RESUME.md)。
