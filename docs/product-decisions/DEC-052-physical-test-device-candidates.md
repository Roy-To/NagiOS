# DEC-052 — 実機テスト端末の候補選定方針

**状態:** CONFIRMED（候補の選定方針のみ）／PROVISIONAL（具体構成・対応範囲・実装時期）

**最終合意日:** 2026-11-16

**適用候補:** 将来の実機テスト（版・実施時期は未確定）

**関連:** [DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-021](DEC-021-roadmap-0.5-1.0.md)、[DEC-042](DEC-042-linux-kernel-transition.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[実機・複数端末の展望](../roadmap/multi-device-outlook.md)、[roadmaps](roadmaps/README.md)

## 目的と位置付け

将来のNagi OS実機テストに向けて、ユーザーが選んだ端末候補を記録する。確定するのは候補の選定方針だけであり、実装済み・起動確認済み・正式対応という記録ではない。

## 確定した選定方針（CONFIRMED）

| 区分 | 実機テスト候補 | 選定範囲 |
|---|---|---|
| PC | Raspberry Pi | 具体型番は未指定。Raspberry Pi 5の選定を意味しない |
| タブレット | PineTab2 | 将来の実機テスト候補に含める |
| スマートフォン／モバイル | 未選定 | 機種は今後検討する |

PCという区分は今回のテスト用途の分類であり、Raspberry Piのアーキテクチャをx86-64と見なすものではない。

## 既存仕様との整合・適用境界

- [0.1実装仕様](../Nagi_OS_0.1_Codex_Implementation_Spec.md)の正式対象はQEMU x86-64のVirtual Reference Machineであり、実機対応を0.1完了条件に追加しない。
- [0.2実装仕様](../Nagi_OS_0.2_Codex_Implementation_Spec.md)のQEMU x86-64主対象、最初の実機参照機を選定したx86-64 UEFIミニPCとする方針、generic ARM64を経てRaspberry Piへ進む境界を変更しない。同仕様のRaspberry Pi 5は従来の候補であり、今回の型番確定ではない。
- [DEC-017](DEC-017-third-party-developer-ecosystem.md)のPC／Desktop firstと将来Mobile／Tablet向け移植支援方針、[DEC-021](DEC-021-roadmap-0.5-1.0.md)および各ロードマップの版境界・既存Gateを維持する。候補追加から実装順序・版割当て・Mobile開発開始を推定しない。
- 実機検討にもDEC-042/043のLinuxカーネル + Nagi専用ユーザーランド方針を適用する。既存Linuxディストリビューションや端末付属OSの起動をNagi OSの起動実績と見なさない。
- [実機・複数端末の展望](../roadmap/multi-device-outlook.md)に残る旧「Raspberry Pi 5を最初の物理ARM64対象」「Pixel 9aを第一候補」という対象選定は、本DECの範囲で置き換える。スマートフォンの現在の選定状態は未選定であり、Pixel 9aを選定済み候補として引き継がない。
- [VISION-001](VISION-001-human-adaptive-os.md)の高水準ビジョンとは区別し、実機候補の正本は本DECとする。

## 未確定・検証待ち（PROVISIONAL / OPEN）

- Raspberry Piの具体型番・構成、PineTab2の試験構成、スマートフォン／モバイルの機種。
- 購入・調達・所有状況。候補の記録は購入決定ではない。
- 起動可否・起動実績、ドライバー、画面・入力・通信・電源管理等の動作、性能、互換性と正式対応範囲。本DECは実機試験結果を追加しない。
- boot構成、移植方法、試験項目・受入条件、担当、実装時期、テスト時期、対応版、提供・リリース時期。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーの意思決定によりPC＝Raspberry Pi（型番未指定）、タブレット＝PineTab2、スマートフォン／モバイル＝未選定を記録。CONFIRMEDは選定方針のみとし、旧展望の機種選定を限定置換。 |
