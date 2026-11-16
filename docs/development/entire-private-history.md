# EntireのPrivate履歴運用

**更新日:** 2026-11-16  
**方針:** [DEC-050](../product-decisions/DEC-050-public-specification-repository.md)

## 保存先と適用範囲

- 公開 `Roy-To/NagiOS`: コード・決定済み仕様と必要なCI。
- Private `Roy-To/nagi-checkpoints`: Entireの会話履歴・関連メタデータ専用。GitHub Actionsを無効にし、履歴保存用workflowを置かない。
- 旧 `Roy-To/NagiOS-internal` への新規記録を再開しない。
- コード・成果物のmirror、`--mirror`／`--all`／強制push、公開Git履歴書換え、既存履歴削除は行わない。認証情報はPrivateへも送らない。

この文書は開発時の履歴運用を扱う。Nagi製品内のMemory／Waybackや既存Release Gateは変更しない。

## 確認した版と公式設定

確認したEntireは **0.12.2**、実際の試験agentは **Codex CLI 0.164.0**。他agentの実記録は未検証。Codexデスクトップの当該会話が自動記録されたという実証でもない。

根拠はインストール済み版の `entire configure --help`／`entire enable --help` と、公式の[同版README](https://github.com/entireio/cli/blob/v0.12.2/README.md)、[同版のセキュリティとプライバシー](https://github.com/entireio/cli/blob/v0.12.2/docs/security-and-privacy.md)。別版へ更新するときはhelpとhook動作を再確認する。

既存のEntire有効環境では、次の公式オプションでローカル設定を保存する。

```sh
entire configure --local --checkpoint-remote github:Roy-To/nagi-checkpoints
entire configure --local --skip-push-sessions
```

保存先は `.entire/settings.local.json` の `strategy_options.checkpoint_remote`。現在は `strategy_options.push_sessions=false` とし、**自動記録は維持するが、既存会話を無人で一括送信しない**。これは保存先設定だけで自動送信の完了を表明するものではない。送信再開には未送信内容の秘密情報確認と、対象ref・親履歴の確認が必要である。

既存hooksを上書きせず、現在のpre-push hookを保存して呼び出す構成とした。追加したローカルの送信境界は次のとおり。

- 公開先へのEntire履歴ref送信を拒否する。
- 旧保存先へのpushを拒否する。
- Private履歴先へのコードref、コードの親履歴を含む送信を拒否する。
- 公開先の通常コードrefの経路を維持する。

これらのhookはローカル環境に適用したもので、公開文書を取得しただけでは他のcloneへ自動導入されない。hookを迂回するpushや、未確認cloneを含めた全環境の漏洩防止は実証範囲外である。

## 設定変更の確認結果

| 対象 | 結果 |
|---|---|
| Privateリポジトリ | 試験前の最新API確認でPrivate。ref取得は空リポジトリ応答だった。過去の確認結果は流用していない。 |
| Private Actions | 変更前は有効。対象リポジトリのActions permissionsを `enabled=false` に変更し、GETでfalseを読み戻した。変更前のrun数は0。run数だけを無効判定に用いていない。 |
| 本体の既存ローカル設定 | Private保存先を確認。既存履歴の一括送信を止めるため自動送信を無効化。 |
| 承認済み範囲の既存worktree | 旧保存先を持つ193件の既存設定だけをPrivate先・自動送信無効へ変更し、ローカルバックアップと読み戻しを実施。 |
| 初期確認で設定がなかったworktree | 23件を確認。設定欠落時にも既定有効となる挙動に備え、承認済み範囲だけで保存先の明示と自動送信停止を追加中。適用完了・動作確認済みとはまだ扱わない。 |
| 今回の承認範囲外 | 32件は変更していない。全cloneへの適用完了とは扱わない。 |
| 送信境界 | 8件の境界checkに加え、実際のhookで公開履歴ref・Privateコードrefの2件を拒否。通常の公開コードpushのdry-runは許可された。実コードpushのCI成功とは別の検証。 |

公開Actions・branch protection・必須チェックを変更する操作は行っていない。公開側の必要チェックは既存ルールに従う。

## 実際の記録・送信・取得試験

Nagi本体コードを含まない合成データだけの試験用リポジトリで、実際のCodex CLIとEntire生成hooksを使用した。会話ファイルを手で置いて記録成功と判定していない。

| 段階 | 確認結果 |
|---|---|
| 初回起動 | hookの信頼設定により記録されず、失敗として扱った。 |
| 修正後の実agent記録 | Entire生成hookの内容を確認後、試験起動だけの信頼設定で再実行。実際のsession記録とgit commit時のcheckpoint生成を確認した。 |
| Privateへの送信 | 試験環境だけ自動送信を有効にし、公開コードpushのdry-run時に既存Entire pre-pushがPrivateへcheckpointを1件送信した。コードpush自体はdry-runのためbranchを送っていない。 |
| 保存先から取得・内容照合 | 別の新規bareリポジトリに対象checkpoint refだけを取得。送信元とSHAが一致し、取得したtranscriptに試験会話の検証用文字列が含まれることを確認。 |
| 試験履歴の公開非送信 | 公開先がadvertiseするrefを確認し、Entire履歴ref・試験コードbranch・対象checkpoint SHAがないことを確認。過去の到達不能objectまでの網羅監査ではない。 |
| 試験後のPrivate Actions・新規run | APIでPrivateを維持、Actions `enabled=false`、run数0のままであることを再確認。 |

送信前に試験refの6ファイルを確認し、親なし・1 commitだけのメタデータrootで、本体コードの親履歴を含まないことを確認した。6種類の認証情報パターンに一致せず、無害な合成会話だけであることを確認してから送信した。試験後のPrivate refはこの1件だけである。パターン検査だけで任意の将来会話の安全を保証するものではない。

通常起動のCodex CLI、デスクトップ、他agent、無人の自動送信については、上記の限定試験から成功を推定しない。試験環境の自動送信成功と、既存履歴を持つ開発環境の自動送信停止を区別する。

## 既存履歴の保全と残件

既存ローカルcheckpoint refは2件あり、親を持たないメタデータ専用のrootであることを確認した。ただし既存transcriptに認証情報パターンの候補を検出したため、**既存履歴は送信していない**。候補の値や会話本文は本書へ掲載せず、原本・未送信履歴は削除せず保全した。パターン検出は確定的な秘密情報判定でも、他の秘密情報が存在しないという保証でもない。

残件は、候補の確認と必要な除去方法の検討、移行対象refの内容・到達可能な親履歴の再確認、通常起動するagentでの記録・送信・取得の再実証、自動送信を再開できる条件の確定である。既存履歴を保全していることと、Privateへの移行が完了したことを区別する。
