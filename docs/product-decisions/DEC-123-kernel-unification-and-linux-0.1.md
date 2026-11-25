# DEC-123 — 独自カーネルの終了とLinux版 Nagi OS 0.1 Developer Preview

**状態:** CONFIRMED（独自カーネル開発の終了とLinuxカーネルへの一本化、Linux版0.1の公開範囲B、旧M0〜M30の記録の保持、0.2有効化条件の置き換え）／PROVISIONAL（各アプリの機能深度・受入詳細、新マイルストーンの区切りと名称、旧コードの削除・移管の方法と時期、CI再編の具体、公開時期）
**最終合意日:** 2026-11-25
**適用候補:** Nagi OS全体の開発方針、0.1 Developer Preview、0.2以降のRelease／Activation Gate  
**関連:** [DEC-042](DEC-042-linux-kernel-transition.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-044](DEC-044-linux-migration-development-authorization.md)、[DEC-047](DEC-047-linux-migration-workspaces-and-integration.md)、[DEC-077](DEC-077-linux-kernel-lts-stable-policy.md)、[ROADMAP](../../ROADMAP.md)

## 目的と背景

DEC-042／043により、製品方針は「Linuxカーネル＋Nagi専用ユーザーランド」に確定している。一方、0.1の実装・ROADMAP・M0〜M30は独自カーネルを前提としたままで、0.2の有効化条件も旧M30のPASSに結び付いていた。PR #163で、Linux開発イメージ上の起動・PID 1・Identityサービス・ログイン・Notesの保存と読み戻しが通る見込みとなったため、独自カーネルの扱いと0.1の位置付けを確定する。

## 確定した仕様（CONFIRMED）

### Q1 — 独自カーネルの開発を終了する

1. **Nagi OSのカーネルはLinuxに一本化する。独自カーネル（`kernel/`、`loader/`、`third_party/relibc` と `target_os = "nagi"` 向けの経路）の新規開発・機能追加・修正は行わない。**
2. 独自カーネル向けのコードは凍結する。リポジトリからの削除または別リポジトリへの移管は、PR #163のマージ後に行う（方法と時期はPROVISIONAL）。
3. 製品のアプリ・サービスはNagi専用ユーザーランドとして作る方針（DEC-043）を変えない。

### Q2 — 0.1 Developer PreviewはLinux版として公開する

1. **「Nagi OS 0.1 Developer Preview」は、Linuxカーネル上のNagiとして公開する。** 独自カーネル版の0.1は公開しない。
2. 旧0.1の全機能（M0〜M30）をLinuxへ移してから公開するのではなく、**範囲を絞ったLinux版0.1**を最初の公開とする。公開範囲はB案とし、基本Desktop、Notes、Files、Settings（英語・日本語）、Calendarを含める。
3. この範囲決定はアプリごとの実装完了や公開受入合格を意味しない。各アプリの機能深度と受入詳細は別途既存Gateで判断する。

### Q3 — 旧M0〜M30の記録は保持する

1. 旧M0〜M30の文書・試験・状態は、独自カーネル時代の記録として残す。既存のPASS／PARTIAL／BLOCKEDを書き換えず、Linux版の結果として読み替えない。
2. Linux版の進み具合は、新しいマイルストーンで記録する（区切りと名称はPROVISIONAL）。

### Q4 — 0.2の有効化条件を置き換える

1. 0.2のRelease Runtime Activation Gateにある「旧M30のPASS」の条件を、**「Linux版0.1 Developer Previewの公開（配布物の作成と検証を含む）」**に置き換える。
2. その他の0.2・0.3・0.4+のGate、Acceptance条件、Workstreamの所有権は変えない。

## 例外・禁止・安全境界

- 独自カーネル向けのコードを凍結している間も、既存の公開済み履歴・タグ・記録は改変しない（作成者名義の統一のための履歴書き換えは別の決定に従う）。
- 旧M0〜M30の結果を、Linux版の受入証拠として使わない。Linux版の受入は、Linux開発イメージ／製品イメージ上の実測だけで判定する。
- 独自カーネルの終了を理由に、既存の安全境界（認証・権限・署名検証・Data保護）を緩めない。

## 未確定・暫定案（PROVISIONAL）

- Linux版0.1に含める各アプリの機能深度・制限、配布物と検証（旧M30相当）の詳細。ブラウザー（Albert／Servo）、ローカルAI（Granite）、音声、意味検索、A/B起動とリカバリーを0.2以降とする案。
- 新マイルストーンの区切り（Linux移行計画のL0〜L6を土台にする案）と名称。
- 独自カーネル向けコードの削除または別リポジトリへの移管の方法・時期。
- CIの再編（Linux向けの回帰を残したまま、独自カーネル向けのビルドジョブを外す方法）。
- ROADMAP、README（英日）、0.1実装仕様、リポジトリ説明文の書き直し。
- Linux版0.1の公開時期と、対応する仮想環境・実機（DEC-102、DEC-116）。

## Roadmapとの関連

- 0.1: Linux版Developer Preview（公開範囲B: Desktop、Notes、Files、Settings、Calendar）。旧M0〜M30は記録として保持。
- 0.2: 有効化条件をLinux版0.1の公開に置き換える。0.2の内容は変えない。
- 0.3／0.4+: 変更なし。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-23 | ユーザーが「独自カーネルはやらない。Linuxカーネル一本に絞る」と確定。0.1 Developer PreviewをLinux版として範囲を絞って公開すること、旧M0〜M30の記録の保持、0.2有効化条件の置き換えを確定。機能範囲・マイルストーン・旧コードの扱い・CI再編・公開時期はPROVISIONAL。 |
| 2026-11-25 | ユーザーがLinux版0.1の公開範囲B（Desktop、Notes、Files、Settings、Calendar）をCONFIRMEDとして選択。狭いQEMU previewのA案は不採用とし、#247 §6に履歴として残す。アプリごとの機能深度と受入詳細はPROVISIONAL。 |
