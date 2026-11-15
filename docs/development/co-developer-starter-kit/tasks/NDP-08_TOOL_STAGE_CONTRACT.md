# NDP-08 — 正規tool利用表とStage契約

状態: 担当確定（Core Developer、2026-11-15）/ NOT STARTED。PT-05。NDP-01、CLI/Package owner確認、担当記録の許可が必要。

## 目的

既存のNagi CLI/SDK/Package toolを再利用し、静的検証、host build/test、Nagi target受入を明確に分ける。別のbuild systemやPackage規格を作らない。

## 読むもの

現行 `AGENTS.md`、developer preview guide、`tools/nagi-cli`とbootstrapの実装、`tools/nagi-pkg`のmain/Manifest validator、SDK contract、DEC-045 PT-05。実際の利用可能コマンドをコードと正規手順で確かめる。

## 作業

各toolについて実在するentry、入力、出力、終了状態、副作用、必要な依存・実行許可を小さな表にする。`./nagi doctor/fetch/build/image/run/test`は全て同じ副作用ではない。名前を知っているという理由で一括実行しない。

Stage 1はManifest/API/Packageの静的な確認に限定し、入力projectのbuild scriptを実行しない。Stage 2は承認されたhost build/testとPackage検査。Stage 3は実在するNagi targetと別途の条件が揃ってから行う。Stage 3未実施でも、Stage 1/2の実施結果は独立に示す。

## 成果

NDP-09が使う正規tool表と小さな内部adapter案。未提供tool、曖昧な終了コード、出力の不足はownerへ返す。未確認のコマンド構文、SDK download、署名手順を作らない。

## 確認項目案

- 正規entryの根拠と対象版がある。
- Stage 1から任意コードや依存取得が走らない設計である。
- 失敗/未実行/実行不能を区別する。
- 開発用署名と正式署名、Package生成とinstallを分ける。
- testsだけのfake runnerを本番toolとみなしていない。

これは契約整理の完了であり、build・QEMU・target試験成功を意味しない。

## 次

許可されたStage範囲が決まればNDP-09へ。tool本体の変更は既存ownerへ最小提案で渡す。

共通: [規則](../03_COMMON_RULES.md)、[レビュー](../06_REVIEW_INTEGRATION_RESUME.md)。
