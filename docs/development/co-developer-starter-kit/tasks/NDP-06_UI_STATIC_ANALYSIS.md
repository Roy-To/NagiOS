# NDP-06 — 宣言的UIのbounded static analysis pilot

状態: PASS（2026-11-15、Qt Designer XMLのhost pilot限定）。18テストと独立再レビューが成功。NDP-07/UI owner接続・target受入は別途。

## 目的

小さなUI定義から構造・部品・event・未知要素を読み取り、Nagiへの再構成に必要な材料を作る。元GUI Frameworkの実行や見た目の完全再現は目標にしない。

## 編集範囲案

`tools/nagi-developer-platform/ui/analysis/**`と独立テスト。`user/nagi-ui/**`やrenderer本体を編集しない。NDP-07のmappingとファイルを分ける。

## 最初の対象

Qt Designer UI、GTK UI、XAML等は候補で、確定済みの対応一覧ではない。現物・権利・既存契約を踏まえて1形式の小さな合成fixtureから始め、選択理由を短く残す。全形式対応を初回の必須条件にしない。

## 実装

選択された入力だけを解析し、階層、layout、control、text、event名、bindingの存在、accessibility情報、unknownを表す内部モデルを作る。これは本taskの技術案であり、Nagiの公開UI schemaを新設するものではない。

入力に埋め込まれたコード、イベント処理、外部resourceを実行・取得しない。XMLを扱う場合は外部entity/DTDの解決を行わず、深さ、要素数、入力サイズを有限にする。具体的上限は実装内で明示して試験し、製品の固定性能要件にしない。

## focused test案

最小画面、入れ子、unknown control、壊れた構文、過大/深い入力、外部entity/resource、event/binding、Unicode、同一入力の順序と結果を確認する。元入力不変・外部通信なし・対象外へのアクセスなしを試す。

## 完了境界

本体parserとhost試験、対応した構造と未対応一覧。画面表示、クリック、keyboard focus、保存などが動いたという結果ではない。

## 次

内部モデル・例・unknown一覧をNDP-07へ渡す。production UIの不足は既存ownerへ返し、別rendererを作って埋めない。

共通: [規則](../03_COMMON_RULES.md)、[レビュー](../06_REVIEW_INTEGRATION_RESUME.md)。

## 2026-11-15 実装結果

担当内の技術判断としてQt Designer XMLを初回形式に選択し、既存のUI契約を照合。実装・テスト・手順は `tools/nagi-developer-platform/ui/analysis/**`。入力と階層の上限、DTD/entity・symlink・範囲外path拒否、未対応と未接続の保持を実コードで確認した。既存renderer/SDK/共有registryは変更していない。対象はhost静的解析のみ。独立レビューで発見・確認した情報欠落を9b68295で修正し、再レビューで未解決P1/P2なし。詳細は [Codexレビュー](../../../../reviews/ndp-host-pilots.md)。
