# DEC-022 — Builderの中断・再起動後の作業復旧

**状態:** CONFIRMED（方式C。保持データ形式・復旧条件などの技術詳細は未確定）  ／PROVISIONAL（Nagi内Role切替の実装詳細・将来専門Agent化）
**最終合意日:** 2026-11-05  
**適用候補:** Nagi OS 0.4のBuild／Execute作業要件（初期Nagi Role切替C。後続Agent配置は未確定）
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-015](DEC-015-builder-role-review.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-019](DEC-019-builder-work-data-cleanup.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)

> **名称・位置づけ訂正（2026-11-05）:** 以前の「Jev」という内部名称は外部のTypeSafe AI製品との混同だったため撤回した。**Builderは暫定の作業担当ラベル**で、独立した専門Agent／Roleへの割当てと初回0.4導入は未確定。選択済みの動作・安全条件を記録するが、誤解を根拠にAgent構成やモデル選定を確定しない。詳細は[DEC-024](DEC-024-external-jev-reference-and-builder-label-correction.md)。

> **Role名の確定（2026-11-05）:** プログラミング担当の正式Role名は `builder`（小文字）。この文書の旧「Builder」「仮称」の記述は命名前の履歴であり、初期はNagiがRole切替で実行し、将来の独立Agent化は未確定（DEC-015／DEC-024）。

> **2026-11-05 別会話継続方式Cとの区別:** [DEC-015](DEC-015-builder-role-review.md)／[DEC-020](DEC-020-ai-os-interaction-path.md)で認めるのは、前景会話のための**Resource待機のみ**から、承認と現行権限が有効なTaskを条件付きで継続すること。ここで扱う**クラッシュ／電源断／再起動等の復旧待ち**は別状態であり、ユーザーの選択前に副作用を自動再実行しない。本決定の復旧方針Cは維持する。

## 目的とユーザー体験

Builderによる作業中にOSのクラッシュ、電源断、再起動、実行環境の中断が発生しても、元のファイルや作業結果を不用意に失わず、ユーザーの意思と最新の権限に従って復旧できるようにする。

## 確定した仕様 — 選択C：状態保持とNagiによる再開提案

1. **自動再実行しない。** 予期しない停止後の起動・復旧時に、Builderの中断Taskや副作用のあるActionを自動再開・再実行しない。Nagiは復旧可能性を調査・提示できるが、ユーザーの選択前に作業を勝手に進めない。
2. **状態と成果を可能な範囲で保持する。** Task Log、Activity Ledger、隔離作業領域、変更差分、検証証拠、反映状況、復旧用データを参照し、実際に保存・照合できた範囲を再構成する。すべての実行途中の状態を完全復元できると約束しない。
3. **Nagiがユーザーに選択肢を提示する。** 復旧状況、最後に確認された成功・失敗・不明のAction、ファイル変更、再実行による影響を説明したうえで、**「続きから再開」「最初からやり直す」「中止」**を選べるようにする。実行できない選択肢は理由を説明する。
4. **再開前に検証する。** ユーザーが再開またはやり直しを選んだ場合も、現在のUser / Profile、OS Policy、Permission、Task／DelegationのScope・期限・Budget・Cancel状態、対象ResourceのRevisionと競合、隔離領域の内容・保護状態を再評価する。失効した過去の許可や委譲をそのまま引き継がない。必要なら新たな承認を求める。
5. **副作用の二重実行を防止する。** Task／Attempt／Actionの識別子とActivity等により実行済み・未実行・結果不明を区別する。「最初からやり直す」も無条件の全Action再送ではない。結果不明の非冪等Actionや不可逆Actionは、自動的に繰り返さず、状況照合と必要な確認・認可を求める。
6. **反映・復旧・整理の境界を維持する。** 反映済みローカル変更、未反映Patch、部分適用、実行途中の作業を区別し、古い成果で最新のユーザーファイルを上書きしない。復旧待ちの成果物や証拠を通常の自動整理対象に含めない（DEC-018／019）。
7. **中止は明示的な終了である。** ユーザーが中止を選んだら、新しいActionの実行を止め、現存する変更・副作用を正確に報告する。中止操作を既存の変更の完全巻戻し・成果物即時削除と同一視しない。

## 例外・禁止・安全境界

- Workspaceのチェックポイントの復元（DEC-014）は、AI Taskの自動再開や再実行を意味しない。本決定もその区別を維持する。
- NagiやBuilder自身の記憶・自己申告だけで「復旧成功」「Action未実行」と判定しない。照合できない結果は不明と示し、必要なActionを保留する。
- 副作用がある操作は正規のPublic ActionとProviderによる現行Permission／Policy検査を通し、復旧のための権限昇格やGit commit／push等の追加権限は認めない。
- 本決定は0.3 Acceptance、0.4 Entry Gate、Integration Owner checkpoint、Workstream activationや実装済みを意味しない。

## 未確定・暫定案

- 永続化するTask状態の項目、再起動時の整合性検査手順、復旧用Checkpoint・Snapshotの具体方式。
- 状態再構成の精度、再開可能／再試行可能の判定、復旧途中の失敗状態遷移、UIでの詳細表示。
- 復旧待ち状態の保持期間、容量上限、作業領域の分類、再検証が必要なAction種別。
- クラッシュ・電源断・Provider再起動などのFault injection / Acceptance testの条件と実測上限。

## Roadmapとの関係

0.4+ RoadmapのTask Log、Activity Ledger、Idempotency、Stop、有限Retry、Revision検査、Waybackに整合する。初回はNagi一体が`builder` Roleに切り替えてTaskを実行する方式C（DEC-015）とし、0.3のTarget Acceptanceおよび明示的な0.4起動Gateを変更しない。

## 決定履歴

> 2026-11-05: 旧内部名称をBuilder（仮称）へ訂正。Agent／Role配置は再検討対象（DEC-024）。

| 日付 | 内容 |
|---|---|
| 2026-11-05 | ユーザーが選択C（中断状態を保持し、起動後にNagiが再開・やり直し・中止を提示）を確定。自動再実行禁止、現行権限・Revision確認、二重実行防止を記録 |
