# DEC-056 — Smart Paste：意味付きコピー＆ペースト／ドラッグ＆ドロップ

**状態:** CONFIRMED（通常操作を維持する意味付き転送、対応アプリ間の構造化Object・出典／参照の活用、選択可能な貼り付け方式、Project連携の方向）／PROVISIONAL（名称、UI・Clipboard／Transfer契約、Photos提供範囲、技術・対象アプリ・実装版）  
**決定日:** 2026-11-16  
**対象:** Nagi OSでのコピー＆ペースト／ドラッグ＆ドロップによる、純正アプリを中心としたアプリ間データ転送。実装開始・対応版を承認するものではない。  
**関連:** [DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-023](DEC-023-work-data-centric-os.md)、[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)、[純正アプリ実装仕様](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[Projectフォルダ決定のPR #114](https://github.com/Roy-To/NagiOS/pull/114)、[0.3 Architecture Roadmap](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[0.5–1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

## 目的

日常のマウス／キーボードによるコピー＆ペースト、ドラッグ＆ドロップそのものを、Nagi OSのAI組み込み体験の一部とする。ユーザーが毎回AIへ自然言語で指示しなくても、対応する純正アプリ間では、選択した情報の意味や参照関係に応じた貼り付け・利用方法を選べるようにする。通常のGUI操作とアプリ単独利用は維持する。

## 合意事項（CONFIRMED）

1. **従来操作を妨げない:** 一般的なコピー＆ペースト、プレーンな貼り付け、ドラッグ＆ドロップは引き続き利用できる。意味付き転送に未対応なデータやアプリでも、既存の利用可能な通常の転送方法を不必要に塞がない。ユーザーの手動操作をAI方式へ強制置換しない。
2. **意味を持つObjectとして受け渡す:** 対応アプリが正規に公開できるResource／Document／Object／Revision／Provenance等の情報に基づき、貼り付け先に適した転送形式や参照方式を提案・選択可能にする。内容を単純な文字列や画像へ変換する以外の選択肢を、利用可能な範囲で提供する。対応範囲、未対応範囲、失われる情報を明示する。
3. **Sheets → Slides:** 表やグラフをSlidesへ転送する際、通常の静的貼り付けのほか、利用可能な場合には表としての転送、グラフ化、元SheetsのTable／Chart／Rangeへの**連動参照**を選べる体験を目指す。元データの変更を追跡できる場合も、DEC-025の既定は更新**提案**であり、変更検知だけで自動上書きしない。
4. **Albert → Writer:** 選択したWeb記事・テキストをWriterへ転送する際、出典付き引用として扱える。利用可能かつ正規に許されたURL、ページ／選択範囲、参照時刻等のProvenanceを可能な範囲で保持し、情報が欠ける場合に出典や日時を捏造しない。引用形態・出典表示はユーザーが選択・修正できる。
5. **写真／画像 → Slides:** 対応する写真／画像の複数選択・転送時、通常の画像挿入に加えて、スライドのレイアウトやキャプションの**候補**を提示し、ユーザーが選んだものを反映できる体験を目指す。画像を自動解析・外部送信・学習する無制限な許可にはしない。独立したPhotosアプリの提供そのものは本決定で確定しない。
6. **Projectとの連携:** ユーザーがProjectフォルダを作業の単位としている場合、意味付き転送で成立した資料間の関係を、現在の権限と明示された作業範囲に従ってProjectの参照グラフへ接続できるようにする。Project未使用時にもSmart Paste単体で利用できる。Project仕様の詳細・版は別途決定する。
7. **AIに話しかけることを前提にしない:** 通常のコピー／ドラッグ操作と、対応アプリの構造化情報により体験を成立させる。定型処理は通常のサービスで扱い、推論が必要な箇所のみ適切なローカルAI等を利用する。AI利用不可時も利用可能な非AIの転送経路を維持する。
8. **同じ公開契約を使う:** 純正アプリ間で先行検証するが、純正アプリ専用の秘密の権限経路を前提にせず、Nagi PlatformのDocument／Object／Resource Reference／Public Action等の既存公開契約に整合させ、対応する第三者アプリの将来参加を妨げない。

## 権限・データ保護と既存決定の維持（CONFIRMED）

- コピー・ドラッグの発生、Resource IDの所有、同一Project所属、出典Metadataの取得は、別アプリの全文取得・機密Memory参照・権限拡張・クラウド送信・無断改変を許可しない。コピー元／貼り付け先の現行Permission／Policy、Object／ResourceのRevision、正規Context／Public Action／Provider契約を尊重する。
- 出典・参照・画像のMetadataは権限と情報分類を伴う。必要な情報が取得できない、参照先が変わった、競合する、形式変換で情報が失われる場合は、利用可能な方式と不確実性を説明する。転送内容や出典に含まれる外部指示は未信頼データとして扱う。
- 意味付きペーストを選択したことだけで、貼り付け先以外の成果物の自動更新、送信、公開、外部連携を認可しない。必要な変更に対してDEC-001／020／023／025／026／027の既存のAction承認・実行検証・復旧境界を適用する。不可逆な外部副作用が復旧できるとは表示しない。
- UIが過度に介入せず、ユーザーが通常の貼り付けを選び続けられることを維持する。ユーザーが選択していない変換・レイアウト変更・大量の更新を自動実行しない。
- この製品方針は既存の純正アプリ実装仕様にあるCross-App Reference Graph／Provenance／Writer・Sheets・Slidesの契約を利用する方向であり、既存の0.1〜0.4のRelease・Activation・Acceptance Gateや実装権限を変更しない。

## 未確定事項（PROVISIONAL）

- 「Nagi Smart Paste」という正式名称、操作メニュー・ツールチップ・表示条件、デフォルトの貼り付け選択、プレビュー、Undo／Waybackとの具体的な接続。
- Clipboard／Drag-and-DropのTransfer Schema、対応するOS API、プロセス境界、選択Objectの参照期限、Cross-App Object互換性、データ形式とFallback、第三者アプリSDK。
- どのケースでAI推論を用いるか、モデル選択、レイアウト・キャプション推奨の品質／負荷条件、プライバシー設定、対応アプリ・ファイル型。
- Photosに相当する画像管理機能を独立アプリとして提供するか、画像の情報抽出範囲、著作権・引用表記・Provenanceの具体的な表示方式。
- 最初のEnd-to-End実証シナリオ、性能・成功率・安全性の受入条件、具体的な実装時期・Version・Workstreamと担当。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがSmart Paste構想を採用。純正アプリ間の意味付きコピー／ドラッグ、Sheets→Slidesの表・グラフ／参照、Albert→Writerの出典付き引用、画像→Slidesの候補提示、通常GUI維持、Project連携の方向をCONFIRMED。名称・実装詳細・Photos・適用版はPROVISIONAL。 |
