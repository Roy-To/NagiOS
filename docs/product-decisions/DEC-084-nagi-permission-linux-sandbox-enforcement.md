# DEC-084 — Nagi Permissionを正本とするLinux Sandboxの強制隔離

**状態:** CONFIRMED（NagiのPermission／Capability判定とLinux低レイヤー隔離の組合せ・迂回防止・最小権限／失効／狭窄・非特権AI）／PROVISIONAL（具体的Linux強制技術・Policy／sandbox設計・各環境での受入／版）
**最終合意日:** 2026-11-16
**対象:** Nagi OS Foundation Checklist #8「Permission / Sandbox」（検討束B2）。Linuxカーネル＋Nagi専用ユーザーランドでのアプリ／サービス／AI実行の権限判定・実効的な隔離
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-083](DEC-083-nagi-identity-linux-credentials-binding.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[0.2 Platform Foundation仕様](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[移行ADR](../decisions/2026-11-12-linux-migration-development-scope.md)、[Roadmaps](roadmaps/README.md)

## 目的

Nagiの正規API上で許可しない操作が、同一ユーザーの通常プロセスやLinux syscall／device／filesystem等の直接経路から実行できてしまう構成を防ぐ。製品のPermission／Capability／Policyの意味と判断責務をNagi側に保持し、Linuxの利用可能な保護機構を組み合わせて実効的な隔離を成立させる。

## 確定した製品方針（CONFIRMED）

1. **NagiがPermission／Capabilityの判断主体となる。** User／Profile／Session／App／Task／Resourceの現在のIdentity・Scope・Policyに従い、データ、デバイス、ネットワーク、サービス／OS機能などの許可を評価する。一般アプリ権限、Platform Capability、System Capabilityは混同しない。
2. **Linuxの保護機構による実効的な隔離を組み合わせる。** Nagiの正規API／Providerが認可を行うだけでは十分とみなさず、アプリ・AI・背景Job・必要なサービスの実行環境に、Linux側の隔離やアクセス制御を適切に適用する。許可されていない対象への直接ファイル／デバイス／ネットワーク／syscall等の経路が抜け道にならないようにする。
3. **必要最小限の権限、取消・失効・狭窄を維持する。** 権限が不足する場合は拒否または正規の許可手続を用い、Task・委譲・Session等のScopeを超えて権限を増やさない。実行中の失効・取消や新しいResource Revision／所有権状態に対応し、過去の許可を無条件に引き継がない。
4. **AI／通常アプリ／Background Jobには特権を自動付与しない。** ログイン済みであること、ユーザーと同一のLinux UIDであること、AIによる提案・学習・GUI操作が可能であることを、OwnerまたはSystem Capability付与の根拠としない。必要な特権操作は正規の権限境界・人間による認証／認可および信頼されたサービス経路で評価する。
5. **権限内の操作を不必要に再確認しない。** 既に承認されたTask・対象・Purpose／Scope・現行Permission／Policy内の操作では、既存DECに従って冗長な確認を避ける。確認を省くことと、必要な認可検証やLinux隔離を省くことは別である。

この方針は「B案：Nagiの権限管理＋Linuxによる強制隔離」の採用を意味する。Linuxの特定の仕組みや全コンポーネントのSandbox設計の完成を確定するものではない。

## 既存仕様との関係・安全境界

- **DEC-083 Identity:** Nagi UserId／ProfileId／SessionIdが論理Identityの正本。Linux UID／GIDは保護のために活用するが、同一UIDは異なるNagi Task／App／Profileへの無制限アクセス権限ではない。
- **DEC-020／027:** 型付きPublic Action／Service APIを優先し、限定的なGUI／Accessibility fallbackも同じPermission／Capability／PolicyとTask Scopeに従う。画面やLinux APIへ直接アクセスできることはAuthorityではない。
- **DEC-001:** 同意済みTask内の可逆な下位操作は冗長な承認なしで行えるが、Data Policy／Provider／Action時点の権限検証とStop／Cancelは維持する。学習・繰返し・推奨から権限を増やさない。
- **DEC-017:** Nagi Native Apps専用の製品方針と純正／第三者の共通Public API／Permissionを維持する。Linux Sandboxの採用をLinux一般アプリの無変更互換の承認としない。
- **DEC-043とLinux移行ADR:** Nagiの製品権限モデルとLinux kernel credentials／capabilitiesは別の層。Linux root／UID／GIDだけでNagi Permissionの同等性が証明されたと扱わず、特権serviceの最小化・実際の迂回拒否を設計・検証する。
- **サービス間呼出:** IPC／Service Broker（Foundation #9）もCaller／Sessionの信頼できる結び付けと権限狭窄が必要。ユーザーが送った任意のID文字列だけでCallerを認証してはならない。
- 既存の0.1／0.2／0.3以降のRelease／Activation／Acceptance Gate、Workstream所有・実装開始許可、対象ハードウェア・対応版をこの記録で変更しない。host上でのロジック試験や旧独自kernelのPASSをNagi Linux imageの隔離成立と扱わない。

## 未確定事項（PROVISIONAL／OPEN）

- Linux namespaces、seccomp、LSM、DAC／ACL、cgroups、device policy／broker、filesystem／mount・network隔離等から実際にどれを採用・どう組み合わせるか。例示は技術の正式採用・依存追加を意味しない。
- アプリ／AI／Model Runtime／サービス／背景Job別のTrust Level、sandbox生成と破棄、同一UID下の分離、最小特権service／brokerの配置。
- 具体的なThreat Model、syscall／device／filesystem／ネットワーク／IPCの迂回経路一覧、否定テストと段階的適用・互換性／性能影響。
- 権限取消やlease失効時の既存handle／file descriptor／IPC・Background Jobの処理、正規許可UI／Owner操作の認証条件、Audit／Privacyの具体仕様。
- Linux target・実機での強制テストとAcceptance、実装時期・担当・Release割当て。

## 技術設計への引渡し（REVIEW NOTE）

Linux adapterと実際のSandboxに対し、許可済みのPublic Action／Resourceアクセスは機能することと、正規Providerを迂回した同じResourceへの直接アクセス、cross-Profile／App侵入、許可取消後のアクセス、AI自己権限昇格が拒否されることをセットで検証する。特に同一Linux UIDやserviceに関係する権限の抜け道をThreat Modelとして評価する。実測と未対応範囲を区別すること。

## Roadmapとの関連

0.2のIdentity／Permission／Service契約と、0.3以降のProfile／Agent利用を安全に支える高水準の製品方針。特定版の採用やSandbox実装・性能・対応端末を確約せず、実装・受入・Gate変更を今回の仕様合意と混同しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #8で「Nagiが権限を判断し、Linuxが実際の隔離も担う」B案を明示的に採用。User／Profile／Session／App／Task／Resource別の現行権限、直接アクセス迂回防止、取消・狭窄、AI／アプリの非特権、既承認の操作への冗長確認回避を含む。具体的Linux強制技術・検証条件・適用版は未決定。 |
