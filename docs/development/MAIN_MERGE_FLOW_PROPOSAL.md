# mainへのマージフロー提案 — 創設者の統合フローへの合流とリスク別マージ

**状態:** 提案（PROVISIONAL）。創設者の承認前であり、権限・GitHub設定・運用はまだ何も変えていない。
**作成日:** 2026-11-18（同日、創設者の現行のmerge方法を確認し、ステップ1を追加）
**提案者:** 共同開発者（Developer Platform／NDP担当）
**宛先:** 創設者
**関連:** [DEC-046](../product-decisions/DEC-046-co-developer-governance.md)、[DEC-047](../product-decisions/DEC-047-linux-migration-workspaces-and-integration.md)、[DEC-049](../product-decisions/DEC-049-development-process-speed-priority.md)、[DEC-050](../product-decisions/DEC-050-public-specification-repository.md)、[DEC-051](../product-decisions/DEC-051-review-methods.md)、[PR #107〜#129統合記録](../product-decisions/PR-107-129-INTEGRATION-2026-11-16.md)。製品側の考え方として[DEC-001 PX-05](../product-decisions/DEC-001-agent-autonomy-actions.md)、[DEC-016](../product-decisions/DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-018](../product-decisions/DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-026](../product-decisions/DEC-026-ai-change-verification-and-recovery.md)、[DEC-103](../product-decisions/DEC-103-testing-release-evidence-baseline-ai-assisted-staged.md)

> DEC-046で「未決定」とされている「branch／PR／review運用」について、共同開発者からの提案をまとめたものです。採用されるまでCONFIRMEDではありません。

## 要点

1. **ステップ1（すぐ始められる）:** 創設者がいま使っている「PRを溜めて、統合パスでまとめてmergeする」流れに、共同開発者のPRも乗せる。GitHubの設定は変えない。創設者には、統合の指示に「`ready-for-merge`ラベルの付いたPRも対象に含める」と1行足してもらうだけでよい。
2. **ステップ2（実績ができたら）:** 担当領域の中だけを変え、CIと独立レビューを通り、1回のrevertで戻せる変更は、共同開発者が自分でmainへmergeする（段階A）。製品の方向、共有契約、安全境界、戻しにくい変更は、これまでどおり創設者が承認する（段階C）。その中間は24時間の通知付きmergeとする（段階B）。
3. ステップ2の境界は、人やAIの自己申告ではなくCODEOWNERSとGitHub Rulesetで機械的に強制する。既定は「創設者の承認」とし、明示的に委ねたpathだけを承認不要にする。
4. 判断待ちの質問はmergeから切り離す。仮定を置いて進め、決まった時点で差し替える。
5. どちらのステップでも、創設者自身の作業手順は変えない。

PR #166は5本に分けて出し直す（[第7節](#7-既存prへの当てはめ)）。ステップ1では創設者の次の統合パスで入る。ステップ2では、追加行の約98%（10,683行のうち10,446行）を共同開発者がそのままmergeでき、創設者が事前に承認するのは約130行と判断事項だけになる。

## 1. 現状（2026-11-18時点）

| 項目 | 確認した事実 |
|---|---|
| repository権限 | 創設者はadmin、共同開発者はwrite |
| mainの技術的な保護 | #166の`mergeable_state`は`clean`。mainへのmergeを機械的に止める設定は見当たらない。待ちを生んでいるのは運用ルール（starter kitの「mainへの直接push、無断mergeは含まない」、registryの`merge_boundary`の「explicit user directionなしにmainへmergeしない」）と考えられる |
| 創設者のPR | 1件ずつ出したものは作成から1時間以内にmergeされている（#131、#140〜#142、#144、#162、#164）。溜めてまとめたものも、10〜19時間後の統合パスでmergeされている（#107、#113、#132、#143） |
| 共同開発者からmainへのPR | **#105**（文書、19ファイル、+102行）は11/15 18時（JST）から待っている。創設者にreview依頼済みで、その後mainが進んで現在は競合している。**#166**（NDP統合、103ファイル、+10,561行、79 commit）は11/17 23時から待っている。共同開発者のPRがmainへmergeされた例はまだない |
| 共同開発者からNDP統合branchへのPR | #165、#167〜#178の13件は、AIの交互実装・独立レビュー・runner試験を経て、作成から1分以内にmergeされている |
| CI | 現行`ci.yml`の`python3 -m unittest discover -s tests`は、`tests/developer-platform/`（名前にハイフンがありpackageではない）と`tools/nagi-developer-platform/`を対象にしていないと見られる。NDPのPython試験はCIで走っていない可能性が高い（要確認） |

### 何が詰まっているか

1. **共同開発者のPRが創設者の統合パスに入っていない。** 統合パスの対象は創設者の指示で決まる（例：「PR #107〜#129」）。#105はその前日に出ていたが、範囲に入らなかった。共同開発者のPRをこの一覧に載せる仕組みがない（第3.1節）。
2. **自律性がmainまで届かない。** DEC-046は担当内の日常的な判断に都度の承認を求めない。しかしmergeは毎回創設者を通るため、担当内の作業もmainの手前で判断待ちになる。
3. **待つほどPRが大きくなる。** mainに入るまでの間に統合branchへ溜まり、#166は1万行を超えた。大きいほど扱いにくく、さらに待つという循環になる。待っている間にmainが進み、#105のように競合も起きる。
4. **質問とmergeが結び付いている。** #166は7件の確認事項、#105はIntegration Owner指定などの未決事項を抱えている。答えが出るまで、動いているコードも入らない。
5. **機械的なガードがない。** CODEOWNERSもRulesetもないため、委ねるとすれば「全部任せる」しかない。その結果、創設者が全件を通す運用になりやすい。

## 2. 設計の根拠 — Nagiの考え方との対応

Nagiは製品の中で、AIに対して「承認された範囲では自律的に進め、通常の変更は検証の証拠で完了させ、重要な変更や不確実な変更だけを人に回す」と約束しています。この提案は、同じ考え方を開発プロセス自身に当てはめるものです。なお、下表の製品DECは製品仕様であり、開発手続きを直接定めてはいません。ここでは設計の一貫性を示す根拠として引用しています。

| 根拠 | Nagiでの考え方 | マージフローへの適用 |
|---|---|---|
| DEC-046 | 創設者が方向と最終判断を担い、共同開発者は担当内を主体的に進める | merge権限も同じ境界で分ける。担当内は段階A、方向や領域横断は段階C |
| DEC-049 | 速度優先。完了の基準は「動くこと」。レビューでmergeを待たせない。継続制約は6つだけ | 段階A/Bではレビュー指摘を後続commitで直す。6制約のうち強制push禁止などはRulesetで強制する |
| DEC-051 | 重要な変更（権限・認可・Capability、認証・Session、署名・鍵・Package信頼、更新、利用者データの保存・整合性・移行、IPC境界、AIによる操作の実行と方針判断）にだけDeepSeekを追加する | 同じ一覧を段階Cの判定に使う。「DeepSeekが要る変更は創設者の承認も要る」とすれば、判定が1つで済む |
| DEC-016／DEC-018 | builderは承認範囲の中で修正と試験まで自律的に進める。通常の変更は検証の証拠で自動完了・条件付き反映する。重要な変更、検証できない変更、競合がある変更は人の確認に回す。自己申告だけでは合格にしない | 段階Aは「担当path内」「CI成功」「競合なし」を条件とする条件付きmergeにする。AIレビューの「問題なし」だけを条件にしない |
| DEC-026／PX-05（DEC-001） | 検証でき、戻せる変更を優先する。確認の重さはリスクに応じて変える。まずB、将来は状況に適応するC | 1 task＝1 PRにして1回のrevertで戻せる形を保ち、戻せる変更ほど速く通す。委ねる範囲は実績に応じて段階的に広げる |
| DEC-047、registryの先例 | 範囲を決めた事前承認で、都度のmerge承認をなくした例がある（Linux laneの統合branchへのmerge、`host-foundations-adoption`の「該当checkがすべてPASSした後のPR・mergeを承認」） | 同じ形の事前承認をmainにも適用する |
| AGENTS.md（Capability、AI security） | 最小権限。universal accessを与えない。判定はAIではなく決定的なPolicyで行う | CODEOWNERSは既定で創設者の所有にし、委ねたpathだけを開ける。段階はpathとCIで機械的に決まる |
| DEC-103／DEC-049 f | 証拠に正直であること。host上の結果とtarget受入を区別する | mergeはhost-onlyの統合であり、milestoneやtarget受入のPASSを意味しない |

一言でまとめると、**「戻せる変更は速く通し、方向を決める変更と戻しにくい変更は創設者が判断する」**です。

## 3. 提案

### 3.1 創設者の現行のmerge方法（GitHubの履歴から確認）

1. **決定ごとにPRを作る。** 会話で決めた内容を、エージェントが`docs/<話題>-<日付>`のbranchからPRにする（例：#107〜#129、#145〜#160）。GitHub上のreviewやコメントはない（#143、#162で確認）。CIは走り、文書だけの変更では重いjobがskipされる。
2. **溜めたPRを統合パスでまとめてmergeする。** 創設者の指示で始まり、次の3通りの進め方が見られる。
   - **順番に更新してmerge:** 各PRのbranchへmainを取り込み、衝突を「両方の決定を残す」形で解消してから、1件ずつmergeする（11/16 14時台に22件を約30分、18時台に8件を約12分。JST）。
   - **統合PRにまとめる:** 複数のPRの内容を1本に集約してmergeし、元のPRは置き換え済みとして閉じる（#161は15件のPXを集約し、作成から20秒後にmerge）。
   - **残ったPRを個別に解消してmerge:** 11/17 9時台の#132、#143。
3. **統合の記録を残す。** どのPRをどのDECへ反映したかの対応表を作る（[PR #107〜#129統合記録](../product-decisions/PR-107-129-INTEGRATION-2026-11-16.md)）。
4. **merge方式はmerge commit。** squashは使っていない。mergeは創設者のアカウントで行われ、一部はカスタムの件名が付いていることから、API／CLI経由と見られる。

### 3.2 ステップ1：創設者の統合フローに乗せる

**共同開発者が行うこと**

- PRはmainへ直接、1 task（または1つの話題）ずつ小さく出す。NDP統合branchには溜めない。
- 出す前にmainとの競合を解消し、CIを通しておく。統合パスでは「mergeするだけ」で済む状態にする。
- PR本文は創設者のPRと同じ形にそろえる（出典、変更範囲、検証、保持した境界、host-onlyの明記）。
- 未決の質問はPRに入れず、判断キュー（第5.2節）へ移す。
- 準備ができたら`ready-for-merge`ラベルを付ける。統合パスが始まった後はそのbranchへpushしない。

**創設者にお願いすること**

- 統合パスの指示に1行足してもらう：「`ready-for-merge`ラベルの付いた共同開発者のPRも対象に含める」。
- 共同開発者のPRは次のように扱ってもらう。
  - 文書の衝突は、創設者のPRと同じように解消してよい。
  - 担当path内のコードは書き換えない。衝突やCIの失敗があればそのPRは飛ばし、コメントで戻してもらう（共同開発者が直す）。
  - コードのPRは統合PRへ集約しない。コードの出所とPRの単位を残すため。

**評価**

| 良い点 | 残る課題 |
|---|---|
| GitHubの設定もルールの文書も変えずに、すぐ始められる | 創設者が統合パスを回すまでは待つ。創設者が作業しない日は止まる |
| 創設者が慣れた手順と判断の場をそのまま使える | 統合パスは文書の決定を前提に作られており、コードの差分を読む工程もNDP試験を走らせる工程もない。コードについては、創設者の承認は実質「入れてよいか」の判断にとどまり、安全性はあまり上がらない |
| 待ち時間が創設者のPRと同じ間隔（数時間〜翌日）になる | 1万行級のPRや、未決事項を含むPRは、統合パスでも扱いにくい |
| 衝突の解消と統合の記録が、既存の仕組みで回る | 判断を要するPRと、確認だけでよいPRが同じ列に並ぶ |

**結論:** まずステップ1から始める。そのうえで、担当path内のコードは統合パスに通しても安全性がほとんど増えず待ち時間だけが残るため、実績ができた時点でステップ2へ進み、そこだけを共同開発者のmergeに切り替える。統合パスは、段階B・段階Cのmerge経路としてその後も使う。

### 3.3 ステップ2：3段階のリスク別マージ

| 段階 | 対象 | mergeの条件 | mergeする人 | 創設者の関与 |
|---|---|---|---|---|
| **A 自律merge** | 担当として登録したpathだけを変える変更。初期案はNDPの`tools/nagi-developer-platform/**`、`tests/developer-platform/**`、`docs/development/developer-platform/**`、`reviews/ndp-*.md`。DEC-051の重要な変更に当たらないもの | 必須CIが成功し、mainと競合がない。独立したAIレビュー（DEC-051方式A）で重大度P1以上の未解決指摘がない。PRに目的、確認結果（1〜3行）、host-onlyであることを書いている | 共同開発者（auto-mergeの予約も可） | 事後に確認する。定期的な一覧を見て、問題があればrevertする |
| **B 通知付きmerge** | 担当外だが製品の判断を含まない、共同開発用の文書（`docs/development/`配下のstarter kit、割当案の状態欄、README索引など） | 段階Aの条件に加え、`ready-for-merge`ラベルを付けて24時間異議が出ないこと。それより前に創設者の統合パスでmergeされてもよい | 共同開発者、または創設者の統合パス | `hold`ラベルかコメントで止められる |
| **C 創設者承認** | `docs/product-decisions/**`、`docs/implementation_status.md`、`AGENTS.md`・`CONTRIBUTING.md`・LICENSE系・`SECURITY.md`、`.github/**`、agentやhookの設定（`.codex/`、`.entire/`、`.claude/`）、共有契約（`kernel/`、`idl/`、`schemas/`、`sdk/`、Cargo workspace／lock、`.dev/workstreams.json`など）、他workstreamのpath、DEC-051の重要な変更、release gateやmilestone状態の変更 | 創設者のApprove（CODEOWNERSで強制）。または創設者の統合パスでのmerge | 創設者（統合パス）、または承認後に誰でも | 事前に承認する |

補足:

- **複数の段階にまたがるPRは、高い方の段階に従う。** 段階Aの部分が止まらないよう、PRは段階ごとに分ける。
- **迷ったときの扱い。** DEC-051の重要な変更に当たるか迷う場合は段階Cにする。それ以外はDEC-049の「迷ったら進める」に従い、AかBで進める。DEC-049は製品の安全設計を対象外にしているので、この使い分けは両立する。
- **内容で決まる段階Cは、共同開発者がラベルを付けて申告する。** DEC-051の重要な変更はpathだけでは判別できないため、共同開発者が`tier:C`ラベルを付ける。AIレビューでもDeepSeek要否と同時に判定する。
- **段階Cも速く回す。** PRの冒頭に「決めること／推奨／影響範囲／戻し方」を書き、小さく保つ。創設者の時間は、差分を読む量ではなく判断の数に使ってもらう。
- **mergeするのは人。** AIが準備したPRでも、merge（auto-mergeの予約を含む）は共同開発者が内容を確認してから行い、その責任を持つ。

## 4. 仕組み：決定的なPolicyで強制する（ステップ2）

### 4.1 CODEOWNERS（既定は創設者、委ねたpathだけを開ける）

CODEOWNERSでは最後に一致した行が優先されます。所有者を書かない行は「所有者なし」となり、その行に当たるファイルにはCode Ownerの承認が要りません。`@FOUNDER`は創設者のGitHubアカウントに置き換えます。

```text
# 既定：明示的に委ねていないpathはすべて創設者の承認が必要（段階C）
*                                             @FOUNDER

# 段階B：共同開発用の文書（所有者なし。24時間の通知は運用ルールで守る）
/docs/development/

# 段階A：共同開発者の担当範囲（所有者なし。条件はCIと運用ルールで守る）
/tools/nagi-developer-platform/
/tests/developer-platform/
/docs/development/developer-platform/
/reviews/ndp-*.md

# docs/development/の中でも創設者が持つもの（後に書いた行が優先される）
/docs/development/entire-private-history.md   @FOUNDER
```

### 4.2 mainのRuleset

| 設定 | 値 | 理由 |
|---|---|---|
| branchの削除禁止、force push禁止 | ON | DEC-049 d |
| PR経由のmergeを必須にする | ON。必要なApprove数は0、Code Ownerのreviewを必須にする | 所有者のいるpath（段階C）を含むPRだけ承認が必要になる |
| 必須status check | 既存CI（`change-scope`、`ubuntu-host`など。名前は設定時に確認する）と4.3のNDP試験 | 段階Aの「動くこと」を機械的に確認する |
| merge前にbranchを最新にする要求 | OFF | CIの再実行による待ちを増やさない。競合はGitHubが検出する |
| bypass | Repository admin（創設者） | 創設者の現行手順（統合パス、mainへの小さな文書の直接変更）を変えない |
| repository設定 | auto-mergeを許可する。secret scanningとpush protectionをONにする | 条件がそろえば自動でmergeされる。公開repositoryへの秘密情報の混入を防ぐ（DEC-049 c、DEC-050） |

設定する前に、次の点をテストPRで確かめます（いずれも未検証）。

- 「必要なApprove数0」と「Code Ownerのreview必須」を組み合わせたとき、所有者のいないpathだけを変えるPRは承認なしでmergeでき、所有者のいるpathを含むPRは止まること。
- `ci.yml`には`paths-ignore`があるため、workflow自体が起動しないPR（例：stateファイルだけの変更）では必須checkが保留のまま止まる。必須にするjobと対象を確認する。なお、`if`条件でskipされたjobは成功として扱われる。

### 4.3 NDPの試験をCIで走らせる

段階Aの条件を自己申告ではなくCIで確認できるよう、NDPのPython試験を実行するworkflowを追加します。たとえば`tools/nagi-developer-platform/**`か`tests/developer-platform/**`が変わったときに、試験のあるdirectoryごとに`python3 -m unittest discover`を実行します。`.github/**`の変更になるため、段階Cとして創設者の承認を受けます。ステップ1の間に追加しておけば、統合パスでもNDP試験の結果を確認できます。

## 5. 運用

### 5.1 1 taskの流れ

1. mainから作業branchを作る。NDP統合branchには溜めず、mainを直接の統合先にする。
2. 実装、試験、独立したAIレビュー（DEC-051方式A）を行う。
3. 1 task＝1 PRでmainへ出す。大きさの目安は、試験fixtureを除いて1,000行以内（提案値）。ステップ2では段階のラベル（`tier:A`、`tier:B`、`tier:C`）も付ける。
4. ステップ1では`ready-for-merge`を付けて創設者の統合パスを待つ。ステップ2の段階Aは、条件を満たした時点で共同開発者がmergeする。どちらも創設者と同じmerge commit方式にする。
5. レビューの指摘は、DEC-049のとおり後続のcommitで直す。

NDP統合branch `codex/ndp-host-integration`は、#166を分割してmainへ入れた後は新しい積み上げに使いません（実験用に残すのは構いません）。

### 5.2 判断待ちをmergeから切り離す

- 創設者への質問は、`founder-decision`ラベルを付けたissueにする。1件ずつでも、1つの判断キューissueにまとめてもよい。各項目には「決めること／選択肢／推奨／決まるまでの仮定／影響するtask」を書く。
- コードはPROVISIONALな仮定を置いたままmergeし、決定が出たら後続のcommitで差し替える（DEC-049の「迷ったら進める」、DEC-111の「実装を妨げる判断だけを先に扱う」）。
- 決定はOPERATING_RULESに従ってDECに記録する。DECの変更は段階Cのまま扱う。

### 5.3 創設者から見えるようにする

- `ready-for-merge`の付いたPRは、GitHubで`is:pr is:open label:ready-for-merge`と検索すれば一覧できる。統合パスの対象を集めるときにも使える。
- ステップ2の段階A/Bのmergeは、`is:pr is:merged base:main label:tier:A`のように検索すれば一覧できる。週に1回（提案値）、共同開発者が「mergeの一覧／各1行の結果／未解決の事項」を判断キューissueに投稿する。
- レビュー結果を`reviews/<レーンID>.md`に残す既存の運用は続ける。

### 5.4 戻し方と停止

- 創設者は、共同開発者がmergeした変更を理由を説明せずにいつでもrevertできる（「先に戻し、後で話す」）。merge commitは`git revert -m 1 <merge commit>`の1回で戻せる。共同開発者は、必要なら段階Cとして出し直す。
- 段階Aのmergeでmainの必須CIが壊れた場合は、共同開発者が当日中に修正するかrevertし、直るまで自律mergeを止める（提案値）。
- 創設者は一言で段階A/Bを止められる。止めている間はステップ1（全件を創設者の統合パスでmergeする運用）に戻る。どの場合も履歴の書き換えやforce pushはしない。

## 6. 段階的に広げる（まずB、将来はC）

| フェーズ | 内容 | 移る条件 |
|---|---|---|
| Phase 0（ステップ1） | 共同開発者のPRを`ready-for-merge`で創設者の統合パスに乗せる。NDP試験のCIを追加する | 創設者の了承（統合指示への1行追加） |
| Phase 1（ステップ2開始） | 段階A＝NDPの4 path、段階B＝`docs/development/`。CODEOWNERSとRulesetを設定する | 例として、ステップ1で共同開発者のPRが5件ほど問題なく統合された時点（提案値）で、創設者が判断する |
| Phase 2 | 共同開発者をDeveloper PlatformのIntegration Ownerに指定し、registryにNDPの行を登録する（`merge_boundary`には本フローを書く）。必要なら共有のCargo lockなどの一部を段階Bへ移す | 例として、2週間、またはrevertのない段階A mergeが20件（いずれも提案値）たまった時点で、創設者が判断する |
| Phase 3 | 共同開発者が増えたら、段階Bと段階Cの一部（製品判断を除く）を共同開発者どうしの相互レビューで代替する。創設者は製品判断とDECに集中する | 創設者の判断 |

委ねる範囲は、実績が積み上がっても自動的には広がりません。広げるかどうかは毎回創設者が決めます。製品側のDEC-074にある「役職や成功申告だけでは権限は生まれない」と同じ考え方です。

## 7. 既存PRへの当てはめ

### #166の分割案（+10,683行の内訳）

| 分割後のPR | 内容 | 規模 | ステップ1 | ステップ2 |
|---|---|---|---|---|
| ① | NDP本体：`tools/nagi-developer-platform/`、`tests/developer-platform/`、`docs/development/developer-platform/`、`reviews/ndp-*` | 81ファイル、+10,446行 | 統合パス | A |
| ② | 共同開発用の文書：starter kitの状態欄、割当案 | 17ファイル、+105行／−15行 | 統合パス | B |
| ③ | DEC-046と決定索引（#105を最新のmainに合わせて統合する） | 2ファイル、+11行／−2行（＋#105） | 統合パス | C |
| ④ | `docs/implementation_status.md` | 1ファイル、+23行 | 統合パス | C |
| ⑤ | `.codex/hooks.json`、`.entire/` | 3ファイル、+98行 | 統合パス（#166の確認事項1。別PRにするか、外すか） | C |

①は既存の13件のtask PR（#165、#167〜#178）でレビュー済みの内容をまとめたものです。ステップ1の統合パスで扱いやすくするため、必要ならUI解析（NDP-06／07）、API差分rules（NDP-04／05）、雛形とStage検証（NDP-02／03／09）、評価（NDP-11）にさらに分けます。#166の確認事項7件は、第5.2節の判断キューへ移します。

## 8. 変えないこと

- DEC-049の継続制約a〜f。force pushと共有履歴の書き換えは、Rulesetでも禁止する。
- 製品の安全設計、release／activation／acceptance gate、milestoneの状態。mergeはhost-onlyの統合であり、target受入ではない。
- DEC-047のLinux移行laneと統合branch `codex/linux-integration-20261114` の運用。この提案が扱うのは、NDPなど共同開発者の作業からmainへのmergeである。
- 製品の方向と担当横断の判断は、最終的に創設者が行う（DEC-046）。
- 創設者の統合パスの進め方。この提案は、そこへ共同開発者のPRを加えることだけをお願いする。
- 秘密情報や個人情報を公開repositoryに出さない（DEC-050）。

## 9. 創設者に決めてほしいこと

1. **ステップ1を始めてよいか。** 統合の指示に「`ready-for-merge`ラベルの付いた共同開発者のPRも対象に含める」を加え、第3.2節の扱い（コードは書き換えず、問題があれば飛ばして戻す。集約しない）で統合してもらえるか。
2. #166を分割して出し直す方針と、7件の確認事項を判断キューへ移すこと。
3. NDP試験のCIを追加するPR（段階C）を受けてもらえるか。
4. **ステップ2へ進む条件。** 3段階のリスク別マージを採用するか（そのまま採用／修正して採用／見送り）、段階Aの初期path（NDPの4つ）、段階Bの待ち時間（提案は24時間）、Phase 1へ移る時期。
5. ステップ2に進む場合のRulesetとsecret scanningの設定。admin権限が必要なため、創設者に行ってもらう。CODEOWNERSのPRは共同開発者が用意し、段階Cとして承認を受ける。

返答の例：「ステップ1を承認する。ステップ2は、共同開発者のPRが5件統合されてからもう一度判断する。」

承認されたら、共同開発者がOPERATING_RULESに従って決定を記録します。DEC-046の未決定事項「branch／PR／review運用」を更新し、詳細は新しいDECに置きます。
