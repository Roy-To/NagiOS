# NDP-07 — UI対応表と未対応レポート

状態: 担当確定（Core Developer、2026-11-15）/ NOT STARTED。PT-04。NDP-06の利用可能な出力、UI ownerとの接続点、担当pathの承認が必要。

## 目的

元UIの構造を現行Nagi UI契約に照合し、対応できる部分、再設計が必要な部分、backend未実装を区別する。表示だけの成功を操作可能なアプリへ読み替えない。

## 編集範囲案

`tools/nagi-developer-platform/ui/mapping/**`とそのテスト。parserと共通型は合意版を読み、同時編集しない。

## 実装

元control/layout/event/bindingに対し、根拠があるNagi UI候補だけを対応付ける。直接対応、要再設計、未提供、不明を区別する。対応表の具体schemaと表現は内部実装案として決められるが、SDK APIや正式support matrixを増やさない。

レポートには、見た目・layout、pointer/keyboard操作、focus、accessibility、localization、event/業務ロジック接続を別項目で示す。rendererの存在が未確認なら明示する。AI補完を後付けする接続点は作れても、初回から外部AI呼出しを必須にしない。

## focused test案

既知control、unknown、eventのみ未接続、長い日本語/英語text、focus情報欠落、binding欠落、SDK版不一致、部分対応、同一入力の決定性を試す。全項目が見えているだけで「移植完了」になる判定を作らない。

元API/候補APIの根拠が消えたり版が違ったら、対応済みのまま放置せず未確認として扱う。

## 完了境界

NDP-06の実出力を使うmapping/reportテストと、対応・不足を説明できる例。Nagi上のrender/操作を試していなければその項目は未検証。host reportの成功だけでGUI受入をPASSにしない。

## 次

不足部品・input/backend・公開APIの改善案をUI ownerへ返す。NDP-11で再設計工数と未対応の比較に使う。

共通: [規則](../03_COMMON_RULES.md)、[権限](../04_SCOPE_AND_AUTHORITY.md)。
