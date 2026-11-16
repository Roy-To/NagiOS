# DEC-085 — Nagi独自IPC／Service BrokerとLinux標準Transportの分離

**状態:** CONFIRMED（Foundation #9でB案を基本方針に選択：Nagi独自の型付きIPC／Service API・Service Brokerを維持し、低レイヤー通信にはLinux標準機構を利用する）／PROVISIONAL（実際のLinux transport・Broker配置・wire／IDL実装・将来の独自kernel IPC検討・性能／安全性評価・適用版）
**最終合意日:** 2026-11-16
**対象:** Foundation Checklist #9「IPC / Service Broker」（検討束B2）。Linuxカーネル＋Nagi専用ユーザーランドにおけるプロセス間通信の責務分担
**関連:** [DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-080](DEC-080-init-service-manager-ab-evaluation.md)、[DEC-083](DEC-083-nagi-identity-linux-credentials-binding.md)、[DEC-084](DEC-084-nagi-permission-linux-sandbox-enforcement.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[0.2 Platform Foundation仕様](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[Roadmaps](roadmaps/README.md)

## 目的と判断の背景

Nagiアプリ・AI・OSサービス間の通信は、Nagiが公開する型付きService／Action API、Service Registry／BrokerとNagi Identity・Permissionを基本契約とする。Linux移行後の低レイヤー通信まで新しいカーネル機構として自作すべきかを検討し、性能・権限連携の独自最適化可能性は認めつつも、実測前の全面独自IPC開発に伴う工数・保守負担を避ける方針を選んだ。

## 確定した製品方針（CONFIRMED）

1. **B案「Linux IPC＋Nagi独自の通信契約」を基本とする。** Nagi独自の型付きIPC／Service API・Service Broker・サービス発見・バージョン／エラー／取消などの契約を保持し、プロセス間でデータを運ぶ低レイヤーはLinuxの標準的なIPC機構を選択活用する。
2. **Service APIとTransportを分離する。** Nagiアプリ／サービスの論理API、`UserId`／`ProfileId`／`SessionId`、Permission／Capability、承認済みTask Scope等はLinuxの具体的transportやPID・UIDに直接依存させない。Transport・接続方式の差し替えで公開Service／Public Actionの意味が変わらないようにする。
3. **Service Brokerで信頼されたCallerと現行権限を検証する。** 呼び出し元の検証はLinuxから得られる実際のpeer資格情報等とNagi Identity／Session／Permissionを安全に接続し、単に呼び出し側が申告したIDや権限情報を認証根拠としない。権限の狭窄・失効・取消と、Action時の再検証を維持する。
4. **低遅延・大容量通信に対応できる設計余地を残す。** 小さな制御要求と大容量のデータ転送を区別し、共通APIのままで効率的な共有メモリ等を評価できる構造にする。特定の性能改善・コピー削減・前景優先の達成は実測前に保証しない。
5. **独自Linux kernel IPCを最初から必須としない。** カーネルレベルの独自機構の導入を初期実装の前提にしない。将来、標準Linux IPCで解決できない権限制御・性能・運用上の制約が実測で明らかになった場合、限定的な追加方式を別判断として検討できる。今回、独自Kernel IPC開発を承認・着手する決定ではない。

「Bかな」というユーザーの選択を**B案を基本に進める高水準方針**として記録する。具体的なIPC方式／実装手段の確約や、独自方式への将来変更を永久に禁止する決定とは区別する。

## 機能契約として維持する既存仕様

- **0.1仕様 §9–11:** Channel、Nagi Service Registry、型付きIDL、非同期サービス呼出、制御メッセージと大容量データの分離は設計上の参照資産。ただし独自kernel用Channel syscall・VMO・固定制限をLinux上にそのまま採用すると決めない。
- **0.2仕様 §11／M10:** Service identity/version、operation identity、request/correlation ID、typed request/response、認証済みCaller／Session binding、bounded message、timeout・cancel・explicit failure、version mismatch、capability enforcement、transport-independent contractを維持する。Nagi Linux targetでの適切な受入は別途必要。
- **DEC-020／027:** 型付きPublic Action／Service APIと正規Providerを優先し、GUI fallbackも現行Permission／Capability／Policyの範囲でのみ許される。Transportを変えてもAIの隠れた特権APIを新設しない。
- **DEC-083／084:** Nagi User／Profile／Sessionは論理Identityの正本、Linux UID／GIDは低層の保護。Nagi Permission／Capabilityの判断とLinux Sandboxによる強制隔離を併用し、Brokerを迂回したLinux直接アクセスによる権限拡大を許容しない。
- **DEC-043／080:** Linuxカーネル＋Nagi専用userlandとInitのA第一候補／B短期比較を維持する。IPCのB案採用を根拠にInitのB案正式採用を推定しない。

## 具体技術の候補と未確定事項（PROVISIONAL）

- **初期Transportの候補:** Unix Domain Socketを有力候補として検討する。Linux peer credentialの取得、fd受渡し、memfd／shared memory等の利用可能性を実機／targetで評価する。候補名は製品仕様の固定ではなく、他のLinux IPC機構との比較は可能。
- Service Brokerのプロセス配置、Service Registry、接続・起動／再起動・再発見、Nagi IDLの実際の生成／wire encoding、ABI／version移行方針。
- Caller→Linux credential→Nagi Identity／Sessionへの真正なbinding、委譲・失効・既存handle／fdや非同期処理への反映、複数profile／guestの分離。
- サイズ上限、large data／ゼロコピー可否、async／cancel／timeout／replay／結果不明・再送と重複実行防止、障害・再起動時の接続復旧。
- Linux Sandbox迂回・詐称・version不一致／malformed request、性能／CPU／メモリ／latency、前景優先・AI Service負荷での実測、対象端末、担当、適用版・Release／Activation／Acceptance Gate。

## 技術設計への引渡し（REVIEW NOTE）

Linux開発targetで小さなNagi ServiceをNagi Broker経由で呼び、同一APIのままTransportを交換可能な境界を検証する。少なくとも正規Callerと偽装Caller、許可と拒否、権限失効・取消、バージョン違い、bounded messages、大容量受渡し、サービス再起動、Task／Actionの重複防止を分けて試す。低レイヤーIPCの独自開発を始める必要性は、標準IPCでの実測ボトルネックや強制不能な要件が確認されてから再評価する。Host-only試験や旧kernelのChannel ABI成功をNagi Linux imageのAcceptance PASSと扱わない。

## Roadmap／実装との境界

これはNagiのService／IPC高水準方針の記録であり、個別Linux transport選定・実装済み・target受入・製品リリースの確定ではない。0.1／0.2／0.3以降のRelease／Activation／Acceptance Gate、既存のLinux移行開発権限・Workstream ownerを変更しない。#9を理由にFoundation全体へ新たな一律着手Gateを設けない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがA案（Linux kernel側まで独自IPC）固有の利点・負担を比較した後、「うーん、Bかな」と選択。Nagi独自のService／Action API・Brokerを維持しLinux標準IPCを活用するB案を基本方針に置く。具体transport（Unix Domain Socket候補）、詳細方式、実測性能や適用版はPROVISIONALとする。 |
