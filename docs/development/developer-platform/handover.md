# NDP統合branchの引渡し記録（NDP-12b）

状態: 2026-11-18時点の事実の整理。製品判断・承認・実装許可・release判断を記録するものではない。
読む人: 創設者、Integration Owner。根拠は `git log`、各README、`reviews/`、[board](board.md)、[接続調査](contract-audit.md)。
host（macOS arm64、Python 3.9.6）での結果だけで、Nagi targetでは何も実行していない。
この記録の作成時はテスト実行ができなかったため、下記の件数はREADMEやrunner記録からの転記であり、今回再実行していない。

## 1. 統合済みの成果

merge先は `codex/ndp-host-integration`（`git log` のmerge message）。main、Linux移行統合branch、正式releaseへの反映はこのlogからは確認できない。
board上の「上流統合なし」は11-17時点の記述で、下の表とはboardの更新順が違う。

| Task | PR / commit | 主なpath | 記録された結果 |
|---|---|---|---|
| NDP-06b | #165（1c8800b） | `tools/nagi-developer-platform/ui/analysis/`、`tests/developer-platform/evaluation/ui/*.ui` | GTK Builderの静的解析。claudeレビューPASS |
| NDP-11b | #167（c22ad05） | `tests/developer-platform/evaluation/run_host_checks.py` | GTK 3/4を繰返し評価へ追加。runnerのfailure-pathテスト14件。codexレビューPASS |
| NDP-07 | #168（8d4ef66） | `ui/mapping/`（`rules.json`、`contract_snapshot.json`、`map_ui.py`） | UI対応表と未対応レポート。claudeレビューPASS |
| NDP-04 | #169（560258f） | `api/windows/`、`api/common/` | Windows API差分rulesと暫定PT-01 adapter。hostテスト30件。claudeレビューPASS |
| NDP-02 | #170（cc3963c） | `generator/contract/`（`level0.json`、`contract.py`） | Level 0雛形契約と実ファイル照合テスト。claudeレビューPASS |
| NDP-05 | #171（5f7cd16） | `api/linux/` | Linux API差分rules。review 3回目でcodex PASS。最終は66件（記録上） |
| NDP-03 | #172（a2c7aa2） | `generator/generate.py`、`test_generate.py` | 雛形generator。review round 3でP1が2件残り、round 4でclaudeがP1/P2なしと判断。41テスト、実validator依存の1件はskip |
| NDP-09s2 | #173（64dec49） | `validation/stage2.py`、`stages.json` | 自作portable C fixture限定のStage 2（固定clang引数）。Stage 3は常に `not_run`。claudeレビューPASS |
| NDP-06c | #174（03b6c74） | `ui/analysis/`、`ui/mapping/` | 6bのP3残件。analysis 45件・mapping 26件（runner記録）。claudeレビューPASS |
| NDP-04c | #175（1c23a63） | `api/windows/` | `windows.h` の大文字小文字・引用符include。68件（runner記録）。claudeレビューPASS |
| NDP-09c | #176（1ead52c） | `validation/generated_check.py` | 生成雛形のManifestをStage 1へ接続。claudeレビューPASS |
| NDP-11c | #177（3cb5715） | `run_host_checks.py` | API rules・generator・Stage 2を各3回の繰返し評価へ追加。runner側40件。claudeレビューPASS |

PR番号が付く前の統合分（boardとgit logより）: NDP-01開始記録（e8071ec）、NDP-06（9b68295）、NDP-10（715d308）、NDP-11初版（e49fb71）、NDP-02/08/09のStage 1実validator接続（5108218）。
PR番号の振られていない分の詳細は `git show` で確認する。

## 2. 各READMEの「仮定（PROVISIONAL）」と差し替え箇所

どれもCONFIRMEDではなく、owner合意は取得していない。差し替えは該当dataまたは1関数に閉じている。

| README | 仮定の要点 | 差し替え箇所 |
|---|---|---|
| [api/common](../../../tools/nagi-developer-platform/api/common/README.md) | PT-01 report v1（PR #91由来）の手書きfixtureによる仮接続。必須5項目、上限（1 MiB、4,096 findings等）は仮置き | `pt01_report.py`のadapter（`adapter=`引数） |
| [api/windows](../../../tools/nagi-developer-platform/api/windows/README.md) | category名、header大文字小文字、代替参照の対応表、代表例のみのcoverage | `rules.json`。`windows.h` だけを `case_insensitive` にしている箇所 |
| [api/linux](../../../tools/nagi-developer-platform/api/linux/README.md) | category名、関数名照合の保守的規則、CMake等のbuild依存は対象外 | `rules.json`（`headers`／`header_prefixes`／`calls`／`identifiers`） |
| [generator/contract](../../../tools/nagi-developer-platform/generator/contract/README.md) | SDK参照はrepo内path依存。revisionはpinしない | `level0.json.sdkReference`、`contract.py::sdk_dependency` |
| [generator](../../../tools/nagi-developer-platform/generator/README.md) | 親directoryを自動作成しない。保護対象3項目。失敗時に何も削除しない。並行競合は脅威モデル外 | `generate._preflight`、`generate.PROTECTED_TREES`、`generate.generate`の失敗処理 |
| [validation](../../../tools/nagi-developer-platform/validation/README.md) | Stage 2はself-made C fixtureのみ。clangはPATH先頭、flagは `-std=c11 -Wall -Wextra -Werror -O0`。生成物接続の既定入力 | `stages.json`、`generated_check.py`の `DEFAULT_INPUTS`、`MANIFEST_NAME`、`check_generated`のstand-in |
| [ui/analysis](../../../tools/nagi-developer-platform/ui/analysis/README.md) | `nested_in_unknown` は内部項目で公開schemaではない。`template` は未知タグのまま | `analyze.py`の該当1か所 |
| [ui/mapping](../../../tools/nagi-developer-platform/ui/mapping/README.md) | UI ownerの承認前の内部照合。snapshotの列挙範囲（`user/nagi-ui/src` の `.rs`）。GtkStack等は未対応 | `rules.json`、`contract_snapshot.json` |
| [evaluation](../../../tests/developer-platform/evaluation/README.md) | 期待値表（`API_CASES` 等）は現行のLevel 0とrulesに合わせた。Stage 2のtimeoutは90秒 | 該当のdata表のみ |

