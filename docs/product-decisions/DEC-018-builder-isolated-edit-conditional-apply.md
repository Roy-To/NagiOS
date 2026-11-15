# DEC-018 — builderの隔離編集と条件付き自動反映

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（方式C。具体的な実装機構・閾値は未確定）  ／PROVISIONAL（Nagi内Role切替の実装詳細・将来専門Agent化）
**最終合意日:** 2026-11-04  
**適用候補:** Nagi OS 0.4のBuild／Execute作業要件（初期Nagi Role切替C。後続Agent配置は未確定）
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-015](DEC-015-builder-role-review.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-036](DEC-036-builder-app-trial-and-installation.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)

> **名称・位置づけ訂正（2026-11-05）:** 以前の「Jev」という内部名称は外部のTypeSafe AI製品との混同だったため撤回した。**`builder`は確定したプログラミングRole名**で、初期はNagi自身がRoleを切り替えて実行する方式Cで確定（DEC-015）。将来の独立Agent対応と具体的なRuntimeは未確定。選択済みの動作・安全条件を記録するが、誤解を根拠にAgent構成やモデル選定を確定しない。詳細は[DEC-024](DEC-024-external-jev-reference-and-builder-label-correction.md)。

## 目的とユーザー体験

builderのコード修正を実際の作業領域へ安全かつ円滑に反映する。テスト失敗や未検証の修正で元のファイルを不用意に変更せず、通常の検証済み修正は必要以上にユーザーを待たせない。

## 確定した仕様 — 選択C：隔離編集・条件付き自動反映

1. **隔離して修正・検証する。** builderは元の作業領域を保護し、別の作業領域で調査、変更、ビルド、テスト、必要な再修正を実施する。隔離領域の作り方（コピー、作業用Workspace、差分適用方式等）は未確定とする。
2. **反映前にNagiが判定する。** Nagiは対象、変更差分、試験・検証証拠、受入条件、元ファイルの現在のRevisionと競合、既存のPermission・Policyを確認する。builder自身の成功申告だけでは合格としない。
3. **通常の変更は条件付きで自動反映する。** ユーザーが事前承認したTask・Workspace・対象ファイル・書込みActionの範囲内で、変更が通常リスクに収まり、検証が成功し、競合がなく、反映前のファイル内容が実際に保護された場合に限り、Nagiは正規Public Actionを通じて自動反映できる。反映のActionは実行時にOS／Providerが再認可する。
4. **重要変更や不確実な変更は確認に回す。** 重要・高リスク変更、承認Scope外、テスト失敗、検証不能・証拠不足、元のファイルの競合・Revision不一致、反映前保護の失敗、Permission拒否、状態不明は、自動反映しない。Nagiは差分・理由・対応案を示し、適切なユーザー確認・追加承認を求める。
5. **元の状態を守る。** 自動反映する前に、対象となるファイル内容の元状態を復旧可能な方法で保護する。保護不能な場合は「保護済み」とみなさず反映を止める。Workspaceの配置・アプリ状態の自動保存（DEC-014）は、ファイル本文の保護や差分の復旧を代替しない。
6. **履歴と結果を明示する。** Nagiは反映対象、変更差分、検証証拠、保護状態、実際に反映した範囲、失敗・部分成功、可能な復旧経路を報告する。対応できないUndoや完全復旧を約束しない。
7. **Git操作などは別権限。** 自動反映は承認されたローカルファイル変更の反映であって、Git commit／push、本番環境反映、外部送信、Release等の許可を含まない。これらは明示的な別許可と現行Policy検査を要する（DEC-016）。

### 新規アプリの試用・初回正式導入との権限区別（DEC-036、2026-11-05 CONFIRMED）

builderが作成した未導入アプリの試用と初回正式導入は[DEC-036](DEC-036-builder-app-trial-and-installation.md)に従う。本DECのコード差分の条件付き反映は、試用・採用・初回インストール・永続登録を含まない。

### 新規アプリの導入後保守・更新との権限区別（DEC-035、2026-11-05 CONFIRMED）

`builder`が作成したアプリの正式導入後の保守は[DEC-035](DEC-035-builder-created-app-maintenance-updates.md)を適用する。ユーザーが承認した開発Taskでの**隔離コード修正・Build／Test・通常差分の条件付きローカル反映**（本DEC）と、**インストール済みアプリのVersion変更・正式更新インストール・永続登録**は別のActionとして扱う。後者はNagiの更新提案、必要な事前Task認可、検証結果・権限・影響の説明とユーザーによる更新承認、正規Package／Policy検査を経るまで進めない。データ・設定の保護、失敗・部分成功・復旧可能範囲も偽らず区別する。

## 例外・禁止・安全境界

- 隔離領域での実行も、必要なResource／Tool／Permission／Budgetを越えてよいことを意味しない。未信頼入力からの指示でScopeを拡大しない。
- 元のファイルがユーザーや他の処理により変更された場合、古い差分によって新しい内容を無断上書きしない。再検証、衝突表示、ユーザー判断へ移す。
- 部分反映・中断・取消・クラッシュ時には、実際の反映状態を調査・記録し、未反映／不明な部分を完了扱いにしない。Stop・Activity・Waybackの既存境界を尊重する。
- 本決定は0.3 Acceptance、0.4 Entry Gate、Integration Owner checkpoint、既存Workstreamの所有権／実装許可を変更せず、実装済みの証拠とはならない。
- 未公開の仕様をPublicリポジトリへ転記しない。

## 未確定・暫定案

- 隔離編集の具体的方式、対象ファイルの保護・復旧・一括反映の実装、隔離領域の状態別整理の詳細（[DEC-019](DEC-019-builder-work-data-cleanup.md)で基本方式Cを確定）。
- 「通常」「重要」の分類、反映のAtomicity、衝突解消・Merge・失敗時の詳細、利用できる検査の種類。
- タスクごとの保護容量・時間・Resource budgetと、ユーザー確認UIの詳細。

## Roadmapとの関係

- **0.3:** 単一Personal Agent、Public Action、Permission、Activity／Waybackなどを前提とし、既存Gateは維持する。
- **0.4:** DEC-015／DEC-016のNagi + builder、承認Scope内の自律修正、リスク別の完了判定を継承し、コードの反映前の隔離・検証・保護・条件付き適用を定める。初回のAgent構成・逐次実行は変更しない。

## 決定履歴

> 2026-11-05: プログラミングRole名を `builder` と確定。初期はNagiのRole切替C、将来の独立Agent化は未確定（DEC-015／DEC-024）。

| 日付 | 内容 |
|---|---|
| 2026-11-04 | ユーザーが選択C（隔離編集・条件付き自動反映）を確定。通常変更は条件付き自動反映、重要・競合・未検証はユーザー確認、原本の保護とGit操作別許可を記録 |
