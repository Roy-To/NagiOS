# DEC-117 — ユーザーデータの暗号化と端末紛失時の方針

**状態:** CONFIRMED（Q1〜Q5の高水準方針）／PROVISIONAL（暗号方式・Filesystem連携・鍵階層・UI・適用版）／REVIEW NOTE（関連DECとの整合）  
**最終合意日:** 2026-11-20  
**適用候補:** Linuxカーネル＋Nagi userlandのUser Data・Profile・Memory。適用版は未確定  
**関連:** [DEC-003](DEC-003-memory-classification.md)、[DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-068](DEC-068-nagi-sleep-resume-recovery.md)、[DEC-079](DEC-079-system-image-readonly-ab-data-separation.md)、[DEC-083](DEC-083-nagi-identity-linux-credentials-binding.md)、[DEC-086](DEC-086-nagi-secrets-service-hybrid-key-protection.md)、[DEC-087](DEC-087-tiered-system-trust-and-signature-verification.md)、[DEC-088](DEC-088-linux-filesystem-nagi-storage-api.md)、[DEC-105](DEC-105-user-environment-backup-and-wayback.md)、[DEC-116](DEC-116-performance-targets-and-minimum-requirements.md)

## 目的と背景

DEC-086で、鍵の保護方式（Nagi Secrets Service、対応機器でのTPM／TEE優先、非対応機器でのパスフレーズ保護、クラウドへの自動預託なし、Profileごとの鍵の分離）は確定している。一方、DEC-088では、ユーザーデータを暗号化するかどうかが未確定だった。

Personal AgentのMemoryとProfileは利用者の記憶・習慣を含むため、端末の盗難・紛失時に漏れた場合の被害が一般的なOSより大きい。本DECで、保存データの暗号化と紛失時の方針を定める。

## 確定した仕様（CONFIRMED）

### Q1 — ユーザーデータの暗号化は標準でオン

1. **ユーザーデータの暗号化は、標準でオンとする。ユーザーはオフにすることもできる（B）。**
2. **ただし、MemoryとProfileは常に暗号化し、オフにできない。**
3. OS本体（System Image）は、通常時read-onlyかつ署名検証の対象（DEC-079／DEC-087）であるため、暗号化の対象外とする。

### Q2 — 起動時の解錠

1. **TPM等のある機種:** TPM等が自動で鍵を開け、ユーザーは通常どおりログインする。
2. **TPM等のない機種:** 起動のたびにパスフレーズを入力して解錠する。
3. TPM等のある機種でも、**起動時にPINを要求する設定をオプションで選べる**ようにする。

### Q3 — 回復キー

1. **初回セットアップで回復キーを表示し、紙に書くなど、端末の外で保管するよう強く促す。**
2. 回復キーはクラウドへ自動で預けない（DEC-086）。
3. パスワード・PIN・回復キーをすべて失った場合、**データは戻せない**ことを、セットアップ時に明確に伝える。

### Q4 — 紛失時のリモート機能

1. **Nagi 1.0では、遠隔での位置確認・データ消去などのリモート機能は入れない。** 紛失・盗難への備えは、保存データの暗号化で行う。
2. 将来リモート機能を検討する場合は、ユーザーが明示的に選んで使う機能として別途決める。Local-first・クラウド不要の方針を既定として維持する。

### Q5 — Profileごとの分離

1. DEC-086の「鍵をUser／Profileごとに分ける」方針を、**Memoryとユーザーデータの暗号化にも適用する。**
2. 1台の端末を複数のユーザー・Profileで使う場合も、他のユーザー・ProfileのMemoryやデータを、その鍵なしに読めないようにする。

## 例外・禁止・安全境界

- 暗号化をオフにしても、MemoryとProfileの暗号化はオフにならない。設定画面でもそのことを示す。
- 暗号化データを復号できたことは、OS上のIdentity／Permission／Owner権限の付与を意味しない（DEC-086）。復号後もNagiのPermission／Policyを通す。
- 回復のための裏口（開発者や第三者だけが使える解錠手段）は設けない。
- AIやアプリに、暗号鍵・回復キーそのものを渡さない（DEC-086）。
- Hibernate（DEC-068）やBackup／Workspace Export（DEC-014／DEC-105）で、暗号化されていないMemory・Profileのコピーを端末内外に残さない方向とする。

## REVIEW NOTE（assistantの整理・未承認）

- **TPM自動解錠の注意点:** TPMによる自動解錠は、盗難時にもログイン画面までは起動できる。ログインの保護（DEC-083、試行回数の制限）と組み合わせて守る前提であり、より強い保護が必要な利用者向けにQ2の起動時PINを用意する。
- **性能への影響:** 暗号化によるストレージ性能とCPU負荷の影響を、DEC-116の仮目標値の測定に含める必要がある。
- **関連DECへの注記:** DEC-088（暗号化が未確定だった点）、DEC-086（Memory・ユーザーデータへの適用）、DEC-099（初回セットアップでの回復キー表示）に、本DECへの日付付き参照注記を追加する必要がある。

## 未確定・暫定案（PROVISIONAL）

- 暗号化の方式（ディスク・ボリューム単位、ファイル単位、両者の組み合わせ）と使う暗号アルゴリズム。DEC-088の用途別Filesystemとの連携。
- 暗号化をオフにする・オンに戻すときの手順、所要時間、途中で電源が切れた場合の扱い。
- 回復キーの形式（文字列、QRコード等）、表示・確認の画面、再発行の方法。
- 起動時PINの桁数・試行制限、TPM等のない機種でのパスフレーズの要件。
- 複数ユーザー・Profileの鍵の階層、ユーザー追加・削除時の鍵の扱い。
- 適用版と実装担当。

## Roadmapとの関連

- 0.2: Identity／Secrets／Storageの土台に暗号化を組み込む。
- 0.3: Personal AgentのMemory・Profileを、常時暗号化の対象として保存する。
- 1.0: リモート機能なしで、暗号化と回復キーによる保護を提供する。

本DECは既存のRelease／Activation Gate、Acceptance条件、Workstreamの所有権を変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-20 | 初版。ユーザーがQ1（ユーザーデータは標準で暗号化オン・オフ可、MemoryとProfileは常に暗号化）、Q2（TPM機は自動解錠＋通常ログイン、非TPM機は起動時パスフレーズ、TPM機でも起動時PINを選択可）、Q3（初回セットアップで回復キーを表示し保管を促す、すべて失うと戻せないと明示）、Q4（1.0ではリモート機能なし、暗号化で守る）、Q5（Profileごとの鍵分離をMemory・ユーザーデータの暗号化にも適用）を確定。方式・UI・数値はPROVISIONAL。 |
