# NDP-05 — Linux由来API差分rules

POSIX、Linux固有（inotify/epoll等）、GTK/GLib/Qtの依存表明の代表例を`rules.json`の
19ルールで分類する、非AIのhost診断。入力は[暫定adapter](../common/README.md)を通した
PT-01 report v1。common側は変更せず再利用する。filesystem scanner、互換レイヤー、
コード変換、元アプリのbuild/runは含まない。Linux kernel採用はLinuxアプリの無変更実行を
保証しない。この診断は互換性を認定しない。

## 実行

repository rootで、明示した相対pathのfindingsだけを受け付ける:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/api/linux/diagnose.py \
  --path watch_changes.c \
  < tests/developer-platform/api/linux/watch_changes.report-v1.json

PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-developer-platform/api -p 'test_*.py'
```

CLIはJSONをstdoutへ出す。入力拒否は終了値2、stderrに固定codeのみ。
Python APIは`diagnose_report(data: bytes, *, allowed_paths, adapter=..., rules=None)`
（NDP-04と同形）。adapterとrulesは信頼する呼出側の差し替え点で、reportから指定できない。
`rules=None`は同梱JSONのみを読む。渡されたrulesは`validate_rules`で構造検査され、不正なら
`ValueError("invalid_rules")`。出力はpath・line・rule id・内容順で安定し、重複findingは
保持する。時刻・環境依存pathを付けない。

## 結果の読み方

状態名はNDP-04（Windows）と同じ意味で使い、Linux側で追加したのは`declaration_candidate`のみ。

- `lexical_candidate`: 根拠断片の識別子・header名にruleが一致した候補。
  実呼出し、型解決、対象OS依存、互換性を認定しない。
- `declaration_candidate`（Linux追加）: 一致が、直前に型らしき識別子を持つ`name(`だけ
  （例: `static ssize_t read(...)`）。ユーザー定義の同名関数の可能性が高いが、
  隠さず残す。同じ断片に呼出し形もあれば`lexical_candidate`。
- `unknown_api`: Linux用と仮定したcategoryだが一致ruleがない。未対応APIの名前を
  推測せず、元reportの位置と根拠hashを残す。
- `not_classified`: それ以外のcategory（例: windows-api）で一致ruleがない。
  Windows由来の根拠はこの診断の対象外で、NDP-04の診断へ回す。
- `no_code_evidence`: コメント・文字列等の除外後に識別子の根拠がない。
  「ソースにAPIがない」という判定ではない。

各rowは元のcategory/path/line/review_status、evidenceのSHA-256、一致したrule内のAPI名
（`linux/*`のようなprefix一致は`prefix*`）、`syntax_shapes`（`include/call/global_scope_call/
declaration_or_definition/identifier`）、分類、理由、実在ファイルへの参照、代替候補、
残る不足を持つ。根拠断片の本文や無関係なreportメタデータは出力しない。未知名も反映しない。

`availability=unsupported`は、この公開SDKに直接の代替候補を確認できないことを示す。
`contract_only`はGTK/Qtでの`Application::presentation`（論理identityのみ）に限る。
`approval=PROVISIONAL`、`compatibility=not_assessed`、`target_acceptance=not_run`は
常に維持し、入力の`review_status=approved`等で昇格しない。

| 分類（ruleの例） | 内容 |
|---|---|
| POSIX file I/O（open/read/write/close/stat等） | 整数descriptor、短縮read/write、path名前空間。公開SDKに一般file APIなし |
| POSIX process/thread/signal | fork/execはspawn指向と相容れない（AGENTS.md）。SDKに直接代替なし |
| POSIX socket/mmap/poll | 実network/memory権限が別途必要。host socketで代用しない |
| Linux固有（inotify/epoll/eventfd/timerfd/ioctl/`linux/*`） | Linux kernel ABI依存。ABIの公開は未決で、互換保証なし |
| GTK/GLib/Qt | 依存・移植論点の分類のみ。構造解析はNDP-06/PT-04 |

## 仮定（PROVISIONAL）

- PT-01正規契約は未確定。入力はPR #91 report v1の手書きfixtureであり、本物のscannerからの
  接続成功とはしない。line・evidence等の仮定と上限は[共通README](../common/README.md)。
  共通adapterは正本schemaではなく、`adapter=`引数で差し替えられる。
- `linux-api/linux-header/posix/posix-api/posix-header/gtk/glib/qt`を未知候補のcategoryと
  して暫定使用する。実際のenum値は未確認。既知名はcategoryにかかわらず照合する。
  category集合・名前・理由・参照・不足は`rules.json`で差し替える。名前の種別は
  `headers`（完全一致）、`header_prefixes`、`calls`（`name(`の形のみ）、`identifiers`。
- evidenceはPT-01が渡したソース断片と仮定し、C/C++風のコメント・文字列・文字・raw文字列・
  行継続を除外する。`#include <...>`/`"..."`はheader名として扱う。完全なparserではない。
  断片前のblock comment状態、macro展開、条件コンパイル、alias解決、実呼出しは不明。
- 関数名の照合は保守的: `x.name(`、`p->name(`、`ns::name(`は一致させず、先頭の`::name(`
  （global scope）は一致させる。`&read`のような括弧なしの関数参照は検出しない。
  `connect`/`send`/`signal`/`fopen`のようにQt等と衝突しやすい名前、ISO Cの名前はruleにしない。
  ただし`int read(`の形は宣言・定義とは限らず、型名らしい識別子の有無だけで判断する。
- CMake/pkg-config等のビルド依存宣言（`find_package(Qt6)`等）は対象外。PT-01のdependencies
  項目の契約が確定するまで、adapterが無視する項目に推測で接続しない。
- 参照は`AGENTS.md`（POSIX互換・spawn・capabilityの方針）と現在の`sdk/README.md`、
  C header、Rust SDKの読み取り結果。SDKに新しいAPIや権限を追加しない。SDK変更時は
  JSONの候補と不足を再点検する。
- 代表例の限定coverageであり、全POSIX／Linux／GTK／Qtのsupport matrixではない。
  POSIXをuser-space互換層で扱うかの方針、正規PT-01 ownerによるadapter差し替え、
  実アプリ／targetの受入は後続工程。今回の範囲の実装・host試験はこれらの確認待ちにしない。

## 検証結果

runner指定コマンドで63テスト成功（既存30＋Linux 33、host、Python 3.9）。既存`watch_changes.c`と
negative controlの手書きreportを元入力の各行に照合し、本体rulesとCLIで診断した。全ruleの
全名前、同名user function・member・qualified名、宣言の降格、コメント・各種文字列、
Windows由来・未知名の混在、壊れた／巨大／深いreport、相対path・範囲拒否、決定性、
入力不変性を確認。NDP-04のfixtureとの突合せ（同じ状態名の意味）は統合時に行う。
POSIX/Linux機能は実行しておらず、Linux挙動・Nagi target・正式PT-01 scannerは未試験。
SDK・共有CLI・board・implementation_statusは担当path外のため変更していない。
