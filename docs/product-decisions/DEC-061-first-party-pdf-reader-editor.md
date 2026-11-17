# DEC-061 — Nagi純正PDF Reader / Editorと段階的な高度編集

**状態:** CONFIRMED（純正アプリ採用・段階的拡張・AI非必須・公開API連携）／PROVISIONAL（具体的な提供版・高度機能の順序・技術・合格値）

**最終合意日:** 2026-11-17（PX-11との整合を追記。2026-11-16の採用決定を維持）

**適用候補:** 将来のNagi OS純正アプリ。具体的なリリース版・実装時期は未決定

**関連:** [DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-023](DEC-023-work-data-centric-os.md)、[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-002](DEC-002-cloud-ai-data-sharing.md)、[純正アプリ仕様](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[ロードマップ索引](roadmaps/README.md)

## 目的とユーザー体験

PDFを読むだけでなく、日常の文書作業をNagi純正アプリで進められるようにする。**Nagi純正PDF Reader兼Editorを採用し、Acrobatのような高度な編集まで段階的に拡張する方向を確定する。** 「Adobe Acrobat完全互換」や全機能の初期実装を約束するものではない。

## 確定した仕様（CONFIRMED）

### 基本体験を先に成立させる

- 高速なPDF表示、検索、ページ操作、注釈、印刷、フォーム入力などの基本体験を先に整える。
- **通常の閲覧・編集はAIに依存しない。** AIが無効・不在・利用不能でも、対応済みの通常機能をマウス・キーボード等のGUIから使える設計を維持する。
- 高速性は製品目標であり、達成済みの性能や特定の待機時間・メモリ使用量を保証しない。

### 高度編集は後続で段階的に検討する

- 既存PDFの本文・画像の高度な編集、OCR、適切な墨消し、署名関連機能を、基本体験の後に段階的に検討・拡張する。
- **墨消しは見た目の黒塗りで済ませず、出力PDFから対象の元内容を削除し、削除できたことを検証する。** 検証できない結果を「安全に墨消し済み」と扱わない。
- 墨消し出力と、原本・旧Revision・Checkpoint・抽出済みデータ・派生資料は区別する。出力の墨消しを、それらの一括消去や既に外部へ送信した資料の回収と表示しない。これらは既存の所有権・Permission・Retentionに従い、追加の削除範囲や復旧との関係は別途設計する。
- 署名関連機能の検討方針は、特定の電子署名方式、証明書・鍵管理、法的効力、検証サービスの採用や対応済み宣言を意味しない。

### Nagiの文書作業と公開APIで連携する

- Nagi Project、Smart Paste、Writer、Sheets、Slides等と、**Public Resource / Document / Object / Revision / Provenance / Action API**の共通契約で連携する方向とする。
- PDFや参照対象を共通のResource / Document / Objectとして扱い、Revisionと出典・変換履歴を追える形を目指す。正本は元アプリ・Provider・利用者の保存先に残せる。具体的なObject粒度、識別子、Schema、Action名は未決定。
- Nagi Project / Smart Pasteは連携先の製品方向として記録する。既存の純正アプリ仕様にあるStudio / Workspaceの実装済み名称変更、提供済みAPI、単一DBへの集約を意味しない。
- 純正専用の秘密APIや追加権限を設けず、第三者アプリと同じ公開契約・Permission / Capability / Policyに従う。

### AI支援は利用者の承認と元資料の境界内で行う

- PDFの表抽出、文章要約、文書比較、引用などをAI支援対象とする。利用者が承認した目的・対象・Task Scopeと、元資料の現在の権限・機密分類・所有範囲内で扱う。
- 内容の取得・出力先への反映は正規Public APIを通し、Resource / Revisionと現在のPermission / Capability / Policyを確認する。PDFを開いたこと、Projectへの所属、コピー操作だけで無制限なAI利用・外部送信を許可しない。
- 抽出・要約・比較・引用の派生結果でも元資料の分類とProvenanceを失わせない。引用元や対象Revisionを確認できないものを、確認済みの引用・正確な抽出として表示しない。
- Cloud AIへの送信はDEC-002の宛先・目的・情報区分等の事前許可と実行時検査に従う。AI支援の承認はCloud送信や第三者共有への包括許可ではない。
- 元資料の変更検知はDEC-025どおり更新案の提示が標準であり、関連するWriter / Sheets / Slides等を自動編集しない。承認済みTask内の例外も現在のScope・権限・Revision確認に従う。

## PX-11 — 品質重視の純正アプリ構成との整合（2026-11-17）

[DEC-021](DEC-021-roadmap-0.5-1.0.md)のPX-11で、**B案：日常作業を純正アプリで実用的に完結できる品質重視の構成**を基本、**将来C案：総合的な純正ソフトウェア環境**へ発展する高水準方針を採用した。

**既存CONFIRMEDは維持:** PDF Reader兼Editorを純正採用し、AIに依存しない高速閲覧・検索・ページ操作・注釈・印刷・フォーム入力等の基本体験を高度編集より先に重視する。高度な本文・画像編集、OCR、真に内容を削除・検証する墨消し、署名関連の段階的な検討、正規Public API／Permission・Provenance、Cloud送信別許可を変更しない。PDFからのAI支援・他アプリ更新は既存の許可・Scopeの範囲に限る。

**PROVISIONAL／OPEN:** PDFアプリを1.0で標準同梱・任意導入・後続のどれに分類するか、1.0の最小機能・PDF対応規格・高度編集／OCR等の範囲、機能順序、性能・セキュリティ・アクセシビリティ合格値、UI・技術・実装／配布Version。PX-11はPDFの全機能1.0搭載、Adobe Acrobat互換、Release／Activation／Acceptance Gate更新の宣言ではない。

## 未確定・暫定案（PROVISIONAL）

- 具体的なリリース版、提供時期、基本機能内の実装順序、高度編集・OCR・墨消し・署名関連機能どうしの順序と各段階の対応範囲。
- 表示・編集エンジン、既存OSSの選択・ライセンス、保存方式、対応PDF規格・版・プロファイル、暗号化PDF等の対応範囲。
- OCRの実装技術・対応言語・実行場所・品質検証、電子署名の形式・証明書・鍵管理・検証方式。
- 墨消しで検査する内容・履歴・埋め込み情報等の具体的範囲、検証手段、旧Revision・派生データとの関係、出力・共有前のUI。
- 正確性、性能、互換性、アクセシビリティ、OCR、墨消し、署名等の試験方法・テスト資料・合格値。
- アプリの正式表示名、Package / App ID、UI、公開APIの詳細、担当WorkstreamとAcceptance。

## Roadmap・既存仕様との関係

- 純正アプリ仕様のWriter / SlidesによるPDF出力とは別に、PDF自体の閲覧・編集を担うアプリの採用方針を追加する。既存の出力機能を置換しない。
- 0.3 Personal Agent、0.4+、0.5–1.0のロードマップでは将来の連携先として参照できるが、特定版や既存M-APPへの割当て・Release blockerを追加しない。
- 本記録は製品仕様の確定事項であり、PDF機能の実装・試験成功・提供済み、Workstreamの割当て・Activation、既存Acceptance Gateの変更を意味しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | 純正PDF Reader兼Editorの採用、基本体験を先に整えた後の高度編集への段階的拡張、AI非必須、公開APIによる文書作業連携を記録。具体的な版・高度機能の順序・技術・合格値は未決定として分離。 |
| 2026-11-17 | PX-11の品質優先B＋将来の総合純正環境Cの高水準方針に整合。PDF Reader／Editor採用と基本操作先行・AI非依存・高度編集段階化・正規API／権限を維持。1.0必須／任意／後続分類、最小機能、技術・互換・性能・提供版はPROVISIONAL。 |
