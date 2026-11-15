# Codexレビュー — NDP host pilots

日付: 2026-11-15。実装担当とは別の独立サブエージェント（gpt-6-luna、reasoning=max）が読取専用でレビューした。branch切替・ソース変更・commit・外部送信はしていない。UI上の新しいチャット作成や速度設定の操作は行っていない。

## 固定対象

| 対象 | BASE | HEAD |
|---|---|---|
| NDP-10初回 | e8071ec | 715d308 |
| NDP-06初回 | e7900aa | cb37e91 |
| 修正の再レビュー | f22d07d | 9b68295 |

## 初回の指摘と修正

- **P2 / 独立担当:** `tools/nagi-developer-platform/ui/analysis/analyze.py` のcustomwidget/class/extends/header宣言が既知タグ扱いのまま結果から欠落していた。下流が基底型・headerを回収できない。9b68295で宣言とmetadataをデータとして保持し、headerを読み込まない回帰テストを追加した。
- **P2相当 / 実装担当の追加再現:** QGridLayoutのitem row/columnが異なっても解析結果が同じだった。9b68295でlayout/item属性、子objectの属性・properties、spacer情報を保持した。別担当が再レビューで妥当性を確認した。
- NDP-10には確証のある指摘なし。portable Cのstrict Clang build/runを独立担当も実行し、正常・拒否・入力/出力不変の確認が成功した。

## 再レビュー結果

**未解決のP1/P2なし。** 初回P2とgrid配置の欠落は解消。custom widgetはrecognizedへ昇格せず、header・binding・resource・eventは読み込み/実行されない。byte/depth/node上限、encoding/DTD/entity/PI拒否、dirfd相対open・symlink/FIFO拒否等の既存境界は保たれている。

確認コマンドは `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/nagi-developer-platform/ui/analysis -p 'test_*.py' -v`。独立担当の初回16件・再レビュー18件はいずれも成功。実装担当は追加2回帰テストを旧cb37e91に対して実行し、custom_widgets欠落とgrid配置同一を実際に検出した。

確認メモ: customwidget内の未知の入れ子はunknownにタグ名を出すが、その全属性値までは保存しない。現pilotは完全なQt metadata保存を約束しておらず、未対応の存在は可視のため、今回は指摘扱いにしない。

## DeepSeek・未検証

[DEC-051方式A](../docs/product-decisions/DEC-051-review-methods.md)の追加条件は非該当と独立担当が判断。対象は合成入力と静的UIのin-memory解析であり、OS権限・認可等の列挙された重要変更ではない。指摘は解消し、見解不一致・未解決項目もないため、今回の追加DeepSeekレビューは不要。実際のDeepSeek送信はしていない。

host限定の結果であり、Nagi target、Qt rendering、実イベント、SDK validator、性能比較、上流統合の成功ではない。
