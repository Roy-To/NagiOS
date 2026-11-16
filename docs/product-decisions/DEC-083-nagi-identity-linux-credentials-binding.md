# DEC-083 — Nagi Identityを正本とするLinux資格情報との連携

**状態:** CONFIRMED（Nagi独自User／Profile／SessionをIdentityの正本とし、Linux UID／GIDは低レイヤー保護に利用・ログイン等をNagiが管理・Profile単位の分離・AI権限非昇格）／PROVISIONAL（ID対応方式・認証技術・Session lease／失効条件・Linux強制機構・実装版）
**最終合意日:** 2026-11-16
**適用範囲:** Nagi OS Foundation Checklist #7「Identity / Session」（検討束B2）。LinuxカーネルとNagi専用ユーザーランドにおけるIdentity／Sessionの製品境界
**関連:** [DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-022](DEC-022-builder-interrupted-task-recovery.md)、[DEC-068](DEC-068-nagi-sleep-resume-recovery.md)、[DEC-082](DEC-082-power-lifecycle-coordination.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[0.2 Platform Foundation仕様](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)

## 目的

Nagi OSの利用者・Profile・SessionをLinuxプロセス資格情報のみに還元せず、同じ端末の複数利用者やAI Memory／Workspace／Taskの分離をNagiの正規サービスで成立させる。Linuxカーネル側の保護機構は利用しながら、Nagi固有の論理Identityと権限判断を正本として保持する。

## 確定した製品方針（CONFIRMED）

1. **Nagi Identityを正本にする。** Nagi独自の`UserId`／`ProfileId`／`SessionId`を利用者・Profile・セッションの論理Identityの正本とする。Linux PID／UID／GID、ファイルパス、プロセス再生成などによって、これらの論理IDを代用・黙って変更しない。
2. **Linux UID／GIDは低レイヤーの保護に活用する。** NagiのIdentityとLinuxのUID／GIDなどを、必要なプロセス／ファイルアクセスの強制機構に対応付ける。Linux UID／GIDそのものをNagiのUser／Profile／Sessionや最終認可の正本とは扱わず、両者の境界を明示する。
3. **ログイン・ログアウト・画面ロック・復帰の認証／権限確認をNagiが管理する。** 正規のNagi Identity／Sessionと既存Permission／Policyに従い、失効した／ログアウト済み／ロック中のセッションの権限を無条件に使い続けない。ロック解除・復帰を本人確認や新しい権限付与と同一視しない。
4. **利用者／ProfileごとにデータとAI状態を分離する。** ユーザー別ファイル・設定・Workspace・Memory／学習状態・AI Taskへのアクセスは現行User／Profile／Sessionと許可に従う。同じOSやLLMモデルを複数ユーザーが利用しても、利用者のMemory・Context・実行権限を混ぜない。
5. **Owner／Standard User／Guestを維持し、将来のTeam Profileへ拡張可能にする。** ローカルアカウントをクラウドアカウントなしで利用できる基本方針と、Guest／一時Sessionの境界、将来の複数Profile／Team／DeviceのIdentityとの分離可能性を保つ。将来のTeam／Cloudアカウントの本番実装や同期を今回承認するわけではない。
6. **AI・アプリにOwner権限を自動付与しない。** 有効なログインやSessionがあってもOwner操作／System Capabilityは自動付与されない。強い操作は既存の人間の認証・認可／委譲・Permission／Policyで評価し、AIや背景Jobが自分で権限を強化できない。

上記は**Nagi Identity／Sessionの高水準の責務分担**を定めるものであり、パスワード・認証器・実際のUID割当・暗号化方式・Session token形式や具体的実装の採用を意味しない。

## 既存正本・機能境界

- **0.1実装仕様 §22–24:** Owner／Standard User／Guest、完全ローカルアカウント、認証と認可の分離、Owner常時rootの否定、AI自己昇格禁止を継承する。
- **0.2実装仕様 §8–10・M08:** `UserId`／`ProfileId`／`SessionId`等の安定した論理ID、Guest／ephemeral Session、Session ownership／lease、Profileごとのアクセス制御、Permission／Capabilityの狭窄・拒否を維持する。Linux側への実際の接続と同等の強制力は別途検証する。
- **0.3改訂ロードマップ:** Profile／Memory／LearningのUser／Profile境界、将来のDeviceId／TeamId等へ結び付け可能な構造を継承する。ロードマップにある提供範囲・時期を今回昇格しない。
- **DEC-068／082:** 画面ロック・スリープ復帰・電源遷移の既存方針を保持する。LinuxでresumeしたことをNagiのSession認可やTrustの再検証成功と同一視しない。
- **DEC-020／022:** AI・Agentを非特権とし、Action時は現行User／Profile／SessionとPermission／Resource Revision等を評価する。電源断・障害・再起動後の復旧を理由に中断AI Task／副作用Actionを無断再実行しない。
- Linux側のPOSIX資格情報だけでNagi独自のPermission／Capability／Policyの強制が完成すると扱わない。低レイヤーの強制機構、IPC caller binding、cross-profile情報漏洩・guest cleanup・session revocationの実装と試験はFoundation #8/#9等で具体化する。

## 未確定事項（PROVISIONAL／OPEN）

- Nagi論理IdentityとLinux UID／GID・プロセス・group・namespace等の実際の対応・割当・ライフサイクル、1対多／共有プロセスを使うか。
- 認証方法（パスワード／パスキー／生体認証等）、Credential保存・保護、画面ロックUI／ログインUXと認証強度・再認証条件。
- Session lease・expiry・logout・lock・resume・切替時の失効条件、権限を保持した資源／handles／常駐serviceの扱い。
- Owner昇格のトラスト経路、Guest／ephemeral Sessionデータの実際の消去時期・範囲、複数ユーザー同時利用、Team共有・Device／Cloud account連携。
- NagiのPermission／CapabilityとLinuxの実際のアクセス制御との強制的接続、回避・拒否・再検証試験、対応端末・実装完了・Release割当て。

## 技術設計への引渡し（REVIEW NOTE）

Nagi `UserId`／`ProfileId`／`SessionId`とLinux credential／processの対応表を作り、ログイン・ログアウト・ロック・再開・Guest・期限切れ、プロセス再起動・権限剥奪・cross-profile読み取り拒否の代表シナリオを同じLinux targetで検証する。強い操作にHuman／Ownerの正規承認が必要な経路とAIの拒否経路を明確化する。OS上の正常動作は実測して判定し、旧独自kernel・hostのみのテストをLinux targetのPASSへ読み替えない。

## Roadmapとの関連

既存の0.2 Identity／Session契約と、0.3のProfile／Memory構想を接続する高水準の製品方針であり、0.2のM08／他のMilestone、Release／Activation／Acceptance Gate、0.3機能の必須提供時期を変更しない。開発開始・実装済み・Linux実機の認証／分離受入の証拠ではない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #7で「Nagi独自のIdentityを正本とし、Linux UID／GIDは低レイヤー保護に利用するB案」を承認。ログイン／ログアウト／ロック／復帰の管理、Profile別データ／AI Memory分離、Guest・将来Teamへの拡張、AI／アプリのOwner非昇格を含む。技術方式・失効条件・認証方式・適用版は未確定。 |
