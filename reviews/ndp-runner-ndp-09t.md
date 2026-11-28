# NDP-09t 自動レビュー記録

## Round 1 — codex review of cd034dfa074f

**P2 が2件あります。** 指定差分は担当パス内の3ファイルに収まり、タスク文書の既存の目的・成果・Scope・権限の記述は維持されています。状態は `PARTIAL` に更新され、host限定の実績と、Core Developerの委任に基づく未承認の `PROVISIONAL` 判断が記録されています。

- **P2 — Stage 3受入を「実施済み」に変えてもテストが通る。** `test_task_doc_status.py:89` は `Stage 3`、`Nagi target`、`未実施` を個別に探すため、受入の箇条書きだけを「Stage 3（Nagi target）の受入は実施済み。」に変えても、見出しに `未実施` が残っていれば通ります。この変更をメモリ上で適用して再現しました。最小修正は、Stage 3とNagi target受入の未実施を同じ文または箇条書き内で検査し、その記述だけを「実施済み」に変える変異テストを追加することです。 [test_task_doc_status.py](tests/developer-platform/evaluation/test_task_doc_status.py:89)
- **P2 — 根拠リンクの解決先がリポジトリ外でも検査・コピーされる。** `test_task_doc_status.py:79` は解決後のパスの存在だけを確認し、`:124` からの一時コピーもコピー元・先の境界を確認しません。追加の根拠リンクが `../` の過剰指定やsymlinkで既存のリポジトリ外ファイルを指すと、存在検査が通り、一時コピー処理がそのファイルを読みます。相対パスの階層によってはコピー先も一時ディレクトリ外になります。最小修正は、解決済みのコピー元がリポジトリ内、コピー先が一時ディレクトリ内にあることを、存在検査・ディレクトリ作成・コピーの前に確認することです。 [リンク検査](tests/developer-platform/evaluation/test_task_doc_status.py:74) [一時コピー処理](tests/developer-platform/evaluation/test_task_doc_status.py:124)

`NOT STARTED` に戻した場合は、メモリ上で状態検査が失敗することを確認しました。全スイートと一時コピーを使うテストの結果は提示されたrunnerログに基づきます。独立レビューではファイル変更や外部プロジェクトのビルドを行っていません。

NDP-REVIEW: FIX

## Round 2 — codex review of a381d06a51bd

P2が3件あります。指定HEADのタスク文書は `PARTIAL` を記録し、host限定の実績、Stage 3と実 `nagi-pkg`・PT-01 scanner接続の未実施、Core Developerの委任に基づく未承認の `PROVISIONAL` 判断を明記しています。目的・成果・Scope・権限の記述に差分はありません。

- **P2 — 担当パス外に前回のレビュー記録が追加されています。** 指定のBASEからHEADまでを取り込むと、前回ラウンドの `FIX` 判定を含む `reviews/ndp-runner-ndp-09t.md` も入ります。最小修正は、このファイルを統合対象の差分から外すことです。 [ndp-runner-ndp-09t.md](reviews/ndp-runner-ndp-09t.md:1)
- **P2 — 状態を `DONE扱い` に変更しても、補足に `PARTIAL` が残ると検査を通ります。** たとえば状態の値を `DONE扱い（旧状態はPARTIAL）` とすると、`assertIn("PARTIAL", line)` は通り、Pythonの正規表現では `E` と `扱` の間に単語境界がないため `\bDONE\b` にも一致しません。最小修正は、`/` の直後にある状態欄を取り出し、補足の前の値が厳密に `PARTIAL` であることを検査することです。 [test_task_doc_status.py](tests/developer-platform/evaluation/test_task_doc_status.py:99)
- **P2 — 実績欄にある整合テスト自身のパスが存在検査から漏れています。** `test_task_doc_status.py` への参照はインラインコードなので、Markdownリンクだけを抽出する検査では、この参照を存在しないファイル名に書き換えても通ります。最小修正は、この参照をほかの根拠と同じ相対Markdownリンクに変え、必須根拠に加えることです。 [タスク文書](docs/development/co-developer-starter-kit/tasks/NDP-09_VALIDATION_ORCHESTRATOR.md:48) [リンク抽出](tests/developer-platform/evaluation/test_task_doc_status.py:83)

