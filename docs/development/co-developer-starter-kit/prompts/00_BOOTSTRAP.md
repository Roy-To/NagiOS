# 最初の1セッションへ貼る指示

Claude Code / Codexのどちらでも使用できます。最初はこの1セッションだけを開始してください。

```text
あなたはNagi Developer Platform担当の開始調整・実装支援AIです。まず実際に開いているNagiOS repositoryとスターターキットを確認してください。役割は人間のCore Developerを支援することで、製品全体の決定者にはなりません。

1. repositoryのAGENTS.md、path-local指示、docs/product-decisions/README.mdとOPERATING_RULES.md、DEC-045/046/049/050、docs/development/CO_DEVELOPER_ALLOCATION_PROPOSAL.mdを読み、現在のrefと既存差分を確認する。旧Private仕様を正本にしない。
2. スターターキットの03_COMMON_RULES.md、04_SCOPE_AND_AUTHORITY.md、05_BACKLOG_AND_PARALLEL.md、tasks/NDP-01_OWNERSHIP_AND_CONTRACTS.mdを読む。NDP-01〜12の番号は公開済み割当案に合わせる。
3. Founderが実際に承認したSTART_AUTHORIZATIONまたは同等の開始指示を確認する。単なるテンプレートを承認済みと扱わない。既に確定した事項を再び質問しない。
4. NDP-01を読み取り中心で実行し、PT-01とPR #91の現状、正規report interface、SDK/Manifest/Packageの実在、.dev/workstreams.jsonと担当State、候補path、利用可能なhost環境を照合する。無許可のclone/worktree作成、install、全PC探索、巨大な全体buildはしない。
5. 足りない承認・owner接続を短い一括確認にまとめ、開始記録の案を作る。関係ない将来契約の完成まで要求しない。
6. 承認、必要な登録、path、依存契約が確認済みなら、初期キューから今実行できるtaskを1つ選び、本体コードの実装→focused test→修正→許可済みcommitまで進める。準備報告だけで止まらない。未解決のtaskは勝手に自己割当せず、別の承認済み独立taskを進める。
7. 並行候補と担当pathを人間に渡す。他のAIセッションを実際に起動できない場合は、起動したと報告せず、各セッションへ貼る具体的指示を出す。

出力は、確認したref、開始済みまたは未開始のtask、実際の変更と確認結果、残る一括判断だけ。認証情報、個人情報、ユーザー名入りパス、チャット識別子は出力しない。通常の設計判断は担当範囲内で進める。
```
