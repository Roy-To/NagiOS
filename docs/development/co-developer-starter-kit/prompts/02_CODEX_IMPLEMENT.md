# Codex実装用

`TASK`を担当IDへ変え、開始記録とともに貼ります。例はNDP-05です。

```text
TASK: NDP-05

NagiOSの指定作業フォルダでTASKを実装してください。最初にAGENTS.mdとpath-local指示、docs/product-decisions/README.mdとOPERATING_RULES.md、関連DEC、スターターキットの03_COMMON_RULES.md・04_SCOPE_AND_AUTHORITY.md・TASKの個別MD・実際に承認されたSTART_AUTHORIZATIONを読んでください。

現在のbranch、HEAD、status、diff、taskのowner/pathと依存契約を実際に確認してください。保存された割当案を無条件の開始許可とせず、既にある開始承認を重複要求もしないでください。条件を満たす範囲では調査・計画だけで終了せず、実装・focused test・修正まで継続してください。

実装は小さい動く単位で行い、既存PT-01/SDK/validator/CLIを再利用してください。実行したテストのコマンドと結果を短く残し、担当差分だけを許可されたbranchへcommit/pushし、指定baseへのPRを用意してください。公開仕様を変える必要がない内部設計は自分で選んで進めてください。

他AIが編集中のフォルダ・共通ファイル・生成物へ書き込まないでください。並列のために別branchだけを同一フォルダで切り替えないでください。共有契約変更は調整担当へ返し、承認済みの独立作業は止めないでください。

同じ問題への意味ある修正3回で解けない場合はblockerと次の候補を記録します。権限拒否を迂回せず、架空SDK、偽の成功、弱めたassertionで通しません。host-onlyとNagi target受入を区別してください。

終了報告はTASK、commit/PR、実際の確認結果、残件、次の作業だけ。実際に走らせていない別セッションやテストを開始済み・成功としないでください。
```
