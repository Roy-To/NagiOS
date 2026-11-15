# 開始条件と一括承認 — テンプレート

**状態: DRAFT / 未承認。** コピーや公開だけでAPPROVEDにしません。NDP-01で実在する条件を埋め、Founderの実際の承認と必要なowner確認・登録を参照します。これは開始時1回の記録で、各作業ごとに増やしません。

| 項目 | 実際の確定内容 |
|---|---|
| 承認日・承認根拠 | 未確定。チャット識別子は書かない |
| Founder / Core Developer / Integration Owner | 役割と担当。公開不要な個人情報は書かない |
| 許可task群 | 未確定。NDP-01〜12から具体指定 |
| 作業環境 | 本人が選択した専用環境。個人絶対パスは非公開 |
| 基準repository / ref / commit | 未確定 |
| 実装branch / PR base / merge boundary | 未確定 |
| 編集path / 他ownerとの競合照合 | 未確定 |
| 共通contract更新担当 | 未確定 |
| host-only checkpoint / workstream登録 | 未確定。既存記録があれば参照 |
| PT-01正規入力・report接続 | 未確定。NDP-04/05の条件 |
| SDK配送・Manifest接続 | 未確定。NDP-02/03の条件 |
| UI形式・現行UI契約 | 未確定。NDP-06/07の条件 |
| 正規tool・実行Scope | 未確定。NDP-08/09の条件 |
| fixture・外部AIへ渡す範囲 | 公開Nagiコードと許諾済み対象のみ。追加対象は未確定 |
| 許可Git操作・push先 | 未確定。共有履歴書換え・main直接pushは含めない |
| clone/worktree・依存導入・資源 | 未確定。新しい課金は含めない |
| 初期並行数とpath分割 | 3〜4は案。実環境で確定 |
| DeepSeek reviewの既存担当への経路 | 未確定。credentialは共有しない |

未確定の依存が残るtaskは対象から分け、条件の揃ったtask群だけ先に承認できます。表のすべてを埋めるために無関係な作業まで止めません。必要な本線registryの変更は指定Integration Ownerが行い、既存ownerを消して登録しません。

確定後の技術詳細、実装順、テスト追加は割当範囲内のCore Developerの裁量とし、同じ開始承認を繰り返し取り直しません。
