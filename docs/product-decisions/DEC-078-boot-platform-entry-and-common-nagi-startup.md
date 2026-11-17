# DEC-078 — 端末別Boot方式と共通Nagi起動処理

**状態:** CONFIRMED（PCのUEFI基本・端末別Boot方式許容・Linux起動後のNagi処理共通化）／PROVISIONAL（具体Boot構成・検証・適用版）
**最終合意日:** 2026-11-16
**適用範囲:** Nagi OSのBoot System（Foundation Checklist #2／検討束B1）。対応機種・製品Release・移行時期は未確定
**関連:** [DEC-042](DEC-042-linux-kernel-transition.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-052](DEC-052-physical-test-device-candidates.md)、[DEC-069](DEC-069-foundation-checklist-management.md)、[DEC-077](DEC-077-linux-kernel-lts-stable-policy.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[Roadmaps](roadmaps/README.md)

## 目的と基本方針

Linuxカーネル＋Nagi専用ユーザーランドを、UEFI対応PCだけでなく、Raspberry Piやタブレット等の異なる起動環境にも展開可能な形で設計する。プラットフォーム固有の起動入口と、Linux起動後に実行するNagi固有の処理を分離する。

## 確定した仕様（CONFIRMED）

1. **PCではUEFIを基本とする。** 標準的なUEFI対応PC向けのBoot経路はUEFIを基本に設計する。すべての機器にUEFIを必須とする意味ではない。
2. **端末に適したBoot方式を許容する。** Raspberry Pi、タブレット等では、ファームウェアやハードウェアに適した起動方式を採用できる。UEFI非対応機器にUEFI方式を無理に適用しない。
3. **Linux起動後のNagi処理を共通化する。** 端末ごとに異なり得るfirmware／loader／kernelへの引渡し部分と、Linux起動後のNagi専用init・サービス等の処理を分け、後者に共通する契約・処理を維持する方針とする。

これはBoot Systemの高水準方針であり、機種を問わず同一バイナリで起動できる、すべての端末で同じdriver／init設定を無変更で使える、という保証ではない。

> **2026-11-16 システム信頼基盤の後続決定:** [DEC-087](DEC-087-tiered-system-trust-and-signature-verification.md)で、対応機器はHardware-backed Boot Trustを活用し、非対応機器もSoftware署名検証するB案をCONFIRMED。UEFI対応PCのUEFI基本／Raspberry Pi・タブレット等の機器別Bootという本DECの方針を維持する。具体Secure Boot／署名方式／Trust Anchor・rollback保護／対応実機とLinux target受入は未決定。

## 既存決定・安全境界との関係

- [DEC-042](DEC-042-linux-kernel-transition.md)／[DEC-043](DEC-043-nagi-userland-independent-os.md)のLinuxカーネルとNagi専用ユーザーランド、既存ディストリビューションを製品OS基盤にしない方針を維持する。
- [DEC-052](DEC-052-physical-test-device-candidates.md)の実機候補（Raspberry Piは型番未指定、PineTab2は候補、スマートフォンは未選定）を変更しない。同DECでRaspberry Piを「PC」区分としたのは実機テスト用途の分類であり、本DECでそのBoot方式をUEFI限定にはしない。
- 旧0.1のUEFI loader、署名付きslot manifest、A/B起動、rollback等は既存実装・設計の証拠として保持する。Linux向けの同一実装・互換性や、その仕組みの無条件採用を本決定では認めない。
- 起動の信頼境界、権限、更新、障害回復について既存決定を緩めず、Linux上の実際の強制方法と受入証拠を別途定義する。
- Release／Activation／Acceptance Gate、Linux移行開発の担当・操作許可、特定版への機能割当てを変更しない。仕様記録は起動成功・実機対応済みを意味しない。

## 未確定事項（PROVISIONAL／OPEN）

- PC側の具体的bootloader・UEFI連携、QEMUのLinux Boot入力、各端末のfirmware／bootloader／Device Tree／ACPI等の採否・構成。
- Linux kernelへの引渡し方式、initramfs／rootfs、Nagi init・サービス起動の技術詳細、デバイス固有の起動差分を隔離するインターフェース。
- 起動イメージ署名、Secure Bootやtrust chain、A/B system slot、障害時の回復起動・rollback、失敗検知・診断のLinux向け具体方式と受入条件。
- 開発用の最初のtarget、端末・機種ごとの適用範囲、対応時期、起動性能と実機試験の実績。Raspberry Pi／PineTab2でのNagi Linux起動は未確認。
- 旧独自kernel用UEFI loader／boot trust実装の再利用・置換・adapter範囲。Linux用の具体ADRや実装契約は後続設計に委ねる。

## 技術設計に渡すメモ（REVIEW NOTE）

最初の開発targetでfirmware→loader→Linux kernel→Nagi init→Nagi serviceの実経路を検証し、プラットフォーム別部分と共通部分の境界を記録する。失敗条件・入力の固定・復旧・署名等の安全設計は別のADR／Acceptanceで検証し、旧カーネルのguest起動実績をLinux target PASSへ流用しない。

## Roadmapとの関連

0.2／0.3／将来版の既存ロードマップ・リリース境界を変更しない。どの機種で何をいつ提供するかは、個別機種の技術設計・実機検証と別途のユーザー決定を要する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #2について「PCはUEFIを基本とし、Raspberry Piやタブレットなどは機器ごとの起動方式を許容。Linux起動後のNagi処理を共通化する」方針を確認。具体bootloader・信頼／復旧方式・機種対応・適用版は未決定とした。 |
