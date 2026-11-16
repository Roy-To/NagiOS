# DEC-098 — Diagnostics / Logging：Nagi統合診断Bを基盤に、AI支援診断Cを段階導入する

**状態:** CONFIRMED（Foundation #29：BのNagi統合Diagnostics／Loggingを製品の基本とし、CのAI支援診断を将来の正式な製品目標として段階導入）／PROVISIONAL（Linux収集backend・AI診断モデル／推論方式・ログ保持量・具体解析・提供版・実機受入）
**最終合意日:** 2026-11-16\
**適用候補:** Nagi OS Foundation #29。特定Releaseへの新規必須割当てと実装開始の承認は行わない\
**関連:** [Foundation Checklist #29](NAGI_OS_FOUNDATION_CHECKLIST.md#f29)、[DEC-020（AIとOSの決定論的な境界）](DEC-020-ai-os-interaction-path.md)、[DEC-022（中断Taskの復旧）](DEC-022-builder-interrupted-task-recovery.md)、[DEC-026（証拠付き変更・復旧）](DEC-026-ai-change-verification-and-recovery.md)、[DEC-002（Cloud送信許可）](DEC-002-cloud-ai-data-sharing.md)、[DEC-003（情報分類）](DEC-003-memory-classification.md)、[DEC-043（Linux＋Nagiユーザーランド）](DEC-043-nagi-userland-independent-os.md)、[0.2実装仕様 §17／M04](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[Roadmaps](roadmaps/README.md)

## 目的・ユーザー体験

Nagi OS自身とアプリのboot／crash／service health等について、原因調査と安全な復旧判断を支える診断情報を、AIに依存しない統一された基盤から確認できるようにする。その上で将来、許可された診断情報と実際に確認できた状態に基づき、AIが原因の候補、確認の方法、対処案を説明してユーザーを支援する体験を目指す。

## 確定した製品方針（CONFIRMED：B基盤＋段階的C最終目標）

1. **製品基本構成はB（Linux基盤＋Nagi統合診断）。** Linuxの利用可能なクラッシュ／サービス・プロセス／システム情報を適切なOS境界で収集する。Nagi独自のDiagnostics Serviceが構造化Event、severity／category、health、関連付け、crash情報、source／build fingerprint、snapshot／reportと診断上の状態を統合管理する。Linuxの詳細な収集component・daemon・sink選定は別判断。
2. **CのAI支援診断を将来の正式な製品目標にする。** Bの信頼できる収集・閲覧・報告基盤を先行させ、CではPermissionと情報区分で許された診断情報に基づいて、原因候補・不確実性・根拠・追加確認方法・対処方法をユーザーに提示する方向へ段階導入する。Cは単なる研究候補ではなく将来的な製品目標。ただし自動修復機能の採用や提供時期を今回確定しない。
3. **事実・観測・仮説を分離する。** 実際に記録・照合できるEvent、確認された障害・状態・検証結果と、AIの推定原因／修復候補を明確に区別する。AIの説明、自己申告や推測のみで診断が確定した、修復が成功した、既存Actionが未実行である等と扱わない。情報が不足する場合は「不明」「根拠不足」「未検証」を表示できるようにする。
4. **基本の診断はAIなしで成立させる。** ログ収集・異常状態の検出・health照会・既存の報告／診断表示は、LLM推論やCloudを必須としない。AIが使えない場合も通常のDiagnostics、障害情報と手動の問題切分けを利用できる。
5. **ログとActivityと権限を混同しない。** Debug／Privileged Diagnostics、ユーザー向けActivity／Task履歴は目的・所有・閲覧権限を分け、ユーザーに意味のある事象のみ明示的なBridgeを通す。ログが存在すること自体はAI／App／Userがその全内容を参照できるPermissionではない。診断情報のアクセス、AIへ渡すContext、Export／共有／Cloud送信、診断から実行するActionは、それぞれ現行Permission／Capability／Policyと必要な承認に従う。
6. **Privacy・安全・資源境界を維持する。** 診断へのSecret／Credential／秘密入力の無用な記録・複製を防ぎ、Redactionと保持・容量・Rateの上限、障害に耐えるsinkを重視する。ログの常時無制限収集、個人情報の無差別なAI投入、無断の外部診断アップロードは認めない。診断サービス自身の不調で通常のOS機能が不用意に停止する構造を避ける。
7. **修復は正規のActionと復旧規則に従う。** AIによる原因候補・修復提案を実行権限と同一視しない。設定変更、再起動、サービス更新、データ操作、Task再開等は正規のAPI・現在のScope・Permission・Policy・承認・復旧可能性確認の内側で行う。中断Task／不確かな非冪等Actionを診断結果だけで無断再実行しない（DEC-022／026）。

## 製品導入段階（方向はCONFIRMED、実装・版はPROVISIONAL）

| 段階 | 役割と目標 | 未確定の具体化 |
|---|---|---|
| **B — 決定論的な統合Diagnostics** | Linuxの実Event／healthをNagiの共通観測・Redaction・Snapshot／Reportに統合し、AIなしでも障害調査と事実確認を可能にする | Linux backend、Log schema・sink、Retention、export UI、実機対象 |
| **C-1 — AIによる分析支援** | 許可された証拠から可能性のある原因と確認手順を説明し、実測事実と推測を分けて提示する | Model、Prompt／Context範囲、Evidence ID、説明品質と不確実性の評価 |
| **C-2 — 修復提案の改善** | 原因候補に対応した、検証可能で安全な対処案や復旧選択肢を提示する将来の体験 | 対処Action範囲、ユーザー確認UI、リスク分類、検証・Recovery・自動修復の採否 |

この段階区分は製品方向の整理であり、各段階のRelease割当て、特定モデルの採用、AI自動修復の承認、性能効果の達成、Linux target受入を意味しない。

## 既存仕様の継承

- **0.2 §17／M04：** structured Events、bounded metadata、severity／category、redaction、crash capture、service／app health、snapshot／report、correlation IDs、source/build fingerprint、Activityへの明示的な橋渡し、sink failure toleranceを維持。M04のhost acceptanceをLinux targetの成功証拠と扱わない。
- **DEC-020：** AIは非特権。診断Contextの取得と修復Actionは正規Service／Public API、Scopeと最新Permission／Policyで決定論的に扱い、全ログの無差別な投入を標準にしない。
- **DEC-022／026：** 実際の証拠と検証で成功／失敗／部分成功／結果不明を区別する。診断と復旧、Taskの復旧提案と正式な再実行を区別する。
- **DEC-002／003：** 診断・学習・AI支援を口実とした未承認のCloud転送、情報分類の解除、他User／ProfileのSecret開示を認めない。

## 未確定（PROVISIONAL／OPEN）

- Linux側の具体的なjournal／trace／crash sink等の採用、collection／Redaction／保存・閲覧・exportのSchema／ACL・責務、保存件数／容量／期間とfailure／offline時動作。
- 最小boot／service証拠、source／build fingerprintの対応、Correlation／Snapshot／Data Provenance、ログ消失や時計ずれがある場合の説明。
- AIに渡す診断Contextと個人情報の分離／Redaction、ローカル推論・Cloud利用の条件、Model選択、推論負荷・AIがない場合のUI。
- 原因分析・修復提案の証拠評価、誤診・誤通知・説明可能性、確認手順の検証、ユーザーの採否・Feedback／監査、結果不明の表現。
- 修復提案から実際の修復実行に移る製品要件・リスク・明示承認、診断によるOS変更の可否。自動修復の採用は未決定。
- 研究・実装計画、owner、デバイス、Linux target実機受入、提供Version、性能・保護合格閾値、Release／Activation Gate変更の有無。

## Roadmapとの関係

0.2のDiagnostics基盤を保持し、将来のPersonal Agentによる説明・問題切分け・修復支援と段階的につなげる。#29のB基本構成とC最終目標の選定は、特定版への自動割当て、既存の検証・Release／Activation／Acceptance GateやWorkstream実装許可を変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | Foundation #29について、ユーザーが「Bを基本に、CのAI支援診断を将来の正式目標として段階的に導入する」方針を承認。Linux情報を用いたNagi統合Diagnosticsを基本とし、証拠に基づくAI分析・修復提案を将来の製品目標とする。個別backend、AI方式、提供時期、自動修復の採用は未確定とする。 |
