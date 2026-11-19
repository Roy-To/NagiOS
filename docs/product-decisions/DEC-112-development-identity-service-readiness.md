# DEC-112 — Linux開発イメージのIdentityサービスとREADY

- 日付: 2026-11-19
- 状態: CONFIRMED（対象開発イメージの構成と進め方）／PROVISIONAL（製品全体の構成と接続仕様の詳細）
- 関連: [DEC-080](DEC-080-init-service-manager-ab-evaluation.md)、[DEC-083](DEC-083-nagi-identity-linux-credentials-binding.md)、[DEC-085](DEC-085-linux-transport-nagi-service-broker.md)

## 決定

- Task24のLinux開発イメージでは、Identityを独立した必須サービスとして起動する。署名済みの起動計画で対象を指定し、実際に起動したIdentityサービスを認証した上で、そのサービスによる実際のREADY応答を確認する。READYは、現在のIdentity所有者と認証済みセッションに結び付く状態の確認を経て発行する。
- この開発イメージのDataとNotesは、現在のPID 1内の所有者処理を使用する。QEMUで認証、IdentityのREADY、Notesの保存、同じDataを使用した再起動後の読戻しまでを確認する。
- 独立Dataサービスの実装は別の開発レーンで並行して進める。上記の開発イメージで正常系を完成させるための前提には含めない。

## 未確定事項

製品全体の必須サービス構成、DataとNotesの最終的な配置、READYの具体的な識別子や通信形式、適用する製品版は今後の判断とする。本決定の実装と検証の結果は、実際の起動・保存・読戻しの結果に基づいて記録する。
