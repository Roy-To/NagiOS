# PT-01 report adapter（暫定）

`pt01_report.py` はPR #91 report v1の限定projection。PT-01のscannerや公開schemaを
変更・再定義しない。Python 3.9標準ライブラリのみ。report内のpathを開かず、
ソース走査・実行・外部通信・ファイル書込みを行わない。

`adapt_report_v1(data: bytes, *, allowed_paths)` はUTF-8 JSONと、呼出側が明示選択した
project相対pathの完全一致allowlistを受け、変更不可の`Finding` tupleを返す。
`read_report(binary_stream)` は上限+1 byteまでしか読まない。
`ReportError` の本文は入力を反映しない固定error code。

## PR #91実装との静的照合（NDP-04d、2026-11-18、PROVISIONAL）

`origin/codex/0.2-developer-tooling:tools/nagi-cli/src/porting.rs`（636行）を
読み取りのみで確認し、出力schemaの事実だけを[`pr91_schema.json`](pr91_schema.json)
（データ）に記録した。scannerのコードはコピーせず、nagi-cliはbuild・実行していない。
実scannerの出力ではなく、ソースの出力規則から手で組んだfixtureによる適合検査である。
検査は`pt01_conformance.py`（`check_report`、偏差は固定code）と
`adapt_report_v1(..., strict=True)`。既定はこれまで通りの寛容なprojection。

| 項目 | PR #91の実装 | adapter | 結果 |
|---|---|---|---|
| root / `schema_version` | object / 整数1 | 同じ | 一致 |
| root追加項目 | `detected_languages`(`c`/`c++`/`rust`)、`manifests`、`dependencies`、`analysis`、`limitations` | 既定は無視。strictで型・enumを検査 | 検査追加 |
| findingの項目 | `category/path/line/evidence/review_status` 5項目のみ | 同じ | 一致 |
| `category` | 7値固定: `filesystem`/`process`/`threading`/`networking`/`dynamic-loading`/`gui-framework`/`os-contract` | 識別子文字列なら何でも可 | **差異**: 旧fixtureの`windows-api`等はscannerが出さない。strictで不一致。Windows側`windows_categories`は実在しない値（下記） |
| `review_status` | 常に`requires-review` | 識別子文字列なら可 | **差異**: 旧fixtureの`needs_review`/`pending`は出ない。strictで不一致 |
| `line` | 1起点の整数。nullなし | 1以上の整数のみ可 | 一致。null/0/文字列/真偽値/小数は両モードで拒否 |
| `evidence` | 常に`matched tokens: ` + 固定token語彙を`, `連結。**ソース断片ではない** | 非空文字列 | **重大な差異**（下記）。strictでprefixとcategory別tokenを検査 |
| `path` | project相対、`/`区切り、非UTF-8は欠損置換 | 安全な相対pathのみ | adapterが厳しい（意図的）。`:` `\\` `%` 先頭`~`や制御文字を含むPOSIXファイル名は`invalid_relative_path`で拒否（fail closed） |
| 並び・重複 | `(path, line, category, evidence)`順、完全重複は除去 | 順序不問 | strictで順序と重複を検査 |
| finding上限 | 10,000件（超過で`analysis.truncated`と`limitations`） | 旧4,096 | **差異を修正**: 実scannerが出せる件数を拒否していた。10,000に、byte上限を8 MiB、値数を200,000に変更 |
| `analysis` | 整数4項目、`truncated`真偽、`conclusion`(`requires-review`/`no-known-boundary-detected`) | 旧: 未検査 | strictで型・enum・整合（truncated/findings/dependenciesありなら`requires-review`）を検査 |
| `limitations` | scan固有文＋固定3文（Nagi互換/POSIX・ABI互換を確立しない、等） | 旧: 未検査 | strictで固定3文の存在を検査 |
| `dependencies[]` | `name/source/review_status` | 旧: 未検査 | strictで検査（利用はしない） |

### 残る差分・要対応