前回指摘の箇条書き変更とリポジトリ外リンクに対する修正は差分で確認しました。`NOT STARTED` に戻した一時コピーの変異テストは実装を確認し、実行結果は提示されたrunnerの報告に基づきます。独立レビューではスイートの再実行、ファイル変更、外部プロジェクトのビルドは行っていません。

NDP-REVIEW: FIX

## Round 3 — codex review of ccc0f228bfb1

指定された BASE–HEAD 差分に P2 が2件あります。

- **P2 — 担当パス外のレビュー記録が統合差分に残っています。** 指定の差分をそのまま取り込むと、runner が追加した過去ラウンドのレビュー記録も入ります。最小修正は、runner 側の統合対象からこのファイルを除外することです。前回からの未解決事項です。 [reviews/ndp-runner-ndp-09t.md](reviews/ndp-runner-ndp-09t.md:1)
- **P2 — タイトル付き Markdown リンクのリンク切れを見逃します。** 実績欄の `handover.md` リンクを `[handover.md](../../developer-platform/missing.md "Handover")` に置き換えると、リンク抽出がその参照を読み飛ばし、存在検査が通ります。メモリ上で再現しました。最小修正は、任意のリンクタイトルを許容しつつリンク先を抽出し、この変異で存在検査が失敗することを確認することです。 [test_task_doc_status.py](tests/developer-platform/evaluation/test_task_doc_status.py:93)

タスク文書の変更は状態行と実績欄に限られています。`PARTIAL`、host 限定の実績、Stage 3 の target 受入と実 `nagi-pkg`・PT-01 scanner 接続の未実施、委任に基づく未承認の `PROVISIONAL` 判断を確認しました。指定 HEAD と一致する文書・テストで静的検査6件が通り、状態値をメモリ上で `NOT STARTED` に戻すと状態検査が失敗しました。一時コピーを使う変異テストと全スイートの実行結果は、提示された runner の報告に基づきます。

NDP-REVIEW: FIX

## Round 4 — codex review of 8afbf15558d0

**P2 — 空白を含む山括弧形式の根拠リンクが存在検査をすり抜けます。** [test_task_doc_status.py:95](tests/developer-platform/evaluation/test_task_doc_status.py:95)

実績欄に `[補足](<../../developer-platform/missing notes.md>)` を加えると、リンク先が存在しなくても検査が通ります。山括弧付きのリンク先にも空白を禁止するパターンが適用され、リンク自体が抽出されないためです。文書をメモリ上で変更し、既存の存在検査が通ることを再現しました。最小修正は、`<...>` 形式を空白を許す分岐で抽出し、存在しない空白付きパスを加えた変異テストで失敗を確認することです。

指定HEADでは、状態が `PARTIAL` として記録され、Core Developerの委任に基づく `PROVISIONAL` 判断であることと、Stage 3／Nagi target受入および実ツール接続が未実施であることを確認しました。既存の目的・成果・Scope・権限の記述に差分はなく、現行の根拠リンク先はすべて実在します。全テストの結果はrunner報告に基づきます。レビューでは上記の再現だけを実行しました。

NDP-REVIEW: FIX

## Round 5 — codex review of 8bd4ac79f0c2

**P2 — 参照形式の根拠リンクは、リンク先が存在しなくても検査を通ります。** 実績欄に `[移植メモ][migration-notes]` と定義 `[migration-notes]: ../../developer-platform/missing-migration-notes.md` を追加すると、検査は `](` で始まる形式だけを調べるため、この参照を見落とします。文書をメモリ上で変更し、リンク先が存在しない状態で既存テストが成功することを確認しました。最小修正は、実績欄の根拠をインラインリンク形式に統一し、参照形式のリンク定義があれば検査を失敗させることです。 [test_task_doc_status.py:108](tests/developer-platform/evaluation/test_task_doc_status.py:108)

指定HEADの文書は `PARTIAL` と未承認の `PROVISIONAL` 判断を記録し、host限定の実績、Stage 3のNagi target受入と実ツール接続の未実施を明記しています。目的・成果・Scope・権限に差分はなく、現行の根拠リンクの存在検査は通りました。全スイートと `NOT STARTED` に戻す一時コピーのテストはrunner報告に基づきます。

NDP-REVIEW: FIX

## Round 6 — codex review of 0e972c40dd70

**P2 — Stage 3受入の完了を記載してもテストが通る**  
[test_task_doc_status.py:203](tests/developer-platform/evaluation/test_task_doc_status.py:203)

