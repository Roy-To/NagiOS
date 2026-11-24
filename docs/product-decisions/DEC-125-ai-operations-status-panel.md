# DEC-125 — AI稼働状況パネル：Task・Model・Resource可視化

**状態:** CONFIRMED（AI／Modelの稼働、Task、資源の可視化パネルを採用）／PROVISIONAL（指標・Provider接続・UI・技術・時期・版）  
**最終合意日:** 2026-11-24  
**適用候補:** Nagi Home／Desktop、Taskセンター、Model Runtime／Resource Managerの統合表示  
**関連:** [DEC-002](DEC-002-cloud-ai-data-sharing.md)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-094](DEC-094-resource-manager-linux-baseline-custom-scheduling-research.md)、[DEC-123](DEC-123-kernel-unification-and-linux-0.1.md)、[0.3 Roadmap](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)

## 目的とユーザー体験

Nagi上のAI／モデルとTaskがいつ動作しているのか、RAM等の資源をどれほど使っているのかをユーザーが把握できるようにする。OmarchyのAIサービス管理UI（[公式OSS](https://github.com/omacom/omarchy)）を参考にするが、同じ表示実装・Provider APIを採用する決定ではない。

## 確定した仕様（CONFIRMED：高水準方針）

- **AI稼働状況パネルを製品方向として採用する。** NagiのローカルLLM、AI Task、関連する資源利用の状態を一元的に確認できる方向とする。
- **事実に基づく表示とする。** 稼働／停止／待機・Task進捗、取得可能なRAM等の使用情報を示し、未計測・推定・取得不能な情報を実測値と偽らない。
- **外部AIの利用枠も取得可能なら表示する方向とする。** 実際に認証・許可されたProviderが取得可能な範囲に限り、全Providerのクォータ／リセット時刻の取得を約束しない。
- **既存の権限と操作を維持する。** パネル閲覧だけでTaskは開始・再開されず、モデルの常時RAM常駐やCloudへの無断アクセスを必須としない。

## 例外・禁止・安全境界

- User／Profile／Permission、Task Scope、停止・復旧待ち、Privacy、Cloud送信の別許可はDEC-002／011／020に従う。
- 外部アカウントの認証情報、秘密情報を画面・Diagnostics／Activityへ転記しない。
- Host-onlyの計測をLinux Nagi Targetの動作証拠と呼ばない。取得元・更新状態を確認できる設計を目指す。
- UI表示や診断処理で前景GUIの応答性を犠牲にしない。

## 未確定・暫定案（PROVISIONAL／OPEN）

- Home／Desktop／Taskセンター等への置き方、情報密度、手動更新や表示頻度。
- CPU／RAM／GPU、ロード状態、Queue、Task進捗、Cloud残量などの詳細な採用項目、計測元、取得周期。
- Provider連携、連携できない場合の表示、アクセス権、Pause／Stop・モデル切替等の操作をパネルに含めるか。
- 実装技術・対象端末・性能・Acceptance・担当・提供Versionは未決定。

## Roadmapとの関連

- 0.1：DEC-123のLinux Developer Preview公開条件・範囲を変更しない。
- 0.2：既存のModel Runtime、Jobs／Task、Resource、Diagnostics基盤を利用する候補。新たなRelease Blockerではない。
- 0.3以降：Personal Agent／Taskの見える化と接続する方向。既存Gateを変更しない。
- 本DECは実装・検証・表示機能の完成宣言ではない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-24 | ユーザーがOmarchyを参考にAI稼働状況パネルを進めることを承認。高水準の可視化方針をCONFIRMED、UI・計測・Provider／提供版をPROVISIONALとした。 |
