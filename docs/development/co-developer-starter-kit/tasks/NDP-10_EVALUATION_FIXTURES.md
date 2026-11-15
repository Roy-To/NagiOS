# NDP-10 — 評価fixtureの選定・権利・作成準備

状態: 開始承認済み（2026-11-15、[開始記録](../START_AUTHORIZATION.md)）/ NOT STARTED。PT-06。NDP-01と、fixture作成path・host実行範囲の承認が必要。

## 目的

PT-02〜05を試せる小さな入力と期待する機能を用意する。実アプリの無断コピーや、架空の移植成功例を作らない。

## 入力

DEC-045 PT-06と `docs/porting-toolkit/PT06_MIGRATION_EVALUATION_TEMPLATE_2026-11-13.md`。既存のsample/fixtureも確認し、同じものを新しい成果として作り直さない。

## 編集範囲案

`tests/developer-platform/evaluation/**`と担当内の短い説明。Windows/Linux rulesの専用fixtureとはpathを分け、共通利用が必要なものだけ調整担当を通す。

## 作業

最初は自作/合成の小さなportable core、Windows由来APIを示す入力、Linux由来APIを示す入力、選んだ1形式のUI定義を候補にする。各fixtureに目的、対象機能、期待される診断、未検証部分、元データを実行してよい範囲を明示する。

実アプリ候補はソース・依存の利用/変更/公開条件、外部AI入力可否、対象revisionを確認してから取得・実行する。候補名を挙げただけで許諾済みとしない。ライセンスの法的判断をscannerの自動推定へ置き換えない。

## 確認項目案

- Windows/Linuxの双方を評価対象として扱い、着手順を優劣へ読み替えない。
- fixtureは小さく決定的で、networkや外部secretを要求しない。
- 元入力と生成物を別に保持し、再実行で元入力が変わらない。
- 期待値は本体の正しい仕様に基づき、実装を常にPASSさせるために書き換えない。
- 自作fixtureのhost成功を実アプリ移植実績にしない。

## 最初の成果

少数のfixtureと、対応する目的・入力・期待結果の短いREADME。PT-06の既存評価票を参照し、同じ内容を別の巨大な管理表へ複製しない。

## 次

利用可能なPT工程からNDP-11へ渡す。第三者アプリが未承認でも、承認済み自作fixtureの準備までを止めない。

共通: [規則](../03_COMMON_RULES.md)、[キュー](../05_BACKLOG_AND_PARALLEL.md)。
