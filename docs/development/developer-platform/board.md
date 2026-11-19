# NDP作業ボード

2026-11-18更新。専用worktreeは利用者が作成を許可。共有checkoutのbranchを切り替えず、専用作業場所で1セッションが書き込む。push/上流merge/共有registry変更は未実施。

| Task | 状態 | 結果 | 次・残件 |
|---|---|---|---|
| NDP-01 | PASS | e8071ecの開始記録 | 担当確定済み |
| NDP-02 | PARTIAL | Level 0雛形契約を実ファイルで照合 | SDK外部参照方式とowner合意 |
| NDP-03 | PARTIAL | 雛形generator、41件OK（実validator依存1件skip） | 雛形のbuild・署名・install未実施 |
| NDP-04 | PARTIAL | Windows API差分rulesと暫定adapter、68件OK | PT-01正規入力と実scanner接続 |
| NDP-05 | PARTIAL | Linux API差分rules、66件OK | PT-01正規入力と実scanner接続 |
| NDP-06 | PASS | Qt XMLとGTK Builderの静的解析、host限定 | renderer・実event・実instance化は未実施 |
| NDP-07 | PARTIAL | UI対応表と未対応レポート、26件OK | UI ownerの部品対応の確認 |
| NDP-08 | PARTIAL | 正規validatorを実接続 | CLI/Package owner確認 |
| NDP-09 | PARTIAL | Stage 1/2と生成物接続、Stage 3は未接続 | Stage 3と実nagi-pkg環境での確認 |
| NDP-10 | PASS | 715d308、独立レビュー済み | 後続の比較評価 |
| NDP-11 | PARTIAL | API・generator・Stage 2を各3回評価 | 手動・LLM比較とtarget未実施 |
| NDP-12 | PARTIAL | 統合済み成果と未検証事項をhandoverに記録 | main等への反映順序をIntegration Ownerへ |

SDKとtoolの読取結果・具体的な不足は[接続調査](contract-audit.md)。引渡しは[handover](handover.md)。PASSは上記の限定作業のみで、Nagi image受入ではない。

確認結果: portable Cの警告エラー付きbuild・境界条件・繰返し一致・入力不変が成功。NDP-06は実際のNDP-10入力とCLIを含む18テストが成功。標準host全体のdoctor、SDK validator、VM/targetは未実施。

Codexレビュー: 初回P2を修正し、9b68295の独立再レビューで未解決P1/P2なし。詳細は[レビュー記録](../../../reviews/ndp-host-pilots.md)。

DeepSeek: 方式Aの追加条件は非該当と独立担当が判断。重要なOS権限等の変更や未解決・見解不一致はなく、今回の追加送信は不要。実送信はしていない。

2026-11-17: `python3 tests/developer-platform/evaluation/run_host_checks.py` 成功。
Cを実build/run、UI CLIを各3回実行。UI中央値約34ms（起動込み、速度比ではない）。
ランナー失敗検知7件と既存UI 18件が成功。詳細・再実行は[評価README](../../../tests/developer-platform/evaluation/README.md)。

独立レビューのP2（同じ誤解析でも一致成功）を修正。部品・tab順・接続を照合し、旧版で5ケース失敗を確認。

NDP-11独立再レビュー: e49fb71でP2解消、未解決P1/P2なし。DeepSeek追加不要。

NDP-02/08/09: 固定nightlyでnagi-pkgをbuild、最小第三者Manifestを受理。
Stage 1 adapter 11件・正規tool 7件成功。Stage 2/3は未実行を返す。

## 自動runner記録