- **evidenceは周辺ソースを含まない。** scannerはlexerなしの行単位・大文字小文字無視の
  部分一致で、コメントや文字列中の出現も報告する。このため診断側のコメント・文字列除外は
  実reportでは作用せず、`no_code_evidence`にもならない（evidenceは常にtoken語彙）。
  コメント／文字列だけの出現かを後から判別するにはPT-01側の拡張（source位置の根拠）が必要。
- **Windows rulesの到達性。** 実scannerのtoken語彙のうちWindows rulesに一致するのは
  `CreateProcess`（今回`WIN-PROCESS`のaliasへ追加）と`CreateThread`のみ。
  `WSAStartup`/`LoadLibrary(`/`_WIN32`はruleがなく`not_classified`になる。
  CreateFileW/ReadFile/レジストリ/UI/CRT/.NETは実scannerが検出しないため、実reportからは
  rulesへ到達しない（既存の`read_prefix`等は合成入力の試験）。
  scannerのcategoryにはWindows専用区分がなく、`windows_categories`は実在しない値のまま
  （`unknown_api`は実reportでは出ない）。token→ruleの追加は別task（担当path内で可能だが範囲外）。
- **truncatedの伝播。** `completeness(data)`で`analysis.truncated`は取得できるが、
  windows/linuxの診断出力にはまだ含めていない（出力形式を変えるため別判断）。
  truncatedなreportではfindingの不在は未使用を意味しない。
- 旧`windows/`・`linux/`のfixtureはPR #91が出力し得ない形。入力処理とlexerの試験用として残し、
  適合fixtureは[tests/…/api/common](../../../../tests/developer-platform/api/common/README.md)。
- 正規PT-01契約が確定したら`pr91_schema.json`（とstrict検査）を差し替える。

## 仮定（PROVISIONAL）

- 接続根拠はユーザーが今回指定したPR #91
  `codex/0.2-developer-tooling:tools/nagi-cli/src/porting.rs` と、worktree内の
  [開始記録](../../../../docs/development/co-developer-starter-kit/START_AUTHORIZATION.md)
  にある過去の読取結果。今回は上記の通り実装を静的に読んで照合したが、実scannerは
  build・実行していないため、実出力との直接接続試験はできていない。
  正規PT-01入力は未確定。fixtureはsource規則から手で組んだものである。
- rootはobject、`schema_version` は整数`1`、`findings` はarrayと仮定する。
  findingは`category/path/line/evidence/review_status`の5項目を必須とする。
  categoryとreview_statusは1〜128文字のASCII識別子（英数と`_.:-`）、
  pathは安全なproject相対文字列、lineは1起点の整数、evidenceは非空文字列と仮定。
  nullable line、evidence object、自由文のreview_statusは未接続として拒否する。
  正規契約確定時はこのadapterだけを置換できる。enum値の確定は行わない。
- `detected_languages/manifests/dependencies/analysis/limitations`等の追加項目は
  JSON全体の上限検査後に無視する。これらの型・意味を検証したとはしない。
  full report validatorの代用ではない。重複keyや非有限数は全項目で拒否する。
- 内部処理上限は8 MiB、10,000 findings（PR #91のfinding上限に合わせた）、
  evidence 4,096文字、構造深度32、値200,000個。pathは1,024文字、lineは最大2,147,483,647。
  巨大整数、壊れたUTF-8、孤立surrogate、制御文字、不正pathを拒否する。
  製品の資源要件を確定する数字ではない。
- allowlistは呼出側の選択範囲そのもの。reportの全findingがその範囲内である必要がある。
  pathは`/`区切りで、絶対path、drive/UNC、空/`.`/`..`要素、`%`、`:`、`\\`を拒否する。
  filesystemを参照しないため、symlink解決・実ファイル存在の確認は行わない。
  後続工程が実ファイルを読む場合は、そのownerが改めて範囲を検証する。

Windows側の使い方と診断限界は[担当README](../windows/README.md)を参照。
