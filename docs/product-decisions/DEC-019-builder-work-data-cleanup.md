# DEC-019 — builder作業データの状態別整理

**状態:** CONFIRMED（整理方式C、数値・実装詳細は未確定）  ／PROVISIONAL（Nagi内Role切替の実装詳細・将来専門Agent化）
**最終合意日:** 2026-11-04  
**対象:** Nagi OS 0.4のBuild／Execute作業要件（初期Nagi Role切替C。後続Agent配置は未確定）
**関連:** [DEC-015](DEC-015-builder-role-review.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-040](DEC-040-builder-app-development-project-retention.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md), [DEC-041](DEC-041-builder-app-uninstall-and-data-deletion.md)

> **名称・位置づけ訂正（2026-11-05）:** 以前の「Jev」という内部名称は外部のTypeSafe AI製品との混同だったため撤回した。**`builder`は確定したプログラミングRole名**で、初期はNagi自身がRoleを切り替えて実行する方式Cで確定（DEC-015）。将来の独立Agent対応と具体的なRuntimeは未確定。選択済みの動作・安全条件を記録するが、誤解を根拠にAgent構成やモデル選定を確定しない。詳細は[DEC-024](DEC-024-external-jev-reference-and-builder-label-correction.md)。

## 目的

builderの隔離編集で生じる一時作業領域、ビルド生成物などを、作業の状態と復旧の必要性に応じて安全に整理する。

## 確定した仕様 — 選択C

- **正常完了:** 変更の安全な反映・検証を終え、必要な差分、検証証拠、Activity記録、復旧手段が保持されていると確認できた場合、不要なTask専用の一時データを自動整理できる。単なる完了判定だけを整理許可としない。
- **失敗・未完了:** 失敗、部分成功、ユーザー確認待ち、反映未了、停止・中断、状態不明の作業については、再開・確認・復旧に必要な作業領域と証拠を保持する。
- **保護対象:** 元のファイル、復旧用データ、ユーザーが保護した成果物、実行中の作業領域、共有キャッシュ、他TaskやOwnerが利用するデータを無断で整理対象にしない。正式なbuilder製アプリ開発Projectと将来の改修・検証・復旧に必要な記録は[DEC-040](DEC-040-builder-app-development-project-retention.md)に従う保護対象であり、Task一時データやCacheとして整理しない。 アプリ本体のアンインストールや関連データの完全削除をTask／Cache整理に含めず、[DEC-041](DEC-041-builder-app-uninstall-and-data-deletion.md)に従う。
- **安全判定:** 自動整理前に対象、所有、使用中か、参照関係、Permission、保持Policyを確認する。判定できない場合は保留する。
- **結果表示:** Nagiは何を整理・保持したかをTask／Activityの証拠と整合する形で示す。保持や復旧の可否を誤って報告しない。

## 例外・安全境界

- 自動整理は未承認の削除権限を発行しない。副作用はOSの正規Public Action／Provider検査を通す。
- Workspace状態保存（DEC-014）はファイル本文の復旧用保護を代替しない。DEC-018の隔離・反映・復旧境界を維持する。
- 本決定は実装済みや0.4 release／workstream activationを意味しない。既存Gate、AGENTS.md、Owner境界を変更しない。

## 未確定・暫定事項

保持期間、容量上限、整理頻度、逼迫時の優先度、保護解除UI、隔離領域・キャッシュの具体的な分類と検証手順は実装・安全性評価で決定する。数値は確定しない。

## 決定履歴

> 2026-11-05: プログラミングRole名を `builder` と確定。初期はNagiのRole切替C、将来の独立Agent化は未確定（DEC-015／DEC-024）。

| 日付 | 内容 |
|---|---|
| 2026-11-04 | ユーザーが選択Cを確定。正常完了後の安全な自動整理、未完了・保護対象の保持、数値未確定を記録 |
