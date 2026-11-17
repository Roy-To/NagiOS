# NDP-04 — Windows由来API差分rules

Win32、Microsoft CRT差分、.NETの代表例を`rules.json`の14ルールで分類する、非AIの
host診断。入力は[暫定adapter](../common/README.md)を通したPT-01 report v1。
filesystem scanner、互換レイヤー、コード変換、元アプリのbuild/runは含まない。

## 実行

repository rootで、明示した相対pathのfindingsだけを受け付ける:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/api/windows/diagnose.py \
  --path read_prefix.c \
  < tests/developer-platform/api/windows/read_prefix.report-v1.json

PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-developer-platform/api -p 'test_*.py'
```

CLIはJSONをstdoutへ出す。入力拒否は終了値2、stderrに固定codeのみ。
Python APIは`diagnose_report(data: bytes, *, allowed_paths, adapter=..., rules=None)`。
adapterとrules引数は信頼する呼出側の差し替え点であり、reportから指定できない。
`rules=None`は同梱JSONのみを読む。出力はpath・line・rule id・内容順で安定し、
重複findingは保持する。時刻・環境依存pathを付けない。

## 結果の読み方

- `lexical_candidate`: 根拠断片の識別子にruleが一致した候補。
  実呼出し、型解決、対象OS依存、互換性を認定しない。
- `unknown_api`: Windows用と仮定したcategoryだが一致ruleがない。
  未対応APIの名前を推測せず、元reportの位置と根拠hashを残す。
- `not_classified`: それ以外のcategoryで一致ruleがない。
- `no_code_evidence`: コメント・文字列等の除外後に識別子の根拠がない。
  「ソースにAPIがない」という判定ではない。

各rowは元のcategory/path/line/review_status、evidenceのSHA-256、
一致したAPI alias、分類、理由、実在SDK/契約への参照、代替候補、残る不足を持つ。
根拠断片の本文や無関係なreportメタデータは出力しない。秘密や絶対pathを含む
コメント等を再掲せず、元reportの位置とhashで照合する。未知名もそのまま反映しない。
呼出側が選択した相対pathは出力に残るので、共有時の機密確認は呼出側の責務。

`availability=unsupported`は、この公開SDKに直接の代替候補を確認できないことを示す。
Nagi OS全体の内部サービスの不存在を断定しない。`contract_only`は型・設計の
再利用候補に限り、`unknown`は対応が未評価。`approval=PROVISIONAL`、
`compatibility=not_assessed`、`target_acceptance=not_run`は常に維持する。
入力の`review_status=approved`等によって互換認定や権限付与へ昇格しない。

| APIの代表例 | 診断・現行契約の候補 |
|---|---|
| CreateFileW / ReadFile / WriteFile / CloseHandle | ファイルI/O・寿命。公開SDKに一般file APIがなく代替未提供 |
| RegOpenKeyExW等 | AppStateBackendをapp限定の状態保存へ再設計する候補。registry互換・実storage backendはない |
| CreateThread / CreateProcessW等 | runtime adapterと認可が必要。SDKに直接代替を確認できない |
| _wfopen / fopen_s / _stricmp / _snprintf等 | encoding、sharing、locale、truncation、constraint handler等の個別差分を要調査 |
| System.IO.File.ReadAllText / Process.Start等 | managed runtime依存。公開SDKに直接代替を確認できない |
| CreateWindowExW / System.Windows.Window | Application::presentationは論理identityの参考だけ。実window・WPF・描画は未接続、PT-04へ |

## 仮定（PROVISIONAL）

- PT-01正規契約は未確定。今回の入力はPR #91 report v1の手書きfixtureであり、
  本物のscannerからの接続成功とはしない。line・evidence等の仮定と上限は
  [共通README](../common/README.md)に記録した。共通adapterは正本schemaではない。
- `windows-api/windows-header/win32/windows-crt/dotnet`を未知候補のcategoryとして
  暫定使用する。実際のenum値の確認は残る。既知aliasはcategoryにかかわらず照合する。
  category集合・alias・理由・契約参照・不足はJSONで差し替える。
- reportのevidenceがソース断片なら、C/C++/C#風コメント、通常文字列・文字literal、
  raw/verbatim/interpolated文字列を保守的に除外する。C/C++の行継続も除去する。
  interpolation内の式は実コードであってもopaqueとするため検出を逃すことがある。
  lexerは完全な言語parserではない。断片前のblock comment状態、複雑な構文、macro展開、
  conditional compilation、alias解決、実呼出しは不明。途中で切れた断片は要手動確認。
- symbolだけのevidenceでは元がコメントか文字列かを判別できない。候補と限界を残し、
  ソースを読み足して判定済みにしない。同名の変数やlocal typeも候補になり得る。
  短縮.NET名は特に要symbol解決。.NET利用自体をWindows専用と断定しない。
- 代替参照は現在の`sdk/README.md`、C header、Rust SDKとApp Contract v1を読み取った
  内部対応表。AppStateBackendは保存処理のinterface、Application::presentationは
  論理データだけであり、実transport・window作成・権限を提供しない。
  SDK変更時はJSONの候補と不足を再点検する。新しいSDK APIや権限を追加しない。
- 代表例の限定coverageであり、全Win32／CRT／.NETを対象とするsupport matrixではない。
  正規PT-01 ownerによるadapter差し替え、SDK ownerによる対応方針、実アプリ／targetの
  受入は後続工程。今回の範囲の実装・host試験はこれらの確認待ちにしない。

## 検証結果

runner指定コマンドで30テスト成功（host、Python 3.9）。既存`read_prefix.c`と
negative controlの手書きreportを元入力の各行に照合し、本体rulesとCLIで診断した。
全aliasと実在参照、CRT/.NET、未知API、同名識別子、コメント・各種文字列、
壊れた／巨大／深いreport、相対path・範囲拒否、決定性、入力不変性を確認。
SDK・共有CLI・board・implementation_statusは担当path外のため変更していない。
Nagi target、実Windows/.NET runtime、正式PT-01 scannerは試験していない。
