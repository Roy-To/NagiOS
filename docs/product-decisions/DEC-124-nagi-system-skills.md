# DEC-124 — Nagi System Skills：OS機能のAI向け操作知識

**状態:** CONFIRMED（System Skillsの採用方向）／PROVISIONAL（具体構成・技術・UI・版・時期）  
**最終合意日:** 2026-11-24  
**適用候補:** Public Action／Settings／Manifestを利用するNagi OSのAI統合体験  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-002](DEC-002-cloud-ai-data-sharing.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-123](DEC-123-kernel-unification-and-linux-0.1.md)、[0.3 Roadmap](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)

## 目的とユーザー体験

NagiがOS固有の機能・設定・操作の意味を理解し、自然言語の依頼から適切な操作候補を案内できるようにする。OmarchyのAI Skills（[公式OSS](https://github.com/omacom/omarchy)）は参考事例だが、技術・内部実装・無制限なシェル操作をNagiへそのまま採用する決定ではない。

## 確定した仕様（CONFIRMED：高水準方針）

- **Nagi System Skillsを製品方向として採用する。** AIがOSの機能、Settings、正規の操作方法を理解し、ユーザーの意図に沿って案内・利用することを目指す。
- **実行の正本は既存の公開契約。** Public Action、Settings Schema、App Manifest／SDK、Identity、Permission／Policyを迂回せず、Skillに書かれた説明だけで追加の実行権限は生まれない。
- **AIなしの通常操作を維持する。** Skillが存在しない、利用不能、誤っている場合でも通常GUI・設定操作が成立する。
- **非特権AI・Offline-firstを維持する。** System Skills採用はCloud送信の自動許可やUser／Profileの一括閲覧権限を意味しない。

## 例外・禁止・安全境界

- Skill文章・第三者による指示は未信頼な情報であり、ユーザー承認、System Capability、Policyと同一視しない。
- Action実行時は現在のCaller／Session、Scope、Revision、Permissionと承認条件を再検証する。GUI fallbackはDEC-027の限定条件内に留める。
- 実装されていない操作・Target未受入の機能をSkillsの説明だけで利用可能と表示しない。
- System Skillsの導入とOSの任意設定ファイルを無制限に書き換えられることは別である。

## 未確定・暫定案（PROVISIONAL／OPEN）

- Skillの形式・登録／配布・信頼性確認・更新・署名・無効化方式。
- Manifest／Action Schema／Settings情報からSkillを生成・照合するか、初期の操作カタログをどう選定するか。
- Model Adapterへの提供方式・Context量・低RAM端末での動作・UI・操作確認・失敗時の表示。
- 実装順序・担当・Acceptance・適用Release。特定の提供版は約束しない。

## Roadmapとの関連

- 0.1：DEC-123のLinux Developer Preview公開範囲・Gateを変更しない。
- 0.2：正規のPublic Action／Settings／App契約を利用する設計候補であり、新たな0.2 Release Blockerではない。
- 0.3以降：Personal AgentがSystem Skillsを利用する方向と整合する。既存のActivation／Acceptance条件を維持。
- 本DECは実装開始・完成・検証成功の宣言ではない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-24 | ユーザーがOmarchyを参考にSystem Skills、AI稼働状況パネル、Workspace手動モード切替の3案を進めると明示確認。System Skillsの高水準採用方向をCONFIRMED、詳細をPROVISIONALとした。 |
