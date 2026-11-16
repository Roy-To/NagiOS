# DEC-081 — Nagi主体の基本ユーザーランドと汎用OSSの選択的活用

**状態:** CONFIRMED（Nagi固有部分は独自設計・汎用libc／ライブラリ／基礎ツールはOSS活用・既存distributionユーザーランド一式は流用しない）／PROVISIONAL（具体コンポーネント・libc／ABI・ビルド・実装・適用版）
**最終合意日:** 2026-11-16
**適用範囲:** Nagi OS Foundation Checklist #5「基本ユーザーランド」（検討束B1）、Linuxカーネル採用後の製品構成方針
**関連:** [DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-069](DEC-069-foundation-checklist-management.md)、[DEC-080](DEC-080-init-service-manager-ab-evaluation.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[Roadmaps](roadmaps/README.md)

## 目的

Nagiが独立したOSとして自らのユーザーランドと公開契約を設計・保守する一方で、汎用技術の不必要な再実装を避け、開発速度・品質・移植可能性を確保する。既存ディストリビューションの完成したユーザーランド全体を土台にしてUIのみを置き換える構成とは区別する。

## 確定した製品方針（CONFIRMED）

1. **Nagi固有のサービス・API・アプリ環境は独自に設計する。** Nagiが固有の機能・権限・サービス契約・Nagi Native Apps向け公開API／SDK・ユーザー体験の設計主体となる。すべてのコードを新規に自作するという意味ではない。
2. **汎用コンポーネントにはOSSを積極的に活用できる。** libc、基本ライブラリ、基礎ツール等は、適合する既存OSSを評価して採用する方針とする。Rust実行環境などの一般的な技術も対象となり得るが、今回個別の採用は決定しない。
3. **既存Linuxディストリビューションのユーザーランド一式は製品の基盤として流用しない。** 完成済みrootfs、サービス／パッケージ体系、Desktop環境を一体で引き継いでNagiの外観だけを載せる方式は採らない。個別OSSの利用、開発ホストとして既存Linuxを利用することまでは禁止しない。

この決定は[DEC-043](DEC-043-nagi-userland-independent-os.md)の既存製品方針を、基礎コンポーネント選択に具体化したものである。

## 方針が適用される領域

| 領域 | 確定した高水準方針 | 未確定の技術判断 |
|---|---|---|
| Nagi Service／Permission／Public API／SDK | Nagi固有の契約と管理をNagiが設計 | 内部実装、Linux adapter、プロセス分離、SDK実体 |
| Nagi Native App環境 | [DEC-017](DEC-017-third-party-developer-ecosystem.md)のNagi Native Apps標準を維持 | ABI／runtime／package形式、移植支援方式 |
| libc／基本ライブラリ／Rust関連runtime | 適合する個別OSSを積極的に検討・活用 | musl／glibc／relibc等の候補、選定理由、採用版・patch |
| shell／基礎tool／診断 | Nagiの基本操作・開発／診断に必要な範囲を提供し、汎用部分はOSS利用可 | コマンド一覧、nshなど既存資産の再利用、機器別同梱範囲 |
| filesystem layout／rootfs／ABI | Linux基盤とNagi固有のサービス／アプリ契約に合わせて構成 | 具体layout・filesystem・syscall境界・POSIX範囲・build方式 |

## 既存決定と安全境界

- [DEC-043](DEC-043-nagi-userland-independent-os.md)のLinuxカーネル＋Nagi専用ユーザーランド・既存distribution基盤非採用と矛盾しない。Nagi独自開発の対象を無条件にlibcなどへ拡大しない。
- [DEC-080](DEC-080-init-service-manager-ab-evaluation.md)のA案第一候補・B案との短期比較方針は維持する。今回の基本ユーザーランド方針から、Nagi独自PID 1や特定OSS Initの**正式採用を推定しない**。
- [DEC-017](DEC-017-third-party-developer-ecosystem.md)のNagi Native Apps標準、Rust SDK中心・必要な安定C ABI、既存OSアプリの無変更バイナリ互換を標準搭載しない境界を維持する。libc等のOSS利用はLinuxデスクトップアプリの一般的な直接実行を約束しない。
- 従来の`user/libnagi/`、`user/nagi-pal/`、relibc／POSIX等の旧独自kernel用資産・仕様は再利用候補として評価する。旧カーネル用のsyscall／ABI／security contractをLinux上に無条件で移植しない。
- OSSの採否は、ライセンス・保守・出所・更新・実機動作・権限境界への適合を個別に評価する。単なるライブラリの採用によってNagiのPermission／Capability／AI非特権やUser／Profile／Resource境界を緩めない。
- 0.1／0.2／0.3以降のRelease／Activation／Acceptance Gate、正式対応端末・採用時期・実装開始の追加許可を本記録で変更しない。Linux imageや実機の起動・機能受入を確認済みにしない。

## 未確定事項（PROVISIONAL／OPEN）

- libcの具体的選定、Rust std／C ABI／POSIXのLinux上での実装・サポート範囲。
- 最小のrootfs・基礎ライブラリ・管理ツール・shellの構成、OSに初期搭載するOSSの具体リスト。
- 依存部品・供給元・revision／patch／ライセンスと脆弱性更新・継続保守責任。
- Nagi PAL／relibc／libnagi／nshを再利用・置換する範囲、移行コスト・実機上の互換性。
- 対応端末・ファイルシステム配置・配布方式・リリース割当て・実装条件・受入結果。

## 技術設計への引渡し（REVIEW NOTE）

最小Linux boot→Nagi userland→Nagi serviceの開発経路に必要なコンポーネントを洗い出し、候補OSSとNagiが責任を持つ抽象契約を区別して実装・検証する。特定のlibcを先に確約するより、再現可能なbuild・必要なABI・権限の強制・ソフトウェアライセンス・保守負担を実証して選択するのが望ましい。これはユーザーが詳細な技術スタックや検証日程を確定したことを意味しない。

## Roadmapとの関連

現在の高水準の製品方針を確定するもので、0.2／0.3／将来版のロードマップ・Gate・実装済み状況を変更しない。Foundation B1の#1〜#5は仕様検討として一巡するが、個別技術設計・実装・実機Acceptanceや#4のA/B最終選択は別作業である。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #5について「Nagi固有のサービス・API・アプリ環境は独自設計し、libc・基本ライブラリ・基礎ツールはOSSを積極活用する。ただし既存Linuxディストリビューションのユーザーランド一式は流用しない」と承認。具体libc等は未決定とした。 |
