# DEC-077 — LinuxカーネルのLTS標準採用とStable先行利用

**状態:** CONFIRMED（LTS標準・Stable先行利用許可）／PROVISIONAL（具体版・運用／更新詳細）
**最終合意日:** 2026-11-16
**適用範囲:** Nagi OSのLinuxカーネル管理方針。対象リリース版・対応端末・正式切替日は未確定
**Foundation Checklist:** #1 Linuxカーネル管理／検討束B1
**関連:** [DEC-042](DEC-042-linux-kernel-transition.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-044](DEC-044-linux-migration-development-authorization.md)、[DEC-069](DEC-069-foundation-checklist-management.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[Roadmaps](roadmaps/README.md)

## 目的

Linuxカーネル採用とNagi専用ユーザーランドの既存決定を前提に、製品の標準系列と、新機能・ハードウェア対応を先行検証する開発上の選択肢を区別する。独自カーネルへの方針回帰や、既存Linux移行の受入完了を意味しない。

## 確定した仕様（CONFIRMED）

1. **LTS系列を標準採用する。** Nagi OSの製品用Linuxカーネルは、長期保守系列（LTS / longterm）を標準とする。
2. **最新Stable系列の先行利用を許可する。** 開発・実験・新しい機能やハードウェアへの対応可能性を調べる先行検証では、LTS以外の最新Stable系列を選択して利用できる。
3. **標準採用と先行利用は別の判断とする。** Stable系列を開発で利用できることは、当該系列を製品の標準カーネルへ自動昇格させる決定ではない。製品版への採用や切替は別途判断する。

上記はカーネル系列の**方針**であり、Linuxの特定バージョン／パッチレベル、ハードウェア対応や性能を確約する決定ではない。

## 維持する既存境界

- [DEC-042](DEC-042-linux-kernel-transition.md)のLinux採用、[DEC-043](DEC-043-nagi-userland-independent-os.md)のNagi専用ユーザーランド／既存Linuxディストリビューションを製品基盤にしない方針を維持する。
- [DEC-044](DEC-044-linux-migration-development-authorization.md)と移行ADRの限定開発許可・可逆的な技術選択を維持する。開発中の固定入力や個別担当の操作許可を無断変更しない。
- 従来の0.1／0.2／0.3以降のRelease／Activation／Acceptance Gate、実装権限、Linux targetの証拠要件を本決定で変更しない。
- 先行利用は技術検証の許可であり、既存実機・ドライバ・ユーザーデータを無条件に変更する許可ではない。

## 未確定・暫定事項（PROVISIONAL）

- 標準採用する具体的LTS系列・正確なkernel revision／patchlevel、製品版や端末別の採用時期。
- Stable先行利用の対象環境・選定基準、LTSへ反映する機能の採用／バックポート条件、正式系列切替の判断手順。
- LTSのサポート期限確認、保守責任者、脆弱性修正の追跡・更新頻度・緊急対応期限、製品向け配布手順。
- kernel config、独自patch／module、driver、再現可能ビルド、供給元・ライセンス・更新テストの具体的な管理方法。
- LTS／Stableの性能・省電力・起動・復帰・周辺機器対応の実測結果。特定系列の優位性は未検証。

## 技術検討メモ（REVIEW NOTE）

今後の設計では、採用系列とは別に実際にビルドした完全なrevision／config／patchを追跡し、LTSとStableの検証結果を混同しないことが望ましい。これはユーザーが更新周期や管理ツールまで確定したことを意味しない。

## Roadmap・実装との関係

各Roadmapの現行版割当ては変更しない。開発版／正式製品版のどの時点で系列を切り替えるかは、Linux移行・boot／system image・hardware support・build／supply chainの設計と受入証拠に基づき別途決める。記録済みの方針を、実装・Linux image起動・実機受入PASSと同一視しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #1について「LTS系列を標準採用」と明示的に決定。続いて「LTS標準＋Stableの先行利用を許可」と確認。LTS製品標準とStable開発／先行検証の併用方針をCONFIRMEDとして記録し、具体版・更新運用・実装／受入は未確定とした。 |
