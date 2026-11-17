# DEC-027 — 正規Public Action APIと限定的なGUI fallback

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（既存選択A：正規Public Action優先・明示許可下の限定GUI fallback、PX-12高水準Smart Paste B＋将来Cとの整合）／PROVISIONAL（具体的UI操作方式・fallback許可UI／詳細制御・対象アプリ・性能・適用版）  
**最終合意日:** 2026-11-17（PX-12との整合を追記。従前の方式Aを維持）  
**対象:** Nagi OSが第三者・未統合アプリを含むアプリをTaskの一部として操作する際の製品方針。実装時期は未確定  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-023](DEC-023-work-data-centric-os.md)、[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[Public 0.2 Platform Foundation仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)、[Private 0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)

## 背景と目的

アプリをまたぐ操作では、アプリが公開する型付きAction／Service APIを通常経路にする。API連携に未対応のアプリにも限定した支援を可能にしつつ、画面を操作できることが権限や広範な互換性を意味しない境界を定める。

## 確定した仕様

1. **正規APIを優先する:** 対象アプリまたはOS機能に対応する型付きPublic Action／Service APIがある場合、Nagiは既存のIPC／Provider境界を通じてそれを優先する。OS・アプリ状態の取得と変更に、スクリーンショット認識と座標クリックをNagi-nativeの標準操作経路として用いない。
2. **未対応アプリの限定fallback:** Nagiから利用可能なアプリで正規Action連携が未対応の場合、ユーザーの明示的な許可があり、許可された範囲を限定できるときに限り、GUI／Accessibility等によるfallback操作を認める。Taskの目的・対象アプリ・許可された操作・対象Resource等、実際に許可されたScope内でのみ行う。
3. **権限境界を維持する:** fallbackも通常のPublic Actionと同様に、現行のPermission／Capability／Policy、OS・Provider制約、プライバシー・セキュリティ規則、承認済みTaskのScopeに従う。GUI入力手段が使えること自体はAuthorityを付与せず、これらの境界や必要なTask承認を迂回しない。高リスク操作に既存の別承認が必要な場合、その要件を維持する。
4. **実行と結果を正直に扱う:** 実行前に許可された対象・操作を確認し、可能な範囲で実行後の実状態を検証する。実施した操作と結果をActivity／Taskへ記録し、部分失敗・不確実・不能を成功として扱わず、ユーザーへ正確に報告する。Stop／Cancelを尊重する。
5. **許可・確認できない場合:** 必要なScopeを限定して認可できない場合、操作対象や結果を安全に確認できない場合、またはOS／Providerが許可しない場合はfallbackを行わず、ユーザーの手動操作に戻す。すべての旧式アプリとの互換性や無制限のGUI制御は保証しない。
6. **通常GUIを保つ:** AI連携の有無にかかわらず、OS本体は従来のGUIを維持し、AIが利用できないときもユーザーが通常操作できるようにする。

### `builder`製アプリでのAction API自動提案・GUI両立（DEC-076）

[DEC-076](DEC-076-builder-generated-action-api-integration.md)の選択Cにより、`builder`は新規アプリの用途に応じてNagiから利用すると有益な型付きPublic Action API候補を提案し、必要なものを承認済み開発Task Scope内で実装・検証する。通常のGUI操作は残し、AI連携が不要ならAction実装は必須としない。**Action APIの作成・発見自体は実行・権限付与・新たなTask承認と別**であり、実際のNagi操作は本DECの正規Public Action優先、Permission／Policy、限定GUI fallback、Activity、Stop／Cancelの条件を一切緩和しない。具体的な生成技術・提供時期・Acceptanceは未確定。

## 例外・禁止・安全境界

- Public Actionがある操作に、より広い・隠れた権限を得るためGUI fallbackを選んではならない。
- 画面やAccessibility情報の閲覧可能性を、アプリ内部データ全体の収集許可とみなさない。TaskとPolicyが許す情報・操作に限定する。
- 許可されたTask Scope外の操作、未承認Taskの開始・再開、停止後の継続、重要操作の追加承認の回避を認めない。
- 本決定は0.1 M30、0.2 M21、0.3 Implementation Spec／Activation等の既存Gate、Acceptance条件、Workstream ownershipを変更せず、実装開始・Release activationを許可しない。未公開仕様をPublicリポジトリへ転載しない。

### 2026-11-17 — PX-12アプリ横断Smart Paste体験との整合

**CONFIRMED（追加高水準方針）:** [DEC-056](DEC-056-smart-paste-semantic-transfer.md)のPX-12では、利用できるコピー／構造化転送／連動参照等を案内して本人が選択するSmart Paste Bと、将来的にProject／Task・変更の影響まで扱う高度な横断ワークフローCへの発展方向を確認した。

**従前の選択Aを変更しない:** Nagiによるアプリ操作は型付きPublic Action／Service APIを優先し、未対応アプリへのGUI／Accessibility fallbackはユーザーの明示許可、許可されたTask／対象アプリ／操作／Resourceの限定Scope、現在のPermission／Policy、状態検証、Stop／Cancelを満たす場合に限る。Smart Pasteの通常の非AIな手動コピー操作と、Nagiによるfallback自動操作を混同しない。対応しない形式／Appは利用可能な通常の手動転送へ戻り、Scopeや実結果を確認できないNagi操作は行わない。画面を操作可能であることは新たなAuthorityではなく、Public Actionを避けて権限を増やしてはならない。

**PROVISIONAL／OPEN:** Smart Pasteの最初の対応App組合せ、コピー／参照の既定、具体的な選択UI・Preview・Transfer Schema、fallbackの制御方式／許可UI／対象条件、技術・性能・実装版。既存のfallbackを許す必要条件を未確定へ戻さず、新しい権限・無断更新・Release／Activation Gateを追加しない。

## 未確定事項（PROVISIONAL）

- GUI／Accessibility／UI automationで採用する具体的な操作方式と対象アプリの適格条件。
- 明示許可の提示・Scope設定・取消UI、Task／アプリ／操作／Resource等のScope表現、制御回数・時間等の具体値。
- 画面画像認識を許容する用途・必要条件、Accessibility情報との使い分け、監査・状態検証の実装。
- 対応バージョン、Workstream、Target Runtimeでの受入方法。これらは既存のRelease gateに従って別途定める。

## 関連決定との整合

- **DEC-001:** 承認済みTaskの自律実行は承認された目的・対象・権限範囲内に限り、Permissionや習慣からAuthorityは増えない。
- **DEC-009:** Nagi-first体験でも従来GUIとAIなしの通常OS操作を維持する。
- **DEC-017:** 第三者アプリの統合は公開契約を段階的に利用する方針であり、全アプリの無変更互換性を約束しない。
- **DEC-020:** 型付きPublic Action／Service APIを正規経路とする原則を維持し、本DECが未統合アプリ向けfallbackの製品条件を確定する。
- **DEC-023／025:** 作業・データ中心の連携、分散した正本、アプリ横断の参照・更新方針を維持し、関連付け自体から権限を生じさせない。
- **DEC-026:** fallbackによる変更もリスクに応じた検証・保護、部分結果の正確な報告に従う。

## Roadmapとの関連

本方針は将来の適切な実装仕様へ引き継ぐ製品要求であり、特定リリースへの割当ては確定しない。Private 0.3改訂ロードマップへの参照追加は既存のPublic Action原則を補足するだけで、Implementation Spec gateやactivation順序を変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 思想⑩の選択Aを確定。対応する正規Public Action APIを優先し、未対応アプリでは明示許可と限定Scopeの下でGUI／Accessibility fallbackを認める。権限・承認・検証・Activity・Stop／Cancelを維持し、実態を確認できない場合は手動操作へ戻す。 |
| 2026-11-17 | PX-12の案内型Smart Paste B＋将来の高度アプリ横断ワークフローCをDEC-056で採用。本DECの正規Public Action優先A、明示許可／限定ScopeのGUI fallback、実行検証・Stop／Cancelと通常手動転送を維持。具体UI・Transfer／fallback制御・性能・技術・提供版はPROVISIONAL。 |

## Foundation #28の後続決定（2026-11-16）

[DEC-097](DEC-097-accessibility-foundation-and-ai-native-long-term-direction.md)により、通常a11y基盤の先行整備を確定した。これは利用者向けの操作基盤の方針であり、本DECのAI向けGUI／Accessibility fallbackの許可・Scopeを拡大しない。正規Public Action優先、明示許可、認可・確認、検証とStop／Cancelを維持する。
