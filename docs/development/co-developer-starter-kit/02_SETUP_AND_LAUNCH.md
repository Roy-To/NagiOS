# 環境の準備とClaude / Codexへの投げ方

## 1. 使う場所

実装は、repositoryを読み書きしコマンドを実行できる **Claude CodeまたはCodex** で行います。通常の相談チャットへMDを添付するだけでは、ローカルのソースが自動で編集されるわけではありません。インストール・利用可否・利用枠は本人の契約と現在の公式案内で確認してください。

- Claude Code: https://code.claude.com/docs/en/quickstart
- Claude CodeのVS Code利用: https://code.claude.com/docs/en/vs-code
- Codex: https://developers.openai.com/codex/quickstart/
- ChatGPT契約でのCodex: https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan

本キットは特定のモデル名、API課金、Fastモード、新しい有料サービスの購入を前提にしません。各自の契約内で利用できる実装向けモデルを使います。

## 2. repositoryを用意する

既存の割当済みcheckoutがあれば再利用します。新しいcheckoutが必要なら、本人が保存場所を選び、創設者の開始記録に含めてから取得してください。創設者のMacやSSDのパスはコピーしません。

新規取得が許可され、Gitが利用できる場合の例:

```sh
git clone https://github.com/Roy-To/NagiOS.git NagiOS-DevPlatform
cd NagiOS-DevPlatform
git status --short
git branch --show-current
git rev-parse HEAD
```

これは上流のread用remoteであり、push権限を与えるコマンドではありません。fork方式なら、本人のGitHubで作成した実在forkをpush先に設定します。URLやアカウントを推測せず、認証は本人が画面で行ってください。トークンをチャットに貼りません。

`git pull`、branch切替、mergeは作業中の差分と他セッションを確認してから行います。初回は既存rootの `AGENTS.md` や `CLAUDE.md` を上書きしません。

## 3. この一式を置く

repositoryにキットが入っていればそのまま使います。ZIPだけを受け取った場合は、中身を `docs/development/co-developer-starter-kit/` に置く案を使えます。ただし同名ファイルが既にあれば上書きせず、版を比較してください。キットをrepository外に置き、AIへ実在するパスを指定する方法でも構いません。

MDは初期資料であり、自動インストーラーではありません。ZIPをAIへ添付しただけで展開・実行されたとみなしません。

## 4. Claude Codeで開始

Claude Codeのデスクトップ／IDEで割当フォルダを選ぶか、CLIが導入済みならそのフォルダで実行します。

```sh
claude
```

最初は [prompts/00_BOOTSTRAP.md](prompts/00_BOOTSTRAP.md) の指示文を貼ります。開始条件の確認後、実装用には [prompts/01_CLAUDE_IMPLEMENT.md](prompts/01_CLAUDE_IMPLEMENT.md) を使います。

Claude Codeの `CLAUDE.md` とCodexの `AGENTS.md` は同じ仕組みと決めつけません。本キットでは両方の既存指示と共通規則を明示的に読むよう依頼します。自動読込用ファイルの新設は必須にせず、最初から `/init` で共有指示を再生成しません。

## 5. Codexで開始

Codexのアプリ／IDEで割当フォルダを選ぶか、CLIが導入済みならそのフォルダで実行します。

```sh
codex
```

本人のChatGPTアカウントでサインインし、[prompts/00_BOOTSTRAP.md](prompts/00_BOOTSTRAP.md)を貼ります。実装は [prompts/02_CODEX_IMPLEMENT.md](prompts/02_CODEX_IMPLEMENT.md)、レビューは [prompts/03_REVIEW.md](prompts/03_REVIEW.md)です。

Codexは `AGENTS.md` の指示を利用します。ただし別ファイルのDEC全文やこのキットが全て自動的に読み込まれたとは考えず、最初に参照ファイルとrefを確認させます。

## 6. 各タスクの投げ方

開始文の `TASK` を `NDP-04` のように1つ指定し、その担当MDを読ませます。開始承認、実際のbranch/baseと変更pathを一緒に渡します。実行済みの開始条件をもう一度承認依頼に戻させません。

同じ作業フォルダにClaudeとCodexを同時に入れて書き込ませないでください。並行時は承認された独立作業フォルダを使い、branch、編集path、build出力を分けます。branch名だけ違っても、同じフォルダなら安全な並行編集にはなりません。

## 7. 必要な依存だけ入れる

Nagiの正規資料は `CONTRIBUTING.md`、`docs/developer-preview/README.md`、`rust-toolchain.toml`、`nagi.toml` です。担当コードが実際に要求するツールを確認し、既存の手順と開始許可の範囲で導入します。

初期の静的解析・fixture試験のためにOS全体のビルド、全依存取得、QEMU、モデルのダウンロードを一律に始めません。`./nagi fetch`、`build`、`image`、`run` は名称を知っているだけで実行してよいものではなく、担当の検証に必要かを確認します。

Windowsでもhost側の作業とLinux/Nagi target試験を区別します。macOS固有パスや未確認のPowerShell/WSLコマンドを推測して実行しません。

## 8. Web上のコード実行環境を使う場合

本人の利用可能なコード実行機能で対象repositoryとbranchを明示選択します。MDをそのbranchへ配置するか、開始文と担当MDを実際に読める方法で渡します。ネットワーク、保存、push、テスト実行の可否を確認し、ローカルと同じ環境だと仮定しません。終了前に保存先とcommitを確認します。

## 公式の指示ファイル資料

- Codex AGENTS.md: https://developers.openai.com/codex/guides/agents-md/
- Claude Code memory: https://code.claude.com/docs/en/memory

上記のツール操作情報は2026-11-15に確認しました。Nagiの製品仕様・権限をこれらの一般資料から補いません。
