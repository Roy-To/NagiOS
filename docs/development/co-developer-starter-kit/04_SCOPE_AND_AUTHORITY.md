# 担当範囲・権限・作業場所

## 1. 確定事項と今回の提案

CONFIRMEDなのは、創設者の全体方針・最終判断と、共同開発者の割当領域内の自律性です。PT-02〜06の具体的な引受け、path、branch、host-only checkpoint、並列数は既存の割当案で未確定です。本キットの具体名は **PROVISIONAL** で、開始記録が承認されて初めて実行に使います。

| 判断 | 担当 |
|---|---|
| Nagi全体の製品方向、共通APIの意味、安全設計、release | Founderと該当owner |
| 割当内の内部構成、アルゴリズム、テスト、仕事の順番 | Core Developer |
| 既存PT-01/SDK/Package/UIへの接続 | Core Developerが調査・提案し、該当ownerと整合 |
| 本線のRegistry、CI、共有workspace、merge先 | 指定Integration Owner |
| 自分のAI・GitHubの認証、ローカルフォルダの許可 | 受取開発者本人 |

## 2. 新規pathの候補

先の会話で挙がった `tools/nagi-porting/**` はPT-01の候補所有範囲と重なります。**このキットではそこを新担当の共通作業場所にしません。**

NDP-01で現物と既存担当を確認するための候補:

| 用途 | 候補path |
|---|---|
| PT-02 generator | `tools/nagi-developer-platform/generator/**` |
| PT-03 Windows rules | `tools/nagi-developer-platform/api/windows/**` |
| PT-03 Linux rules | `tools/nagi-developer-platform/api/linux/**` |
| PT-04 UI | `tools/nagi-developer-platform/ui/**` |
| PT-05薄い連携 | `tools/nagi-developer-platform/validation/**` |
| 接続契約・内部共通部 | `tools/nagi-developer-platform/contracts/**`（調整担当だけ） |
| 分野別テスト | `tests/developer-platform/<task-or-domain>/**` |
| PT-06専用fixture | `tests/developer-platform/evaluation/**` |
| 進捗・担当内資料 | `docs/development/developer-platform/**` |

これらは存在・空き・利用許可を保証しません。既に実装がある場合は同機能を別名で複製せず、ownerを確認する。実装言語・crate構成・内部schemaは技術提案であり、新しい公開SDK契約ではありません。

## 3. 初期対象外

`kernel/**`、`loader/**`、`user/nagi-init/**`、`user/libnagi/**`、`sdk/**`、`idl/**`、`tools/nagi-pkg/**`、`tools/nagi-cli/**`、`tools/nagi-porting/**`、`docs/porting/multiarch/**`、共有 `Cargo.toml` / `Cargo.lock` / CI / registryは、この初期分離案の直接編集先ではありません。

必要な変更が見つかれば、理由と最小差分を関係ownerへ渡す。DEC-049で既に明示許可された変更は同じ承認を取り直さないが、他laneへの許可を自分のものとみなさない。

## 4. branchと作業フォルダの候補

- 分野内統合: `codex/ndp-host-integration`
- task例: `codex/ndp-04-windows`、`codex/ndp-05-linux`、`codex/ndp-06-ui`、`codex/ndp-10-fixtures`
- PRのbaseは開始記録で選ぶ。`main`にもLinux移行統合branchにも自動mergeしない。

branch名は提案です。実際の命名規則・registryに合わせて確定します。1つのフォルダでbranchを切り替えながら複数AIを走らせません。独立した作業フォルダ・生成物を用意できない場合は書込みを1セッションにし、レビューを順番に行います。

## 5. GitHub権限

公開資料の閲覧と本体へのWriteは別です。自分のforkを使う案と本体collaboratorを使う案を開始時に選びます。本キットは招待、権限付与、保護解除をしません。pushできない場合に他人のtokenを借りず、許可済みローカルcommitを保全して権限条件を報告します。

## 6. 未確定をまとめる

`templates/START_AUTHORIZATION.md`を使い、実在する値だけで開始記録を1つ作ります。既にある記録へリンクするだけでも構いません。関係ownerが未回答の境界を確認済みにせず、未解決taskだけを外して開始可能な群を提案します。
