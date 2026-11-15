# Claude Code実装用

`TASK`だけを担当IDへ変え、開始記録とともに貼ります。例はNDP-04です。branch/pathは承認済み記録から読み、推測しません。

```text
TASK: NDP-04

このNagiOS作業フォルダで、スターターキットの03_COMMON_RULES.md、04_SCOPE_AND_AUTHORITY.md、TASKに対応するtasks/のMDと、実在するSTART_AUTHORIZATIONを読んでください。repositoryのAGENTS.md、既存CLAUDE.mdとpath-local指示も読んでください。

あなたは指定TASKの実装担当です。最新の確定仕様、branch/base、許可path、依存契約、host-only開始条件を確認し、条件が満たされていれば計画だけで止めず実装してください。既存のPT-01、SDK、Manifest、Package、CLIの正本を再実装しないでください。

実装→format/static check→focused test→失敗修正→再実行→差分確認→許可されたbranchへcommit/push→小さいPR、の順で進めてください。実装言語、内部構造、テストの分け方は担当内で判断して構いません。テストコマンドは実在する設定から確認し、架空のコマンドを成功扱いしないでください。

他セッションと同じファイルや共通lock/BOARDを同時編集しないでください。共通変更が必要なら調整担当へ最小差分を渡してください。1つの実装が完了したら、同じ担当内の残件または明示割当された次taskへ進めてください。担当外taskを独断で引き取らないでください。

同じ問題に意味のある修正を3回行っても解消しなければ、そのblockerだけ短く記録し、他の許可済み作業を続けてください。権限拒否や未知の結果を迂回しないでください。

報告はTASK、commit/PR、実行コマンドと結果、未検証、次の作業を短く。host成功をNagi target成功にしないでください。
```
