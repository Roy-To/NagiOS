# DEC-056 — Smart Paste：意味付きコピー＆ペースト／ドラッグ＆ドロップ

**状態:** CONFIRMED（従来操作・意味付き転送・出典／参照と選択式の既存方針、PX-12の貼り付け方式を案内する統合体験B＋将来の高度なアプリ横断ワークフローCという高水準方針）／PROVISIONAL（初期アプリ組合せ、コピー／参照の既定方式、詳細UI／プレビュー・Clipboard／Transfer契約・Fallback制御の細部・技術・性能・提供版）  
**決定日:** 2026-11-17（PX-12追加。2026-11-16の既存合意を維持）  
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

## PX-12 — 案内型Smart Paste B＋将来の高度なアプリ横断ワークフロー C（2026-11-17）

### CONFIRMED — B案を基本とする高水準の利用体験

**従来のコピー／貼り付けやドラッグ＆ドロップを自然に続けられること**を前提として、対応アプリ間で転送可能な意味付きデータ、貼り付け先のCapability、元のResource／Revision／Provenanceと既存Permissionを確認したうえで、利用可能な**貼り付け方法をユーザーに分かりやすく案内し、本人が選択できる統合体験**を基本とする。

例えば独立した静的コピー、可能な場合の編集可能な構造化Objectへの転送、元データへの連動参照など、**実際に利用できる方式と差異・制限・失われる情報を明確に示す**。参照先が無効・権限不足・形式未対応等の場合は、成立する通常の貼り付け／手動転送へ戻れる方向とし、使えない参照や未検証の出典を利用可能と表示しない。

この方針は、特定メニューを必ず表示する、すべての転送に確認画面を出す、静的コピー／参照のどちらかを既定選択にする、特定Appペアを最初にReleaseする、といった**詳細UIや初期対応範囲の確定ではない**。

### CONFIRMED — 将来C案へ発展する高水準方向

B案のユーザーによる方式選択を保ち、既に許可されたProject／Task文脈とResource間の関係、Provenance、変更Event等を活かして、**複数アプリにまたがる転送・参照関係の整理、変更影響の理解、関連成果物の更新候補を一貫して支援する高度なワークフロー体験**へ段階的に発展させる。利用できる連携や復旧能力と実際の状態を正確に示し、無断の更新・データ収集・外部送信を伴わない。

将来Cは新たな自動実行権限、無条件の双方向同期、アプリ間の完全互換、提供版・初期実装順序の確定ではない。

### 維持する既存CONFIRMEDと安全境界

- **[本DECの既存Smart Paste](DEC-056-smart-paste-semantic-transfer.md):** 通常のコピー・プレーンな貼り付け・Drag-and-Drop、Sheets→Slidesの表／Chart／連動参照、Albert→Writerの出典付き引用、画像→Slidesのレイアウト／キャプション候補、Project参照グラフへの許可済み接続、AI不在時の通常転送、純正と第三者の共通Public APIを維持する。これらは体験方向であり、初期の対応アプリ組合せ・形式、個別実装完了の宣言ではない。
- **[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)の方式Aは不変:** Resource／Document／Object／Revision／Provenanceの**共通参照と元App／Providerの分散正本**を維持。元データ変更時は**検知・更新提案が標準**で、承認済みTaskで明示された対象・操作・目的・Scope内の場合に限り、現在のPermission／Policy・元／先Resource Revision等を実行時に再検証して自動更新できる。Smart Pasteを選ぶだけで他成果物の変更・同期を承認したことにはならない。
- **[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)の方式Aは不変:** アプリ操作は**型付きPublic Action／Service API優先**。未対応アプリでNagiがGUI／Accessibility fallbackによって操作する場合は、明示許可と狭めたTask／App／操作／Resource Scope、現行Permission／Policy、実行後検証、Stop／Cancelを守る。これをSmart Paste自体の非AIの通常手動貼り付けと混同しない。安全に許可／確認できない場合は手動操作へ戻し、隠れた権限拡大にfallbackを用いない。
- **データ・UI・Gate:** 元Resourceと派生情報の分類／Owner／Permission／Privacy、推測と確定済みProvenanceの区別を維持し、引用元やRevision不明のものを捏造しない。UIの過度な割込みや選んでいない大規模変更を避ける。異なるAppへのアクセス、Cloud AI送信、共有、未承認のActionは別途の許可が必要。既存0.1〜0.4のRelease／Activation／Acceptance GateとWorkstream開始条件・実装の進捗を変更しない。

### PX-12の未決事項（PROVISIONAL／OPEN）

- **初期に対応するアプリ間の組合せ、対応Object／ファイル形式・最初のE2E試験フロー**。既存のSheets→Slides、Albert→Writer、画像→Slidesは採用した体験方向であり、初期出荷ペアの確約ではない。
- コピー／構造化転送／参照の**既定選択**、貼り付け方法のメニュー・プロンプト・プレビュー・出典表示・更新提案・Undo／Wayback接続の具体UI、抑制・表示条件。
- Clipboard／Drag-and-DropのTransfer Schema、Object互換性・元参照の寿命、Fallback手段と具体制御・非対応時の変換、第三者SDK、Provider・Context／Permission・Revisionの具体的接続。
- データ更新と多アプリTaskのPartial／Conflict／Recovery表示、AI推論の適用条件、モデル・技術方式、性能・安全性・正確性のAcceptance基準、実装順序／Workstream／提供Version。既存の明示許可と限定fallbackはPROVISIONALに戻さない。

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
| 2026-11-17 | PX-12で貼り付け方式を分かりやすく案内する統合Smart Paste体験Bと、将来の高度なアプリ横断ワークフローCを高水準でCONFIRMED。DEC-025／027の既存方式A、DEC-056の既存体験・通常GUI・権限／承認を維持。最初の対応App組合せ、既定のコピー／参照、詳細UI・Transfer契約・fallback制御・技術・性能・提供版はPROVISIONAL。 |
