# NDP-09 — 薄い検証オーケストレーター

状態: 担当確定（Core Developer、2026-11-15）/ PARTIAL（host限定、PROVISIONAL。根拠は末尾の「実績・根拠」節。Stage 3とNagi target受入は未実施でDONEではない）。PT-05。NDP-08、独立path、実行Scope、host条件の明示承認が必要。

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

## 実績・根拠

統合branch上の実装に合わせた記録（2026-11-28）。すべてhost限定であり、Nagi target受入ではない。

- Stage契約（NDP-09s2）: [stages.json](../../../../tools/nagi-developer-platform/validation/stages.json)と[validation/README.md](../../../../tools/nagi-developer-platform/validation/README.md)。Stage 1はManifestの静的検証のみ、Stage 3は未接続（`connected`ではない）。
- 自作fixture限定のStage 2（NDP-09s2）: [stage2.py](../../../../tools/nagi-developer-platform/validation/stage2.py)。固定引数のclangで承認済みfixtureだけを扱う。テストは[test_stage2.py](../../../../tools/nagi-developer-platform/validation/test_stage2.py)と[test_stage2_diagnostics.py](../../../../tools/nagi-developer-platform/validation/test_stage2_diagnostics.py)。
- Stage 1と生成雛形の接続（NDP-09c）: [generated_check.py](../../../../tools/nagi-developer-platform/validation/generated_check.py)と[test_generated_check.py](../../../../tools/nagi-developer-platform/validation/test_generated_check.py)。Manifest検証の入口は[validate.py](../../../../tools/nagi-developer-platform/validation/validate.py)と[test_validate.py](../../../../tools/nagi-developer-platform/validation/test_validate.py)。
- 一通しの移植レポート（NDP-09r）: [report.py](../../../../tools/nagi-developer-platform/validation/report.py)（`--fixture all`を含む）と[test_report.py](../../../../tools/nagi-developer-platform/validation/test_report.py)。引き継ぎ記録は[handover.md](../../developer-platform/handover.md)、進捗は[board.md](../../developer-platform/board.md)。

未実施（このため状態はDONEにしない）:

- Stage 3（Nagi target）の受入は未実施。何も試さずPASSを返す実装はない。
- 実`nagi-pkg`環境での確認と、実PT-01 scanner接続は未実施。
- host成功はNagi install/GUI成功ではない。

この状態（PARTIAL扱い）はCore Developerからrunnerへの委任に基づくPROVISIONALな判断であり、承認済みではない。根拠が揃うまで実際より低い状態に保つ。記録は[評価README](../../../../tests/developer-platform/evaluation/README.md)のPROVISIONAL節にもある。整合は[test_task_doc_status.py](../../../../tests/developer-platform/evaluation/test_task_doc_status.py)が検査する。

共通: [規則](../03_COMMON_RULES.md)、[レビュー](../06_REVIEW_INTEGRATION_RESUME.md)。
