# NDP-05 — Linux由来APIの静的差分rules

状態: 担当確定（Core Developer、2026-11-15）/ NOT STARTED。PT-03。NDP-01、PT-01 interface合意、host-only開始承認が必要。NDP-04の実装完了待ちは不要だが同じ接続契約を使う。

## 目的

POSIX、GTK、QtなどLinux側依存の候補を分類し、Nagiの実在する公開APIとの差分を示す。Linux kernel採用をLinuxアプリの無変更実行保証と混同しない。

## 編集範囲案

`tools/nagi-developer-platform/api/linux/**`と独立したLinux用fixture/test。共有report型やcommon helperは調整担当へ変更案を渡し、Windows担当と同じファイルを同時編集しない。

## 実装

既存PT-01の正規report入力を消費するrules/adapterに限定する。新しいディレクトリscanner、任意コマンド実行、include先の無制限探索は作らない。根拠・分類・対応候補・不足・不明を区別し、利用したSDK/contract版を説明する。

最初はファイルI/O、process/thread等から小さな一分類を選ぶ。元APIと同名のNagi APIを想像して対応付けない。GTK/Qtの構造解析はNDP-06に任せ、このtaskでは依存・移植論点の分類に留める。

## focused test案

既知候補/未知候補、同名user function、コメント/文字列、複数platformの混在、不正report、対象外path、壊れた文字列、過大入力、同一入力の決定性を検査する。入力を変更せず、元プロジェクトのbuild scriptを実行しない。

host-independentなrules試験は、POSIX機能を本当に実行するテストと区別する。Windows上でfixture分類が通っただけでLinux挙動を検証したとしない。

## 完了境界

実際のrulesを呼ぶhostテストと正規入力の接続例。ソース移植済み、UI動作、Nagi権限検証を示す成果ではない。NDP-04と共通fixtureを比較し、同じ状態名の意味がずれていないことを統合時に確認する。

## 次

不足APIをownerへ短く返し、NDP-08/11で使える検証・評価材料へつなぐ。接続待ちを独自SDKの作成で回避しない。

共通: [規則](../03_COMMON_RULES.md)、[キュー](../05_BACKLOG_AND_PARALLEL.md)。