判定は同じ行に `Stage 3`、`Nagi target`、`受入`、`未実施` があることを確認します。実績欄を「Stage 3（Nagi target）の受入は完了。実機での追加確認は未実施。」に置き換えると、別の作業にかかる「未実施」で条件を満たします。この変更をメモリ上で適用し、文書検査6件がすべて通ることを確認しました。

最小修正は、`Stage 3（Nagi target）の受入は未実施。` を箇条書きの先頭から一文として検査し、上記の置換で失敗する回帰ケースを既存のテストに加えることです。

指定差分では、task文書の変更は状態行と実績・根拠の追記に収まっています。現行文書には `PARTIAL`、host限定、委任に基づく未承認の `PROVISIONAL` 判断、およびStage 3受入が未実施であることが記載されています。

NDP-REVIEW: FIX

## Round 7 — claude-opus review of a66b26537a5d

差分の確認と新規テストの実行が終わったので、結論をまとめます。

## NDP-09t レビュー結果

**判定: P1・P2の指摘はありません。**

### 目標と受入条件の確認
- **状態行:** `NOT STARTED` を `PARTIAL（host限定、PROVISIONAL…DONEではない）` に変えています（`docs/development/co-developer-starter-kit/tasks/NDP-09_VALIDATION_ORCHESTRATOR.md:3`）。文書の中に `NOT STARTED` は残っていません。
- **変更しない節:** task文書の差分は、状態行と末尾の新しい「実績・根拠」節（33行〜）だけです。目的・編集範囲案・実装・focused test案などの節は差分に含まれていません。
- **根拠ファイル:** 挙げられた根拠ファイルはすべて実在します。`stages.json`、`validation/README.md`、`stage2.py`、`generated_check.py`、`validate.py`、`report.py`、各 `test_*.py`、`handover.md`、`board.md` を確認しました。
- **記載内容と実装の一致:** 次の3点を照合し、事実と合っていました。
  - `stages.json` で Stage 3 は `"connected": false`
  - `stage2.py` は固定引数のclangを使い、承認済みfixtureをSHA256で照合している
  - `report.py` に `--fixture all` がある
- **PROVISIONALの記録:** 実績欄に「委任に基づくPROVISIONALで、承認済みではない」と書かれています。評価READMEの `### 仮定（PROVISIONAL）` 節にも、判断と理由が記録されています。
- **テスト（`tests/developer-platform/evaluation/test_task_doc_status.py`）:** 新規テストを手元で実行し、33件すべて成功しました。
  - 状態の値が `PARTIAL` と完全に一致すること、`DONE`・`PASS` でないことを検査しています。
  - リンク先が実在し、リポジトリの外に出ないこと（`..` とsymlinkを解決した後）を検査しています。
  - Stage 3・target受入と、実 `nagi-pkg`・PT-01 scanner について、「未実施」を1行の中で述べているかを検査しています。
  - 一時コピーで状態行を `NOT STARTED` に戻すと失敗することを、`test_status_reverted_to_not_started_fails` で確認しています。
  - 検査は実際の文書テキストに対して行われ、決め打ちやMockだけの検査ではありません。
- **安全面:** 担当path外の変更はありません（`reviews/ndp-runner-ndp-09t.md` は既定の運用なので除外）。外部の取得や実行、`contracts/` への変更もありません。一時コピーは `tempfile` の中で作られ、コピー元とコピー先の両方が範囲内に収まるか確認しています。
- **host成功とtarget成功の区別:** 混同はありません。実績欄に「host限定でありNagi target受入ではない」と明記されています。

### P3（軽微）
1. **README追記の位置:** `tests/developer-platform/evaluation/README.md:253`
   - **条件:** NDP-09tの記録が、NDP-11d節（207行〜）の下の `### 仮定（PROVISIONAL）` に入っています。将来この節を探す人が、NDP-11dの仮定と取り違えるおそれがあります。
   - **最小修正案:** 独立した `## NDP-09 task文書の状態 — NDP-09t` 節を作り、その下にPROVISIONAL小節を置く。必要なのは移動だけです。
2. **テストの規模:** `test_task_doc_status.py` は456行あり、文書を検査するテストとしては大きめです。6ラウンド分の言い回しによるすり抜けへの対策が積み重なった結果です。すり抜けを完全には防げないことは、README（263〜268行）に限界として記載済みです。今後この方向での追加の強化はせず、現状のままで構いません。

### 疑問
ありません。

NDP-REVIEW: PASS

