# DEC-126 — Writer／Sheets／Slidesの標準保存形式とOffice互換

**状態:** CONFIRMED（B案：Nagi独自の公開Native形式を標準保存形式とし、Office形式の入出力を提供する方向、共通コンテナによる再利用と内部表現の柔軟性）／PROVISIONAL（拡張子、コンテナ／Schema、互換性の具体範囲、実装順・性能・提供版）
**最終合意日:** 2026-11-25
**適用候補:** Writer、Sheets、Slides。個別機能の提供版は未確定。
**関連:** [DEC-021](DEC-021-roadmap-0.5-1.0.md)、[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-056](DEC-056-smart-paste-semantic-transfer.md)、[DEC-090](DEC-090-common-persistence-contract-distributed-authority.md)、[純正アプリ実装仕様 §40、57–59](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

## 目的とユーザー体験

通常の文書、表計算、プレゼンテーションの作成・保存と他OS利用者との交換を可能にしながら、Nagiの独自の参照・出典・Object管理などの機能を外部のファイル形式の制約で失わないようにする。

## 確定した仕様（CONFIRMED）

- **B案を採用:** Writer／Sheets／SlidesはNagi独自のNative形式を標準保存形式とし、その仕様を公開する。Microsoft OfficeのDOCX／XLSX／PPTXは対応する各アプリの入出力形式とする。Office形式を全アプリの内部正本・共通保存形式として強制しない。ODFを標準保存形式とするA/B/C比較のC案は採用しないが、将来のODF入出力を禁止する決定ではない。
- Nagi独自機能に必要なDocument／Object ID、Revision、Provenance、連携参照等の保持を目指す。元資料の正本と共通データ契約はDEC-025／090に従い分離し、第三者アプリの内部保存形式を強制しない。
- 既存純正アプリ実装仕様 §57.14／58.11／59.11のOffice形式の基本入出力・未対応機能警告、Markdown／テキスト、CSV／TSV、PDF／画像等の対応目標を維持する。完全なOffice互換、マクロ実行、高度な機能の無損失往復を保証しない。AIなしで通常の編集・保存を成立させる。

## 例外・禁止・安全境界

- 変換不能・欠落する機能を黙って破棄せず、出力・保存結果を正確に伝える。元のNative文書や読み込んだ元ファイルを無断上書きせず、競合・保存失敗・結果不明を成功扱いしない（DEC-090）。
- 参照・出典・外部形式へのExportは、現在の所有・Permission／Policy・機密分類・Persona／Secret・外部共有の境界を維持する。ローカルの参照情報や機密メタデータを無断で外部形式に付加しない。参照の解決・関連データの更新はDEC-025／056の既存ルールに従う。
- ファイルの読み込みで外部由来の内容を命令・承認として扱わない。未対応マクロ等の自動実行を本決定で許可しない。

## 未確定・暫定案（PROVISIONAL）

- 拡張子候補: Writer `.nwriter`、Sheets `.nsheets`、Slides `.nslides`。正式名称・MIME type・登録は未決定。
- 構造化コンテナ＋公開metadata／Schemaを候補とし、物理コンテナ、圧縮、暗号化、添付／参照の表現、Schema version／migration、第三者向け仕様・検証ツールは詳細設計で決定する。
- Office互換の対応機能一覧、未対応情報の保護／警告／変換プレビュー、元ファイル保護、参照・出典・独自機能のExport時の扱い、往復互換・不正入力・容量・性能の試験、ODF対応の採否、個別提供版は未確定。

## Roadmapとの関連

既存の0.2 Document Framework／公開契約・SDK基盤を利用し、後続のWriter／Sheets／Slides実装に接続する。既存Release／Activation／Acceptance Gate、Workstreamのowner・開始条件、個別1.0搭載範囲を変更しない。これは製品方針の記録であり、実装や互換性検証が完了したことを意味しない。

## 2026-11-25 — 共通コンテナの基本候補と内部表現の柔軟性

**CONFIRMED（方向）:** Writer／Sheets／Slidesの公開Native形式では、既存技術を活用した構造化コンテナと共通Document Frameworkによる保存・読込等の再利用を進める方向とする。可読性・処理性能・容量を両立するため、すべての内部データをJSONに統一せず、用途ごとに適切な公開仕様の表現を選択できるようにする。第三者アプリの内部形式を強制しない。

**PROVISIONAL（第一候補・検証対象）:**

- ZIPを基本コンテナ候補とし、metadata／小規模構造化データはJSON等、大量セル等は効率的なバイナリ表現、画像・音声・動画等は適切な既存形式を候補とする。具体圧縮方式、ZIP64、分割・部分更新、SQLite等の代替、Schema／拡張子・名称は未確定。ZIPを全アプリに強制しない。
- manifest、アプリ別本文、assets、styles、references／provenance、必要に応じたpreviewの役割を分離する案。本文・メタデータの形式／内部ファイル名／配置は未確定。アプリ間転送の共通契約と保存コンテナを区別する。
- 現在の文書と文書内のコメント・レビュー情報、OS側のWayback／Activity履歴、再生成可能な検索Index／preview、復旧用一時データを区別し、過去の全版を文書に無制限に蓄積しない設計を検討する。共有・Exportにおける機密メタデータの漏えい防止、現在の権限・分類を維持する。
- 先行比較候補: 読み書き・Schema検証・version migration・添付／preview・安全な保存／復旧・Office変換。大容量／部分更新、RAM・一時容量・性能、破損／容量不足／中断、不正入力（展開上限、path traversal等）、互換性を検証する。既存DEC-088／090、Release／Activation／Acceptance Gate、Workstream ownerと開始条件を維持する。具体技術の採用・実装開始・提供版の確約ではない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-25 | B案（Nagi独自の公開Native形式を標準とし、Office形式を入出力）をユーザーが選択。具体拡張子・形式・互換範囲・版はPROVISIONAL。 |
| 2026-11-25 | 既存技術を活用した共通コンテナと用途別内部表現の方向を追記。ZIP・JSON・バイナリ等の具体採用、保存構造・履歴分担・性能はPROVISIONAL。 |