## 3. 未検証事項

- **Nagi target**: Nagi image、VM、QEMU上では何も実行していない。host結果はtarget受入ではない。
  Stage 3は未接続で、常に `not_run`。
- **標準host要件**: Python 3.9.6で実行しており、標準の3.10+は満たしていない。
  標準host全体の `./nagi doctor` は未実施。
- **実nagi-pkg**: 11-17には固定nightlyで一度buildし受理を確認した記録がある。
  ただし作成時点のこのworktreeでは未buildのため、実validatorを使うテストはskip。
  skip条件を持つテストは `validation/test_validate.py`、`test_generated_check.py`、`generator/test_generate.py` にある
  （validation READMEは41件中2件skipと記載、generator READMEと09c reviewは実validator依存テストのskipに言及）。skip件数の合計は今回数えていない。
  skipは合格ではない。実validatorによる雛形のManifest受理は、構築済み環境で別に確認が必要。
- **生成物**: 雛形のcrate build・実行・署名・install・起動は未実施。
- **API診断の入力**: NDP-04/05はPT-01の手書きfixtureで検証しており、実scanner出力との接続は未試験
  （PR #91の実装はこのcheckoutに無い）。
- **UI**: renderer、実event、IME、a11y service、GTK/Qtの実instance化は未実施。
- **評価**: 手動baseline、ローカルLLM補助、外部開発AIとの比較、能動作業時間、速度比は未実施・未測定。
  実測は1台・1セッションのみで、速度比や移植成功は主張できない。
- **DeepSeek**: 今回の記録にDeepSeekの実送信は無い。
  NDP-06/10/11初版は[レビュー記録](../../../reviews/ndp-host-pilots.md)に「方式Aの追加条件は非該当と独立担当が判断、実送信なし」とある。
  runnerのレビュー（上表のPR分）はclaudeまたはcodexで、DeepSeek実施の記載は無い。
  追加条件に該当するかの再判断は、各PRについて行っていない。
- **競合・TOCTOU**: generator、stage2、run_host_checksの一時領域を同時に別processが操作する競合は範囲外。
  generatorは作成物を削除せず残置して報告する設計。
- **残るP3**（担当pathや軽微さのため未対応と記録されたもの）:
  `<sub/windows.h>` が `windows.h` に一致し得る点、`stage2.py` の `prior` は呼出側の自己申告であること、
  Stage 2が診断出力を返さない点、`generate.py` の `0o700` 厳密判定がumaskで誤判定し得る点。

## 4. 他ownerへの接続依頼

どれも依頼であり、承認や合意が得られたことを示すものではない。

- **PT-01 owner**: 正規のreport入力（schema、enum値、line/evidenceの扱い）を教えてほしい。
  確定後は `api/common/pt01_report.py` を差し替える。実scannerの出力で一度試験したい。
- **SDK owner**: 外部app向けのSDK配送・pin方式（現在はrepo内path依存の仮定）と、最小Manifestの外部参照の可否。
  `sdk/rust/Cargo.toml` の存在確認をstand-inにしている箇所の扱い。
- **UI owner**: `ui/mapping/rules.json` の部品対応・layout再設計の確認、`contract_snapshot.json` の更新手順、
  row `status` と `mapping.status` の二重表現のどちらを残すか。
- **CLI / Package owner**: `nagi-pkg manifest validate` をStage 1が呼ぶ使い方の確認、
  `./nagi` 経由での統合方法、Stage 2/3の起動条件。
- **共有path（Registry、CI、root workspace、Cargo）**: 本担当では変更していない。
  `tools/nagi-developer-platform/` を共有側へ登録するか、CIで実validator buildを走らせるかは、指定ownerの判断待ち。
- **Integration Owner**: `codex/ndp-host-integration` からmain等への反映方法と順序。
  boardのTask表（NDP-03/04/05/07/09等が古い状態）は調整担当が更新する。この記録では変更していない。
  `reviews/ndp-runner-ndp-05.md` の本文にローカル依存のリンク（7行・11行付近）が残っている。担当path外のため触れていない。

## 5. 最小の引渡し

```text
成果: NDP-02〜09の限定host実装とNDP-11の繰返し評価（PR #165, #167〜#177、merge先 codex/ndp-host-integration）
確認: 各READMEとrunner記録に件数あり。いずれもhost-only。この文書作成時の再実行なし
接続: PT-01の正規入力、SDK配送方式、UI対応表、CLI/Package呼出し。未接続点は第3・第4節
次: 実nagi-pkg構築済み環境でskipされる実validator依存テストを実行。Python 3.10+ hostで再確認。ownerの回答に応じたdata/adapter差し替え
```
