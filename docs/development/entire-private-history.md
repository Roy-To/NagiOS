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
```

保存先は `.entire/settings.local.json` の `strategy_options.checkpoint_remote`。初回は `entire configure --local --skip-push-sessions` で一括送信を止めた。同日後続の内容審査・移行後、既存の他設定を保持して次の公式JSON設定を保存し、読み戻して**本番の自動記録・自動送信を有効にした**。

```json
{
  "strategy_options": {
    "checkpoint_remote": {"provider": "github", "repo": "Roy-To/nagi-checkpoints"},
    "push_sessions": true
  },
  "checkpoints": {"primary": {"type": "git-refs"}}
}
```

確認した0.12.2では `--skip-push-sessions=false` による再開を確認できなかったため、公式READMEに定義されたJSONキーを更新した。履歴保存にCIは使わない。保存先設定だけで送信成功と扱わず、後述の実試験を行う。

既存hooksを上書きせず、現在のpre-push hookを保存して呼び出す構成とした。追加したローカルの送信境界は次のとおり。設定が欠落していてもEntireは既定で有効になるため、保存先が不明・旧保存先のままの場合も送信を拒否する。

- 公開先へのEntire履歴ref送信を拒否する。
- 旧保存先へのpushを拒否する。
- Private履歴先へのコードref、コードの親履歴を含む送信を拒否する。
- 公開先の通常コードrefの経路を維持する。

これらのhookはローカル環境に適用したもので、公開文書を取得しただけでは他のcloneへ自動導入されない。hookを迂回するpushや、未確認cloneを含めた全環境の漏洩防止は実証範囲外である。

## 初回設定変更の確認結果（PR #141時点）

この表は初回保留時点の記録である。後続の移行・再開結果は末尾の節を参照する。

| 対象 | 結果 |
|---|---|
| Privateリポジトリ | 試験前の最新API確認でPrivate。ref取得は空リポジトリ応答だった。過去の確認結果は流用していない。 |
| Private Actions | 変更前は有効。対象リポジトリのActions permissionsを `enabled=false` に変更し、GETでfalseを読み戻した。変更前のrun数は0。run数だけを無効判定に用いていない。 |
| 本体の既存ローカル設定 | Private保存先を確認。既存履歴の一括送信を止めるため自動送信を無効化。 |
| 承認済み範囲の既存worktree | 旧保存先を持つ193件の既存設定だけをPrivate先・自動送信無効へ変更し、ローカルバックアップと読み戻しを実施。 |
| 初期確認で設定がなかったworktree | 承認済み範囲の23件にPrivate保存先・自動送信停止のローカル設定を追加し読み戻した。追跡対象の既存ファイルを先に確認し、共有info/excludeへの追記でローカル設定をignore対象とした。追跡済みソースは変更していない。 |
| 承認済み範囲の合計 | 既存193件と追加23件、計216 linked worktreeの保存先・自動送信停止を読み戻した。本体rootと今後の作成に使われるroot設定も同じ。既存post-checkoutの保存先保護と他hooksは保持した。 |
| 今回の承認範囲外 | 32件のローカル設定は変更していない。共有Git管理を使う場合は共通pre-pushの境界が適用されるが、各環境での正常記録・送信を確認したとは扱わない。 |
| 送信境界 | 設定欠落時もEntireが既定有効になる挙動を再現し、境界を閉じる修正を含む10件のcheckに加え、実際のhookで公開履歴ref・Privateコードrefの2件を拒否。通常の公開コードpushのdry-runは許可された。実コードpushのCI成功とは別の検証。 |

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

### 実開発リポジトリの通常CLI起動

実際の開発リポジトリの正規パスから、hook信頼設定の一時上書きを使わず、Codex CLI 0.164.0で無害な読取り専用会話を実行した。Entireの対応するsession状態にCodex・終了状態・1 turn・checkpoint count 1が記録され、Entire自身が作成した2つのmetadataファイルに試験用文字列が含まれることを確認した。

この確認ではファイル変更・git commit・公開送信を行っていない。**実開発リポジトリでの通常CLI会話のローカル自動記録**は実証したが、その会話の永続checkpoint ref生成やPrivate送信までの実証ではない。この初回試験時点ではrootの自動送信停止も維持した。

以上は初回試験・初回保留時点の記録であり、その時点ではDesktop・他agent・本番自動送信の成功を推定していない。同日後続の本番自動送信再開と実開発試験は、以下の節で区別して記録する。

## 既存2件の精査・保全・移行（同日後続）

初回は未解消の認証情報パターン候補があり、送信を保留した。後続作業でデコードした会話・入れ子の記録・補助transcriptの文脈を確認したところ、残っていた2種類の候補はサービス名の説明と、認証情報除去のunit testに使われたダミー値だった。実認証情報は確認されなかった。taskパス内の文字列や既に伏せられた値も誤検出に含まれていた。

原本の2 checkpointは、送信対象と独立したローカルbareリポジトリへ保存し、commit・tree・全blobの一致を確認した。保管先にはremoteを登録していない。不要な本文除去や履歴書換えは行わず、審査した元の2 refだけをPrivateへ明示送信した。別の新規bareリポジトリで再取得し、SHA・tree・親なしを照合した。本体コードの親履歴は送信していない。原本の削除やqueueを送信済みと偽装する操作も行っていない。

誤検出の例外は審査済みの不変commitだけをローカルで指定した。サービス名やダミー値を、新しい任意の履歴に対する恒久的な文字列例外にはしていない。会話本文・候補値・会話識別子は本書へ記載しない。

## 本番自動送信の再開と実開発環境の再検証

既存のrootと216 linked worktree、および再検証用に追加した実NagiOS worktreeの合計218設定で、Private保存先・`push_sessions=true`・`git-refs`を読み戻した。今後の作成時に使うroot設定も更新した。今回の範囲外32件は変更していない。

hook再導入によりEntireの送信が検査より先になる順序変更を検出し、送信検査を最外側へ戻した。従来のhookは別途保全し、元のEntire hookを1回だけ呼び出す。送信境界を次のとおり再検証した。

- 公開への履歴ref、checkpoint SHAをコードrefに見せかけた送信、旧保存先への送信を先に拒否する。
- Privateには許可したcheckpoint refとメタデータの親履歴だけを許可する。
- 自動送信では`git-refs`を必須とし、送信queueの内容・親履歴・新たな認証情報候補を確認する。
- Private維持とActions無効をAPIで確認してから自動送信へ進む。
- GitHub URLと同名のローカルパスを区別し、記号入りパスワード候補を見落とさない検査へ修正した。

検査が送信より先に実行されること、拒否時に後続hookを呼ばないこと、通常コードpushでは元hookを1回呼ぶことを確認した。URL・backend・認証情報候補の回帰確認11件も通過した。これらはqueue検査とEntire内部送信を単一transactionにするものではなく、並行commitに対する原子的な保証は検証していない。Entireの生成時除去と送信前検査も、任意の将来会話の安全を無条件に保証するものではない。

### 実NagiOSリポジトリでの試験結果

既存作業中の変更を守るため、同じNagiOS Git管理を使う専用worktreeで実際のCodex CLIを実行し、公開可能な運用文書だけを編集した。合成データだけの独立fixtureとは別の検証である。新しい作業場所のEntire生成hookが既存の確認済みhookと一致することを検証し、この起動だけhook信頼設定を指定した。新しい作業場所での恒久的なhook信頼を設定したという意味ではない。

| 段階 | 結果 |
|---|---|
| 実agentの会話記録 | Entire自身のsession状態と1 turnの記録を確認。手動で会話ファイルを置いていない。 |
| 永続checkpoint生成 | 実際の文書変更をcommitし、対応するcheckpointを確認。親のないメタデータのみのcommitで、送信前検査も通過。 |
| Privateへの自動送信 | 通常の公開文書branch pushがEntire pre-pushを起動し、Privateへ自動送信。コードは公開側、会話はPrivateへ分離された。 |
| 保存先からの取得 | 別bareリポジトリで対象refのみを取得し、送信元とのSHA一致・試験会話の検証用文字列・親なしを確認。 |
| 公開への非送信 | 公開advertised refsに履歴ref・checkpoint SHAなし。公開文書に会話本文・検証用文字列・識別子がないことも確認。 |
| Privateの状態 | Private、Actions `enabled=false`、run数0を再確認。この実開発試験の直後時点では、履歴は初回fixture 1件・移行2件・実開発試験1件の計4 ref。 |
| queue | この実開発試験の直後、Entireの実送信後に未送信queueが0件になったことを確認。 |

本番の保存先分離・自動送信再開と、このCodex CLI経路での記録→Private送信→取得を実証した。Desktop/app、他agent、全worktreeでのagent hook信頼・実記録の実証は引き続き別対象である。公開側のCI・main保護・必須チェックは維持する。
