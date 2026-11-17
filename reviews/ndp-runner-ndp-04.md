# NDP-04 自動レビュー記録

## Round 1 — claude review of 7b198c19d95e

P1/P2 の指摘はありません。ただし Bash が拒否されたため、レビューは diff の読み取り中心です。テストは再実行しておらず、runner が出した結果（30件成功）に依拠しています。

## 確認できたこと
- 変更は担当 path（`api/windows/**`、`api/common/**`、`tests/developer-platform/api/windows/**`）内だけです。
- 新しい filesystem scanner はありません。入力は stdin の bytes だけで、report 内の path を開くコードもありません。`--path` の許可リストと照合し、symlink 逸脱の余地はありません。
- 入力には上限があります。byte 1MiB、finding 4096件、evidence 4096文字、JSON 深さ32、値数5万、整数20桁です。重複 key、NaN/Infinity、不正な path は拒否されます。rules は同梱 JSON のみで、report から選べません。外部取得や任意コード実行もありません。
- 参照先 `sdk/README.md`、`nagi_sdk.h`、`APP_SDK_CONTRACT.md`、`state.rs`、`lib.rs` は実在します。`AppStateBackend`（`state.rs:27`）と `Application::presentation`（`lib.rs:33`）も確認しました。代替候補はこの2つだけで、いずれも `contract_only` と制約付きで書かれており、架空 API はありません。
- 出力は `compatibility=not_assessed`、`target_acceptance=not_run`、`approval=PROVISIONAL` に固定されています。`review_status=approved` でも昇格しません。
- README の「仮定（PROVISIONAL）」節があり、Nagi target・実 Windows・正式 PT-01 は未試験と明記されています。
- テストは次の点を実装に対して検証しています。
  - fixture の evidence と `read_prefix.c` の行を突き合わせています。
  - 全 rule・全 alias を網羅しています。
  - 負例（`text_only`）、壊れた report、巨大 report、深い入れ子、決定性、入力不変を扱っています。

## P3（軽微）
1. **`api/windows/diagnose.py:11-21`**
   - 起きる条件: `#include <Windows.h>`（大文字）や `#include "windows.h"`（引用符形式）が来たとき。
   - 結果: alias 照合が大文字小文字を区別します。引用符形式は文字列として除外されるため、`WIN-HEADER` にも一致しません。どちらも `unknown_api` 扱いになり、誤認定ではありません。
   - 修正案: header 系の alias だけ大文字小文字を無視して照合する。あわせて `#include "..."` の扱いをテストするか README に限界として書く。
2. **`api/windows/diagnose.py:119-125`**
   - 出力は evidence 本文を落とし、SHA-256 と位置だけを残します。機密を再掲しない意図は README に書かれています。
   - 一方で、reviewer が元 report を引かないと根拠を確認できません。タスク記述の「根拠」をハッシュだけで満たすかは設計判断です。
3. **`api/windows/test_windows.py` 全体**
   - 大文字小文字違いの header と、`#include "…"` 形式を検証するテストがありません。上記 1 と同じ理由で、限界を固定するテストを足すとよいです。

## 疑問（確証なし）
- `Process.Start` や `FileStream` のような短縮名は、同名のローカル型でも候補に出ます。README に限界として明記済みなので、現状は許容範囲と見ています。
- 評価入力は `windows/read_prefix.c` の1本だけです。Win32 以外（CRT・.NET）は合成 evidence の単体テスト止まりです。これはタスクの「代表例から」という範囲内です。

NDP-REVIEW: PASS

