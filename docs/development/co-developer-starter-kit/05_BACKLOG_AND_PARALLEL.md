# 12タスクと並行開発キュー

IDと目的は公開済みの `CO_DEVELOPER_ALLOCATION_PROPOSAL.md` 第7節に合わせています。以前の会話中の12件の番号ではなく、**NDP-01〜12**を使います。詳細成果・テスト案は各taskファイルにあります。初期状態は全件 `PROPOSED / NOT STARTED`。文書作成を着手と数えません。

| ID | タスク | 主な依存 |
|---|---|---|
| NDP-01 | 担当・契約・開始条件のread-only照合 | なし |
| NDP-02 | 既存SDKを使うLevel 0雛形契約 | 01、SDK owner |
| NDP-03 | 雛形generator pilot | 02、承認済みpath |
| NDP-04 | Windows API差分rules | 01、PT-01正規interface |
| NDP-05 | Linux API差分rules | 01、PT-01正規interface |
| NDP-06 | 宣言的UIの静的解析pilot | 01、形式・UI契約 |
| NDP-07 | UI対応表・未対応レポート | 06、UI owner |
| NDP-08 | 正規tool利用表とStage契約 | 01、CLI/Package owner |
| NDP-09 | 薄い検証オーケストレーター | 08、実行Scope |
| NDP-10 | 評価fixtureの選定・権利・作成準備 | 01 |
| NDP-11 | 比較runと実測・改善 | 10、利用可能な02〜09 |
| NDP-12 | 分野内統合と引渡し | 関係する成果、統合owner |

## フェーズA: 初回は1セッション

ClaudeでもCodexでもよいので、NDP-01を読んで照合する。既存のread-only scanner、PT-01記録、共有SDKを二重実装しないための接続点を短く整理する。Founderの開始記録と必要な登録を確認する。全OSのビルドや全ての将来契約の完成を待つ工程にはしない。

## フェーズB: 最初の並行候補

開始条件と独立したpathが揃ったものだけを始める。3〜4件は初期案であって最低ノルマでも固定上限でもない。

| セッション例 | 初回 | 次の候補 |
|---|---|---|
| Claude A | NDP-04 Windows rules | 04の改善、別担当成果のレビュー |
| Codex B | NDP-05 Linux rules | 05の改善、NDP-08 |
| Claude C | NDP-06 UI解析 | NDP-07 |
| Codex D | NDP-10 fixture | NDP-02、NDP-11の準備 |

これは得意不得意の断定ではなく割当例。契約・利用枠・実行環境に合わせて交換できる。片方のAIしか使えない日も同じtaskを引き継げる。

Windows/Linux rulesがPT-01接続待ちなら、その2件の実装を自己許可しない。承認済みのUI/fixtureや契約調査を先行する。全体を一律に停止しない。

## フェーズC: generatorと検証を接続

NDP-02→03、NDP-08→09を進める。SDK配送・API不足を仮のpackageや偽実装で埋めない。対応済み範囲を小さく確実に増やす。NDP-11で使える工程から比較し、NDP-12は各小成果の節目で行う。

## 並列事故を避ける具体策

- task・branch・編集pathを一対一にする。共通contract、workspace、lock、BOARDの書込み担当は1人にする。
- 共有contract変更は調整担当が1つの小commitにし、各laneが同じ版へ追従する。各laneで別schemaを正本にしない。
- 同じPCではbuild出力、ログ、一時ファイル、ローカルテスト用portを分ける。resource不足時は実行数を減らし、許可済みの軽い作業へ移る。
- AIのセッション数で進捗を測らない。動く成果とテスト結果を見る。
- 自動継続は実行中セッションの能力の範囲。閉じたAIや利用枠切れのAIが勝手に再起動するとは考えず、`prompts/04_RESUME.md`で再開する。

## 使う指示文

実装: `prompts/01_CLAUDE_IMPLEMENT.md` または `02_CODEX_IMPLEMENT.md`。
調整: `prompts/05_COORDINATE.md`。
レビュー: `prompts/03_REVIEW.md`。
再開: `prompts/04_RESUME.md`。
