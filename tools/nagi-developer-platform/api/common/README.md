# PT-01 report adapter（暫定）

`pt01_report.py` はPR #91 report v1の限定projection。PT-01のscannerや公開schemaを
変更・再定義しない。Python 3.9標準ライブラリのみ。report内のpathを開かず、
ソース走査・実行・外部通信・ファイル書込みを行わない。

`adapt_report_v1(data: bytes, *, allowed_paths)` はUTF-8 JSONと、呼出側が明示選択した
project相対pathの完全一致allowlistを受け、変更不可の`Finding` tupleを返す。
`read_report(binary_stream)` は上限+1 byteまでしか読まない。
`ReportError` の本文は入力を反映しない固定error code。

## 仮定（PROVISIONAL）

- 接続根拠はユーザーが今回指定したPR #91
  `codex/0.2-developer-tooling:tools/nagi-cli/src/porting.rs` と、worktree内の
  [開始記録](../../../../docs/development/co-developer-starter-kit/START_AUTHORIZATION.md)
  にある過去の読取結果。現在のcheckoutに同ソースはなく、必要なGit objectも
  ローカルにないため、今回その実装との直接接続試験はできていない。
  正規PT-01入力は未確定。手書きfixtureによる仮接続として扱う。
- rootはobject、`schema_version` は整数`1`、`findings` はarrayと仮定する。
  findingは`category/path/line/evidence/review_status`の5項目を必須とする。
  categoryとreview_statusは1〜128文字のASCII識別子（英数と`_.:-`）、
  pathは安全なproject相対文字列、lineは1起点の整数、evidenceは非空文字列と仮定。
  nullable line、evidence object、自由文のreview_statusは未接続として拒否する。
  正規契約確定時はこのadapterだけを置換できる。enum値の確定は行わない。
- `detected_languages/manifests/dependencies/analysis/limitations`等の追加項目は
  JSON全体の上限検査後に無視する。これらの型・意味を検証したとはしない。
  full report validatorの代用ではない。重複keyや非有限数は全項目で拒否する。
- 内部処理上限は1 MiB、4,096 findings、evidence 4,096文字、構造深度32、
  値50,000個。pathは1,024文字、lineは最大2,147,483,647。
  巨大整数、壊れたUTF-8、孤立surrogate、制御文字、不正pathを拒否する。
  製品の資源要件を確定する数字ではない。
- allowlistは呼出側の選択範囲そのもの。reportの全findingがその範囲内である必要がある。
  pathは`/`区切りで、絶対path、drive/UNC、空/`.`/`..`要素、`%`、`:`、`\\`を拒否する。
  filesystemを参照しないため、symlink解決・実ファイル存在の確認は行わない。
  後続工程が実ファイルを読む場合は、そのownerが改めて範囲を検証する。

Windows側の使い方と診断限界は[担当README](../windows/README.md)を参照。
