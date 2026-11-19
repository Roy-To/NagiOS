# DEC-044 — Private Linux移行開発の開始承認

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（対象限定の開発開始・技術判断委任）／PROVISIONAL（個別構成・正式製品切替）
**合意日:** 2026-11-12
**根拠:** ユーザーが2026-11-12に提示した「Nagi OS：Linux移行開発の開始承認と並列開発体制への切替」（添付本文）。
**対象:** PrivateのLinux移行用開発・検証。既存releaseのPASS、公開・配布許可ではない。
**関連:** [DEC-042](DEC-042-linux-kernel-transition.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[対象限定の技術移行規則](../decisions/2026-11-12-linux-migration-development-scope.md)

> **2026-11-14 後続承認（DEC-047）:** 初回記録にある作業場所・担当の「NOT ASSIGNED」、worktreeを許可しない旨、および未許可のGit/build/VM操作に関する記載は、本日明示されたlane別のSSD worktreeとそのworktree内操作許可の範囲でDEC-047に置き換わる。旧本文は履歴として保持する。統合先・許可操作・継続する列挙事項は[DEC-047](DEC-047-linux-migration-workspaces-and-integration.md)を参照。
## 今回の明示的な開始承認

次の設計具体化・実装・検証を、非公開のLinux移行用開発範囲として開始することを承認する。

- Linux kernelとNagi専用ユーザーランドの境界設計、移行ADR、必要な実装仕様。
- 開発用のLinux最小構成・起動イメージ・build/test経路。
- 既存サービス・アプリ・runtimeの移植とLinux backend/adapter。
- OS非依存部分の再利用・分離・必要修正・検証。
- Linux上での認可・IPC・保存・復旧・UI等の成立性の実装・試験。

この範囲では旧独自kernelの完成・旧kernel固有Milestone完了を一律の着手前提としない。実際に再利用する契約・安全条件・入力を工程ごとに確認する。DEC-042/043の「製品方針だけでは実装開始を許可しない」は当時の承認範囲の記録として保持し、今回明示した開発範囲には本DECが後続承認として適用される。

## 技術判断の委任と残る判断

Dotと担当は固定kernel revision、開発用参照target、toolchain、基本library、boot構成、内部adapter、試験構成について、適合性・保守性・固定入力・依存・出所・license条件を確認し、理由と変更可能な境界を記録して検証用の案を選べる。製品の恒久採用や互換性保証とは別である。全体設計の完了を待たず、最初の工程の境界・責任者・接続条件・検証条件から具体化する。

正式な製品切替時期、適用release番号、公開、配布、利用者データ移行、互換性保証、既存release Gate変更は別途確定する。それらの未確定を今回承認した開発の一律停止理由にしない。公開契約や権限モデルを変更する判断は推奨案と影響をユーザーへ提示する。

Linux + Nagi専用userland、既存distributionを製品OS基盤にしない方針を維持する。個別OSSの選択的再利用は可能。開発hostやbuild toolとしてのdistribution利用と製品image構成を区別する。未許可の環境新設・install・課金は自動的に許可されない。

## 適用場所と割当状態

- 正本・規則改訂先: Private `Roy-To/NagiOS-internal`、既存branch `main`。
- 正本変更責任者: Dot。既存DEC-042/043の保存済み成果を継承し、同じ変更を重複させない。
- 正本編集手段: 認可済みGitHub connector。ローカルcheckout・SSDの走査やGit操作を伴わない文書更新。
- 実装branch、個別変更path、実装担当、検証環境: **NOT ASSIGNED / 未確定**。この正本保存時点では、利用可能性・保護条件が確認された実装割当や実開始を示さない。
- 個々の実装は、実在する許可済み作業場所、変更対象、共有接続条件、資源条件、統合先を明示して開始する。全Goal文面の一括更新や全移行設計の完成は開始条件ではない。
- 既存worktree/cloneの追加を許可しない。安全な追加環境が本当に必要なら、対象・必要性・容量・成果保護方法をまとめてユーザーへ提示する。

## 並列開発と最初の統合

24 Goalは製品目標の追跡であり担当人数の固定枠ではない。既存の一次分類を再利用する。必要な独立工程と資源がある間は実作業を最低5本・通常10本程度とし、実装・検証・引渡しを終えた担当は次の必要工程へ移す。新チャットはLuna Max・通常速度（Fast不可）。設定や送信だけを実開始と数えない。

最優先の統合目標は単一の開発用targetで Linux boot → Nagi userland → Nagi service に到達する再現可能な最小経路。上位処理・adapter・アプリ・試験を並走し、次に既存の一アプリを通常画面から操作、実データ保存、再起動後再読込までつなぐ。未接続機能を接続済みと表示せず、認可を外さない。

共通依存・runner・資源監督は既存の成功経路を再利用し、担当ごとの作り直しを避ける。同一ファイル・共有Git状態・生成物は所有者を分ける。根拠のない追加上限や一回限り制約を新設しない。既存の明示的制限では必要試験が不可能な場合は対象・必要量・理由を提示する。

## 継承する安全・証拠境界

- AI非特権、型付きPublic Action、決定論的認可、取消・失効、データ所有・保護を維持。Linux UID/rootだけで同等性成立としない。
- SSD、Git、保全対象、拒否、結果不明操作、共有資源と個別承認条件を維持。拒否を別担当・別経路で迂回しない。
- 旧コード・branch・未統合差分・試験証拠・既知残件を削除しない。旧kernel専用低層の完成だけを目的とする新規拡張は保留する。
- 旧M30等をPASSにしない。既存release Gateの全面解除、未承認将来機能の一括activationは行わない。
- Private仕様・移行設計・未公開コードはPublic PR/issue/CI log等へ転載しない。
- OS非依存logic試験、Linux上の同一本体コード/adapter試験、Nagi用Linux image内統合試験を別々に記録する。前段成功、旧kernel PASS、test-only別実装を新target PASSとしない。
- 既存の許可済み上位作業は止めない。保存失敗時は未保存と具体的障害を報告する。設定保存・指示送信・引受け・実開始・成果・正式受入を区別する。

## 変更履歴

- 2026-11-12: ユーザーの明示承認を記録。対象限定のLinux移行開発を開始可能とし、製品切替・公開・既存release Gateとは分離。個別実装割当は未確定として記録。

## 開発環境での初回統合到達点（2026-11-16 13:16 JST）

**実装記録／開発target限定:** 04:16 UTCの保存済み実行結果を再確認し、x86-64 QEMUで Linux起動 → Nagi userland → 開発用service READY → Notes通常Desktop入力からNagi VFSへ保存 → 同じディスクで別起動後に同一object・revision・本文を読戻す経路を確認した。両phaseのNotes childとQEMUはexit 0、timeout/cancelなし。固定sourceの新しいexportからlocked buildした成果物に対する結果であり、今回の文書更新で再実行した結果ではない。

この到達点は本DECの最初の開発統合目標に対応する。**実機受入、製品向け認証済みREADY、Linux移行全体の完了、電源断耐久、ReleaseのPASSを意味しない。** 開発限定handshakeと通常製品initの認証経路は別で、後者の未接続部分はUnavailableを維持する。後続変更のhost testや型検査をこのrunのruntime成功へ合算しない。既存のRelease／Activation／Acceptance Gateと安全境界は変更しない。

詳細・残条件は [implementation status](../implementation_status.md) の「Linux development-target acceptance — 2026-11-16 13:16 JST」を参照。非公開の生ログ・ローカル個人パス・会話記録は本記録に含めない。

## 認証付き正常経路の開発QEMU実測（2026-11-19）

署名済み開発イメージの x86-64 Linux QEMU（2 GiB RAM、4 vCPU）で、実認証、C1 Session、正式な service READY、Notes の保存・正常終了、同じ Data での冷間再起動・再認証と同一オブジェクト／版／描画内容の読戻しが成立した。固定版は A `eecf55f988` と C2 `747feeefd4`、同一接続証明は `6064decb3b`。手順と result.json の要点は[実装状況](../implementation_status.md#linux-development-target-acceptance--2026-11-19-task24)を参照。

これは開発環境の正常系の記録であり、製品の署名や構成、実System RO、WS-10、8 GiB／4 vCPU性能の受入を示さない。実機検証は #163 の受入条件から外し、DEC-102 の別工程とする。

## PR #163 の開発イメージ受入基準（2026-11-20）

この節は #163 の開発イメージに限る。製品構成、対応機器、既存 Release Gate は変更しない。

1. **性能の測定・記録:** ARM ホスト上で x86-64 を TCG によりエミュレートする QEMU を 8 GiB・4 vCPU に固定し、通常操作の入力から描画までの遅延と、ゲスト RAM の最大使用量を測定・記録する。測定方法、測定窓とサンプリングの限界を明記し、ホスト側 QEMU プロセスのメモリをゲスト RAM の代用にしない。#163 に数値による合格ラインは置かない。数値基準は DEC-102 の実機検証で決める。
2. **PID 1 の全体 READY:** 開発イメージの必須サービス（現在は Identity のみ）すべての READY を確認した後、PID 1 自身が起動全体の READY を起動ごとに一度だけ外部へ通知する。Identity 側の通知だけでは代用しない。必須サービスが揃う前には全体 READY を通知しない。これは製品の必須サービス構成を確定しない。

System RO と WS-10 は引き続き別の受入項目。クラッシュ時の回収確認では、別の単体試験だけで代用せず、統合イメージで通常稼働中のサービスが異常終了した経路を試す。