- 11-17 NDP-06b: GTK Builder静的解析と安全性・回帰テストを追加（PR #165、runnerテスト成功、claudeレビューPASS）
- 11-17 NDP-11b: GTK 3/4 を run_host_checks.py の繰返し評価と failure-path テスト（14 件）に（PR #167、runnerテスト成功、codexレビューPASS）
- 11-17 NDP-07: UI対応表・未対応レポートと実fixtureテストを実装（PR #168、runnerテスト成功、claudeレビューPASS）
- 11-17 NDP-04: Windows API差分rulesと暫定adapterを実装、hostテスト30件成功（PR #169、runnerテスト成功、claudeレビューPASS）
- 11-17 NDP-02: Level 0雛形契約と実ファイル照合テストを実装（PR #170、runnerテスト成功、claudeレビューPASS）
- 11-17 NDP-05: Linux API差分rulesを実装（公開前チェックの誤検出を修正して再公開）（PR #171、runnerテスト成功、codexレビューPASS）
- 11-18 NDP-03: 並行競合を脅威モデル外としてREADMEに限界明記、41テストOK（実validator未準備の1件はskip）（PR #172、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-09s2: Stage契約(stages.json/README)と自作fixture限定のStage 2(固定clang引数・拒否（PR #173、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-06c: NDP-06cのP3残件を実装済みで、analysis 45件・mapping 26件のunittestがすべてOK（PR #174、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-04c: NDP-05 の P2 は解消済みを既存テストで確認、NDP-04 の header 大文字小文字・引用符 includ（PR #175、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-09c: 生成雛形のmanifestをStage 1へ接続（generated_check.pyとテスト追加、実nagi-pkg検（PR #176、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-11c: API rules・generator・Stage 2を繰返し評価に追加（各3回一致・入力不変・失敗検知）、runner（PR #177、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-12b: 統合済み成果・PROVISIONAL一覧・未検証事項・他owner依頼を handover.md に記録（テストは Ba（PR #178、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-A001: `WIN-HEADER` は path 付き include（`<sub/windows.h>` など）に一致しなくなり（PR #179、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-A002: generate.pyのdirectory mode判定をumask連動に修正（受入後にfchmod 0o700、0o4（PR #180、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-A003: Stage 2 に伏字化・上限付き diagnostics 要約を追加し、実 clang テスト 67 件が成功（スキッ（PR #181、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-A004: UI mapping の contract 列挙で symlink を fail-closed 拒否（5回帰テストとRE（PR #182、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-12c: boardのTask表を統合branchの実状態に更新し、レビュー記録のローカルパスリンクをrepo相対に修正（テストは（PR #183、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-04d: PR #91 scannerのreport schemaとadapter/fixtureを静的照合し、差異（catego（PR #184、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-06d: XAMLを3つ目のUI形式として解析・NDP-07 mapping・fixture・focused test・READM（PR #185、runnerテスト成功、claudeレビューPASS）
- 11-18 NDP-11d: 実nagi-pkgによる雛形Manifest受理（未準備時はnot_run）とXAML 3件の繰返し評価をrunnerに（PR #186、runnerテスト成功、claudeレビューPASS）
- 11-19 NDP-A005: 未知namespace要素のcustom扱い・x:Key・DataTemplate同名x:Nameの現状をテスト固定し、（PR #187、runnerテスト成功、claudeレビューPASS）
- 11-19 NDP-A006: evaluation README から旧「40 checks」を除去し、README 件数（65）と収集テスト数の一致（PR #188、runnerテスト成功、claudeレビューPASS）
- 11-19 NDP-A007: 読めないディレクトリの列挙挙動を実測し、テストとREADMEに反映（PR #189、runnerテスト成功、codexレビューPASS）
- 11-19 NDP-09r: Stage 2 の承認済みスナップショット照合を追加し、指定テスト78件が成功（PR #190、runnerテスト成功、codexレビューPASS）
- 11-19 NDP-08b: 利用表12行を更新し、根拠検査5件が成功（PR #191、runnerテスト成功、claudeレビューPASS）
- 11-19 NDP-A008: 移植レポートに overview（件数と非passed一覧）と reason 説明表付きの Markdown 冒頭節を追（PR #192、runnerテスト成功、claudeレビューPASS）
- 11-19 NDP-A009: `report.py --fixture all`（JSON の summary と Markdown 冒頭の行列表）を（PR #193、runnerテスト成功、claudeレビューPASS）
- 11-19 NDP-A010: 4 fixture の全 reason を表か許容リストで網羅するテストを追加し、compiled/test_passe（PR #194、runnerテスト成功、claudeレビューPASS）
- 11-19 NDP-A011: UI mapping未対応行を mapping.status で絞り、深刻な順に上限付きで JSON と Markdow（PR #195、runnerテスト成功、claudeレビューPASS）
- 11-19 NDP-A012: 移植レポートMarkdownに API gaps 節を追加（unsupported/未解決行を行番号・API名付きで表示（PR #196、runnerテスト成功、claudeレビューPASS）
- 11-19 NDP-A013: 移植レポートの host_build に Stage 2 診断項目（file/line/severity/categor（PR #197、runnerテスト成功、codexレビューPASS）
- 11-19 NDP-A014: 移植レポートの generation check に上限付き相対名一覧 files と Markdown の Scaff（PR #198、runnerテスト成功、codexレビューPASS）
- 11-19 NDP-A015: レビュー P1(read_manifest 経由のコピーと非変更確認)と P2(API 名照合だけを検証するテスト)を修（PR #199、runnerテスト成功、claudeレビューPASS）
- 11-19 NDP-A016: ui_mapping の各ファイル結果に軸別件数 axis_gaps を追加し、Markdown に軸別の未対応・要判断（PR #200、runnerテスト成功、claudeレビューPASS）
- 11-19 NDP-A017: 未解決行を実在の未収録 API（CancelIoEx / euidaccess）に置き換えて snapshot とテスト（PR #201、runnerテスト成功、codexレビューPASS）
- 11-19 NDP-A018: `report.py --fixture app_windows`（API+WPF XAML の自作 fixture）を（PR #203、runnerテスト成功、codexレビューPASS）
