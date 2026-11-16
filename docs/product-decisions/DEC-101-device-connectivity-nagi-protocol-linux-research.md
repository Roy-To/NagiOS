# DEC-101 — Device Connectivity：Bを基本にNagi独自の端末連携プロトコルを設計し、独自通信技術を並行研究

**状態:** CONFIRMED（Foundation #32：Linux／OSS通信技術を利用するNagi統合Connectivity ServiceのB方式を基本採用。PC・Tablet・Phone間で使うNagi独自の連携プロトコルを設計する方向を確定し、必要性が判明した独自通信技術は並行して研究する）／PROVISIONAL（プロトコルの具体Schema／Version、接続・暗号方式、独自通信技術の採否、端末・提供版・受入条件）\
**最終合意日:** 2026-11-16\
**適用候補:** Nagi OS Foundation #32。実装開始条件、Release割当て、Linux target／実機対応は未決定\
**関連:** [Foundation Checklist #32](NAGI_OS_FOUNDATION_CHECKLIST.md#f32)、[DEC-066（LAN-first共通接続）](DEC-066-device-connectivity-lan-first.md)、[DEC-064（Nagi Share）](DEC-064-nagi-share-nearby-file-transfer.md)、[DEC-065（Nagi Handoff）](DEC-065-nagi-handoff-work-continuation.md)、[DEC-014（Workspace移行／同期）](DEC-014-workspace-state-checkpoints.md)、[DEC-043（Linux＋Nagi Userland）](DEC-043-nagi-userland-independent-os.md)、[DEC-052（実機テスト候補）](DEC-052-physical-test-device-candidates.md)、[Roadmaps](roadmaps/README.md)

## 目的とユーザー体験

PC／Tablet／Phone等のNagi端末が、Nagi Shareの単発ファイル転送やNagi Handoffの意味的な作業継続を安全に実現できる共通の端末連携基盤を持つことを目指す。端末間の体験・データ契約はNagiが設計する一方、発見・接続・暗号化Transport等の成熟したLinux／OSS技術を活用し、基礎通信一式を全面自作する過大な開発・保守負担を避ける。

## 確定した製品方針（CONFIRMED）

1. **Bを製品の基本方式とする。** Linux／OSSの利用可能なNetwork・Transport・端末発見等を選択的に利用し、Nagi独自のDevice Connectivity ServiceがDiscovery、Pairing、Device Trust、Session、接続と取消、Transport差異を共通に管理する設計方針を採用する。具体的OS Component／Protocol／Service実装はまだ選ばない。
2. **Nagi独自の端末連携プロトコルを設計する。** Nagi Share／Handoff等の上位体験が通信Transportの種類に縛られないよう、端末能力や利用可能機能の表現、対象・操作・応答／結果・Revision／状態の授受、Protocol Version／互換など、NagiのDevice／Continuity／Public API／Permission体系に整合する連携契約を設計する。何を通信するかはNagiが定めるが、具体メッセージ形式・API Schema・互換性表は後続設計で決める。
3. **通信方式と上位のNagi連携契約を分離する。** 初期は既存の[DEC-066](DEC-066-device-connectivity-lan-first.md)の同一Wi-Fi／LANターゲットを維持し、将来の近距離直接通信や遠隔接続にTransport Adapter等を追加できる方向を保つ。Transportが変わっても利用者の明示許可、Peer Trust、User／Profile、Resource権限が抜けない構成を目指す。
4. **必要な独自通信技術を並行研究する。** 既存のLinux／OSS技術だけではNagi固有の要求を満たせない可能性のある部分について、必要性・性能・Security・携帯端末の制約・実装／保守負担を検証する研究を進める。全面独自通信Stack／暗号方式／無線Driverの開発を本決定で標準採用するわけではない。研究の成果を製品へ正式採用するかは検証後の別判断とする。
5. **既存のShare／Handoff／Workspace契約を変更しない。** Shareは承認された単発ファイル共有、Handoffは認可された作業継続、DEC-014はWorkspaceの選択式移行・同期と標準の暗号化Export／Importを各々維持する。連携プロトコルを共有しても、ShareがHandoffの必須Transportとなる、常時同期が既定で始まる、ファイル実体が必ず届く、Clipboard同期が採用されると解釈しない。
6. **安全な認証・認可とOffline-firstを維持する。** Peerの認証・信頼登録・暗号化、送受信側の明示許可、User／Profile／Owner／Permission／Privacy／Capability／Policy、失効・取消・Source／Revision再確認を既存DECどおり区別する。暗号化済み接続やNagi独自Protocolへの対応だけで操作権限やCloud送信・Data移行許可を与えない。端末発見・接続・送信の基本機能にLLMを必須とせず、ローカル連携を常時Cloud運用に依存させない。

## 設計上の分担（方向はCONFIRMED、技術形式はPROVISIONAL）

| 層 | 基本方針 | 未決定 |
|---|---|---|
| **Nagi連携体験** | Share／Handoff／将来の選択式Continuity等、各機能とその認可をNagiが設計する | 共有範囲・UI・アプリ連携の細部 |
| **Nagi端末連携Protocol／Service** | Device Capability・機能選択・Resource／Version／Session／結果の意味的な交換をNagi共通契約として設計し、Transportから分離する | 具体メッセージ／API Schema、互換・Version交渉、Serviceの境界 |
| **Discovery・Trust・Transport** | Linux／OSSの成熟した技術を優先的に検討し、NagiのPolicy・Session／Peer Trustと結線する | Discovery／Pairing／暗号／接続・失効の具体方式、利用するOSS |
| **独自通信技術の研究** | 標準技術では不足する要求がある箇所を限定して調査・測定し、必要性が示された場合のみ採用判断する | 対象、Benchmark、脅威評価、採否・利用範囲、検証機器 |

## 例外・禁止・安全境界

- **独自Protocolの決定 ≠ 独自Transportや自前暗号方式の採用。** 独自連携ProtocolはNagi固有の意味的連携契約であり、信頼できる基礎暗号・通信方式の選択を妨げない。自作暗号や独自無線Driver等の採用は本決定に含まれない。
- **Device Trust ≠ データ操作の許可。** Peer登録、発見、接続、Protocol交渉の成功だけでFileの閲覧・転送、Workspaceの復元・同期、遠隔Action、Profile／Secretへのアクセスは許可されない。
- **配信・移行・同期・Task再実行の境界を維持。** 新たなConnectionによりデータが自動で渡るわけではなく、未確認の転送完了・対応可能性を成功として表示しない。中断したAI Taskや副作用Actionは自動再開／二重実行しない。
- **ハードウェアの制約を無視しない。** Nagi独自通信契約があることを理由に、全機器でのWi-Fi Direct／Bluetooth／遠隔通信や、全機種への互換性・性能を保証しない。

## 未確定・暫定案（PROVISIONAL／OPEN）

- 上位Protocolの型、Message／Session／Error／Version交渉Schema、Service Boundary、既存Public API／Device／Continuityとの責務と整合方法。
- 初期LANでの発見・Pairing・Identity／Trust／暗号方式、対象Discovery／Transport OSSの採否とライセンス・Security維持方法。
- 同じ端末の複数User／Profile、Capability差、権限失効・再Pairing、異種CPU／OS／端末UIでのVersion互換、秘密・データの保持／削除範囲。
- 中断・復帰・再送・競合・結果不明・Rate／Buffer／Resource制限、通信経路が変わる際の継続／失敗表示、Nagi Share／Handoffとの詳細結合。
- 独自通信の研究対象と採用判断に使う測定条件（遅延・接続成功率・RAM／電力・安全性・保守工数）、代替OSSとの比較、正式採否。
- 近距離直接通信／VPN／Internet／Relay／Online Account／Cloudの採用・運用主体・費用、対象機器、対応Version／開発時期、実機受入条件。

## Roadmapとの関係

DEC-066の「同一Wi-Fi／LAN開始・後続Transport追加可能・初期Cloud／Remote非必須」を製品前提として維持する。0.2の基盤、将来のPC／Tablet／PhoneとShare／Handoff／ContinuityがこのNagi独自Protocolを利用する方向とするが、今回の仕様追加は特定Releaseでの実装・Handoff成功・全端末対応を確定するものではない。既存のRelease／Activation／Acceptance GateとDesktop firstは変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーはFoundation #32について、全面独自通信の開発負担を考慮し、Bを基本採用、Nagi独自の端末連携Protocolを設計し、必要な独自通信技術を並行研究する方針を明示承認。具体Protocol／Transport・独自通信の正式採用・提供機器・版／測定値は未決定。 |
