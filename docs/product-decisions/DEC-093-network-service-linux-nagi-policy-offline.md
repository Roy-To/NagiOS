# DEC-093 — Network Service：Linux通信基盤＋Nagi独自Network Service

**状態:** CONFIRMED（Foundation #24の方式B：Linuxの既存ネットワーク基盤を活用し、Nagi独自Network Serviceで通信設定・接続状態・アプリ権限・Offline対応を管理）／PROVISIONAL（具体Linux stack、通信仲介・強制機構、接続・権限UI、性能、版・受入）\
**最終合意日:** 2026-11-16\
**適用候補:** Nagi OS Foundation #24。Release割当て未確定\
**関連:** [Foundation Checklist #24](NAGI_OS_FOUNDATION_CHECKLIST.md#f24)、[DEC-002](DEC-002-cloud-ai-data-sharing.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-066](DEC-066-device-connectivity-lan-first.md)、[Roadmaps](roadmaps/README.md)

> **2026-11-25 参照:** [DEC-101の7/12合意](DEC-101-device-connectivity-nagi-protocol-linux-research.md#nagi-networkconnectivity-b案2026-11-25)。接続管理はNetworkManager、TLSはrustlsを優先評価する。個別採用・API／版・ライセンス・実機性能はPROVISIONAL。Network Serviceの通信認可と、Device ConnectivityのTrustや個別のデータ操作認可は独立して扱う。

## 目的・ユーザー体験

Nagi OSの一般アプリ・Albert・Nagi AI・端末間連携に必要なIP通信等を、Linuxの成熟した通信機構とNagi自身の統合Network Serviceの組合せで提供する。オンライン／オフラインの状態、通信設定と権限・送信先を区別し、既存のOffline-first／User／Profile／Permission／Privacyの製品契約を守る。

## 確定した製品方針（CONFIRMED：方式B）

1. **Linux標準通信基盤を利用する。** TCP/IPなどの一般的なネットワーク処理を全面的にNagi独自実装する方式Cは標準採用しない。Linuxカーネルのネットワーク機能と、必要に応じて適切なOSSのDNS／TLS／接続管理等を選択活用する。ただし個々のOSSや管理daemonの採用は別途判断する。
2. **Nagi独自のNetwork Serviceを設ける。** Nagiユーザーランドが、ネットワーク接続状態、通常の通信設定、オフライン表示・エラー／再接続、アプリ／AI Taskが使う正規のネットワーク管理・認可契約を統合する。具体的なプロセス分割やAPI形状は確定しない。
3. **外部通信は既存の権限境界に従う。** アプリ／Agentによる通信の制御と、Cloud AIや外部サービスへユーザーデータを送る許可は区別し、DEC-002のProvider／Purpose／Resource／Profile／情報区分／期間・Taskと送信時の現行Permission／Policyを維持する。ネットワークにつながることだけではデータ取得・共有・Action実行が認可されたことにならない。
4. **Offline-firstを維持する。** 接続が切れても成立するローカル作業を維持し、通信障害・DNS／TLS失敗・タイムアウトを正常完了と区別する。接続復旧だけで未承認の送信、同期、AI Task／副作用Actionの自動実行を認めない。
5. **端末・機器・通信サービスの責務を分ける。** #18のEthernet／Wi-Fiなどの機器・radio管理、#24のIP等のネットワークService、DEC-066のNagi端末間Connectivity／Trust／Transportは、相互連携するが同一権限として扱わない。
6. **将来の限定的な独自最適化を閉じない。** Nagi端末間接続やAI連携等で特別な実測要求が生じた場合、対象経路・プロトコルの最適化や選択的な独自実装を評価できる。ただし今回の合意は特定の独自プロトコル・高速化効果・採用時期を確定したものではない。

## 安全境界・例外

- Network ServiceのAPIを通ることとLinux側でアプリの通信迂回を防止できていることを同一視しない。実際の強制にはプロセス／sandbox、適切なLinux機構、権限取消時の再評価、target上の検証が必要。具体的方式と達成状態は未確定。
- 通信の権限、Cloud AIへ機密情報を送る権限、Nagi端末を信頼すること、リモートAction実行は独立した判断とする。暗号化や同一LANへの接続だけでData／Actionの権限は生じない。
- 通信経路はAI／LLM推論を必須にせず、AIなしでも通常のネットワーク・ブラウザー・アプリ通信を利用できる方向を継承する。
- Linuxを使うことは既存distributionの完成したユーザーランド／ネットワーク管理構成全体の採用を意味しない（DEC-043）。
- 既存Release／Activation／Acceptance GateとDesktop firstを維持する。仕様の承認はLinux targetや実機での通信・強制試験の成功を意味しない。

## 未確定事項（PROVISIONAL／OPEN）

- Linuxネットワーク設定・管理の具体stack、DNS／DHCP／IPv4／IPv6／TLS／proxy／routing／firewall／certificate更新の責務・実装と採用OSS。
- Nagi Network ServiceのAPI／schema／process境界、アプリ別・目的別・宛先別Permissionの具体粒度、現行認可の強制と監査・失効の方式。
- App sandbox／socket仲介・低レイヤーの直接通信経路を用いる場合の権限強制、Offline／復帰・再試行・通信失敗の具体UIとstate。
- LAN／WAN／VPN／proxy環境、実機NIC・Wi-Fi・Bluetoothとの連携、証明書の信頼・更新とセキュリティ試験。
- 速度・遅延・消費電力・通信断復旧の実測値、適用機器、開発順序、担当、提供Version・実機受入。

## Roadmapとの関係

DEC-020の型付きPublic Action／Permission、DEC-002の外部Cloud送信制御、DEC-066の同一LAN開始・共通Device Connectivity、DEC-043のLinux＋Nagi専用ユーザーランドと整合する。#18のradio／device、#24のNetwork Service、#32の端末間信頼／Transportの責務を区別する。0.2／0.3／0.5–1.0の既存版配置・Gate・担当の変更を伴わない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #24の方式Bを承認。Linux通信基盤＋Nagi独自Network Serviceで通信設定・状態・アプリ権限・Offline対応を統合し、全面独自TCP/IP実装を標準としない。特殊通信経路の限定的最適化の余地を保持。個別stack・強制機構・性能・版・受入は未確定。 |
