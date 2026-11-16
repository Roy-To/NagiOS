# DEC-065 — Nagi Handoff（仮称）：別端末での作業継続

**状態:** CONFIRMED（純正OS機能としての採用・意味的な作業継続・安全境界）／PROVISIONAL（名称・UI・復元範囲・取得／転送方式・クラウド利用・性能・適用版）

**最終合意日:** 2026-11-16

**対象:** PC・タブレット・スマートフォンのNagi端末を横断する作業継続。対応端末・実装時期は未確定

**関連:** [DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-023](DEC-023-work-data-centric-os.md)、[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)、[純正ソフトウェア仕様](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[ロードマップ索引](roadmaps/README.md)

## 目的とユーザー体験

ユーザーがPC・タブレット・スマートフォンを持ち替えたとき、対応する文書や作業の続きへ戻れる純正OS機能を採用する。作業名は「Nagi Handoff」であり、正式名称は未確定。

例：PCでWriterの文書を編集中に、スマートフォンのNagi Homeから「PCの作業を続ける」を選び、対応文書の編集位置から作業を再開する。これは採用した基本体験の例であり、現在の実装・全端末への対応・具体的な表示文言を確約するものではない。

## 確定した仕様（CONFIRMED）

- **別端末で作業を継続する:** 編集中の文書、開いているResource／Document、編集箇所、対応アプリの意味的な作業状態を、別のNagi端末で継続できる体験を純正OS機能として採用する。
- **意味を引き継ぎUIを再構築する:** 移行先の画面・Surface・Device Capabilityに応じてUIを再構築する。ピクセル座標、マウス位置、Desktopのウィンドウ配置やプロセスのbinary stateの複製を前提にしない。
- **既存契約を最大限再利用する:** First-Party Implementation Specの§27–30（Device／Surface／Device Capability／Continuity）、§54.3 Continueと§54.7 `home.continue_from_device`、§65.5 Handoff、§71 PC→Phone→PC、§97 Continuity、およびDEC-009の再開体験、DEC-014の選択式Workspace移行／同期を基礎にする。DEC-023／025の公開契約・共通参照・分散正本を維持する。
- **状態とファイル実体を分離する:** 意味的な状態や参照が渡っただけではファイル本文が移行先にあるとは扱わない。実体がない場合は、ユーザー許可に基づく正規の取得／転送経路、または利用可能な代替を提示する。取得不能・未許可であれば、その制約を表示する。
- **現行状態を再検証する:** 受信先の現在のUser／Profile／Owner／Permission／Privacy、Capability／Policy、Source有効性とRevisionを、表示・取得・復元・適用の正規経路で再検証する。元端末の権限・古いプレビュー・端末の信頼状態だけで認可を代替しない。
- **結果を正確に示す:** 競合、未対応、参照先の失効、部分復元、復元不能、不明を成功と区別して表示する。対応する範囲を超えて完全な作業移行を装わない。

## 例外・禁止・安全境界

- Handoffの選択を、ファイル・個人データの無断転送、クラウド送信・同期、Profile全体の移行、未公開アプリ内部状態の収集の包括的許可とみなさない。対象と必要な取得／転送範囲をユーザーが確認・許可できるようにする。
- DEC-014の端末ごとの独立管理、選択式移行／同期、暗号化Export／Import、競合保護・中断復旧、現行権限・明示的忘却の再評価を維持する。Handoffの採用だけで同期を既定で有効にしたり、標準の移行方式を置き換えたりしない。
- 受信先の既存文書・Workspace・履歴・手動配置を無断で上書き・削除せず、競合・Revision不一致・安全に解決できない状態は確認に戻す。意味的な編集位置の復元はファイル本文の巻き戻しではない。未保存編集はDEC-014の対応アプリの回復情報と競合処理に従い、任意の未保存内容が必ず届くとは扱わない。
- 中断したAI Taskや副作用Actionを自動再開・再実行しない。作業画面・状態の継続と、Task実行・データ変更・外部送信の認可を分離する。
- 情報区分、外部共有、所有範囲、正規Public Action／Service APIの境界を維持する。純正機能や同じ所有者の端末であることを理由に権限を拡大しない。

## Nagi Shareとの関係

Nagi Share（仮称）は近距離での単発ファイル直接転送、Nagi Handoff（仮称）は作業を別端末で継続する体験であり、別の概念である。Nagi Shareの採用からHandoffや継続同期を導かず、Handoffの採用からNagi Shareを必須転送方式とも決めない。

先行するNagi ShareのDEC-064は[PR #125](https://github.com/Roy-To/NagiOS/pull/125)に記録され、本記録時点ではmain未マージである。同記録がHandoffを今回の採用範囲外のアイデアとした点に対し、本DECは後続の独立した採用決定を記録する。Nagi Share自体の範囲や転送方式は変更しない。

## 未確定事項（PROVISIONAL）

- 正式名称、Home／Continue／Devicesでの表示UI・文言・通知・操作詳細。
- どのアプリがどの意味的状態・編集位置・未保存編集を復元できるか、対応範囲と互換性。
- ファイル実体を届ける取得／転送経路、共有・端末間接続・認証の具体方式、クラウド利用の採否と構成。共通参照があっても実体の可用性を保証しない。
- 切替時の瞬時性・待ち時間・性能、オフラインで成立する具体的条件、競合解消・代替提示のUIと実装方式。
- 対応端末、Version、実装順序・時期・担当、受入条件。

「全アプリ・全状態の完全移行」「インターネットなしで必ず端末間Handoff」「自動同期」は今回の合意で保証・確定しない。DEC-014やNagi Shareのオフライン方針を、あらゆるHandoffの成立保証へ拡張しない。

## Roadmapとの関連

既存の端末非依存モデル、Desktop first、Resource／Document／Revision、Workspace／Continuity、Permission／Capabilityの基盤を再利用する。0.3 Personal Agent、0.4+、0.5〜1.0の特定Versionへ今回の記録で割り当てず、既存Release／Activation／Acceptance Gate、担当・着手条件を変更しない。

純正ソフトウェア仕様§71の `revisions sync` は、現行認可・明示的な範囲選択・実際の対応方式が成立する場合の例として読む。Handoffによる無条件の自動同期を確定する記述ではない。本記録は製品方針の採用であり、実装開始・実装完了・実機での作業継続成功の証拠ではない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | Nagi Handoff（仮称）を純正OS機能として採用。Nagi端末横断の意味的な作業継続・Capabilityに応じたUI再構築と既存Continuity／Home／DEC-014の再利用、許可に基づく実体取得、現行認可・Source／Revision検証、正確な結果表示、Task／副作用Actionの自動再実行禁止を確定。名称・復元範囲・経路・クラウド・性能・適用版は未確定。単発ファイル転送のNagi Shareと区別する。 |
