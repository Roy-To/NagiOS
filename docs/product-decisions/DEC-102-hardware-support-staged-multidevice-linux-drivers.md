# DEC-102 — Hardware Support：Linux Driverを活用する段階的マルチデバイス対応（B）

**状態:** CONFIRMED（Foundation #33：LinuxのDriver／Hardware機構を活用し、PC・ARM64・Tablet等へ段階的に広げ、機種別の機能・実機検証に基づいて正式対応を判断するB方式）／PROVISIONAL（具体機種・Driver／Firmware Matrix・必須機能・実装・提供Version／Linux target受入）  
**最終合意日:** 2026-11-16  
**適用候補:** Nagi OS Foundation #33。機種の正式製品対応やRelease／Activation Gateの変更を意味しない  
**関連:** [Foundation Checklist #33](NAGI_OS_FOUNDATION_CHECKLIST.md#f33)、[DEC-052（実機候補）](DEC-052-physical-test-device-candidates.md)、[DEC-043（Linux＋Nagi専用ユーザーランド）](DEC-043-nagi-userland-independent-os.md)、[DEC-078（端末別Boot）](DEC-078-boot-platform-entry-and-common-nagi-startup.md)、[DEC-017（Desktop first／Native Apps）](DEC-017-third-party-developer-ecosystem.md)、[0.2実装仕様 §5](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[Roadmaps](roadmaps/README.md)

## 目的とユーザー体験

まず限定された開発・実機環境でNagi OSの信頼できる基本体験を成立させ、PC、ARM64機器、タブレット等へ段階的に展開する。幅広い機種へ早期一斉対応するより、Linuxの成熟したDriverを生かして、機種・Hardware Capabilityごとの実証と安全な製品サポートを優先する。

## 確定した製品方針（CONFIRMED：方式B）

1. **段階的なマルチデバイス対応を採用する。** Desktop／PCを基本に、ARM64機器・Tablet等へ段階的に拡張する。将来のPhone／Mobileも構想として維持するが、今回のBではPhone機種・Version・着手時期を決めない。多数のPC／Tablet／Phoneを同時に早期正式対応するC方式を標準にはしない。
2. **Linux Driver／基礎Hardware機構を活用する。** Boot後のHardware／Driverへのアクセス、Display／GPU・Input・Audio・Network／Radio・Storage・Power等の低レイヤーはLinuxの既存機構と選択したOSSを活用し、NagiのUI、Service、Public API／Permission、Native Apps、Model／Agentは端末固有差から分離できる設計を志向する。個別Driver／Firmware／API・配布方法は未確定。
3. **開発／検証ターゲットと正式な製品対応を分ける。** QEMU／Host上の試験、旧Kernel時代のGuest成功、一般Linuxの起動のみからNagi OSの実機動作・OS機能・製品サポートを主張しない。実際のNagi Linux Image／対象Hardware／Version／Configurationの受入結果を示して初めて、対象範囲の対応可否を判断する。
4. **各機種の機能を検証して正式対応範囲を決定する。** Boot、画面／GPU、Keyboard・Touch等Input、Audio、Storage、Connectivity、Power／Sleep、Security・Recoveryなどは機種別の必須／任意／利用不可のMatrixで確認し、機能ごとの制約と失敗を正確に扱う方向とする。試験項目・必須機能・性能閾値・製品認定規則は後続設計に委ねる。
5. **既存のPlatform移行・機種選定の境界を維持する。** [DEC-078](DEC-078-boot-platform-entry-and-common-nagi-startup.md)のPC UEFI基本・端末別Boot・Linux起動後Nagi処理共通化、[DEC-052](DEC-052-physical-test-device-candidates.md)のRaspberry Pi（型番未定）／PineTab2／Phone未選定と、0.2 §5のQEMU x86-64主対象→選定x86-64 UEFI mini-PC実機Reference、generic ARM64（QEMU aarch64/virt）→Raspberry Pi Portの順序を維持する。候補の選定は購入・起動成功・製品対応ではない。

## 開発Target・候補と正式対応の区別

| 種類 | 現行の位置付け | 未決定 |
|---|---|---|
| QEMU x86-64 | 0.2の主要決定論的開発／試験対象 | Linux移行後の実際の合格・既存受入との差分 |
| x86-64 UEFI mini-PC | 0.2で最初の実機Referenceとして選ぶ機器区分 | 実際の機種／構成・Driver・正式対応・Linux受入 |
| generic ARM64 | 先にQEMU aarch64/virt等でArchitecture差を検証 | 機能・性能・機種別差分・製品対応 |
| Raspberry Pi | DEC-052の将来実機テスト候補。型番は未選定 | Board別Boot／Driver／GPU等、正式なPort結果 |
| PineTab2 | DEC-052のTablet実機テスト候補 | Touch／画面／電源／無線等の受入とVersion |
| Phone／Mobile | 長期構想の対象で、実機候補未選定 | 対応機種・Radio／Telephony・時期・受入 |

Raspberry Piの「PC」区分はDEC-052の試験用途分類であり、x86-64やUEFI搭載と見なす意味ではない。上表は製品対応済みリストではない。

## 例外・禁止・安全境界

- 0.2で任意PC、全GPU／Wi-Fi／Bluetooth／Sleep機能、Raspberry Pi／Tabletの幅広い互換性をRelease条件にしない。既存の0.1／0.2／0.3／0.4+ Release／Activation／Acceptance Gateは維持する。
- LinuxにDriverが存在するだけで、当該Nagi機器の動作・Firmware可用性・Security・性能・License条件を満たすとは推定しない。未検証は未検証、未対応は未対応として示す。
- Device Capability・入力方式や機器固有Firmware／Driverの差による制約を、Nagi共通Serviceの権限昇格・未認可データアクセス・AIによる勝手な操作の根拠にしない。
- Desktop／PC firstと将来Mobile／Tablet対応を区別する。Androidや既存Linuxの一般的アプリ実行互換を本DECから新規に採用しない（DEC-017）。

## 未確定事項（PROVISIONAL／OPEN）

- 対象機種／SKU、正式Linux Reference機、必須Device／Driver／Firmware Matrix、起動信頼・更新・Hardwareの認定方法。
- 画面／GPU加速、Keyboard／Touch、Audio／Mic、Storage、Wi-Fi／Bluetooth／Cellular、Sleep／Power／Thermal等の機種別必要条件、性能・安定性・Securityの合格閾値。
- Linux Board Support／Driverの取り込み、Adapter設計、Firmware／Libraryの出所・更新保守・License、Hardware故障時の診断・Recovery。
- ARM64／Raspberry Pi／Tablet／Phoneの具体的機種、検証順序・開発担当、対応予定版・ReleaseとLinux実機受入結果。

## Roadmapとの関係

0.2 §5とDEC-052の検討境界を維持し、PCから他のArchitecture／Form Factorへ段階的に展開する製品方針のみ確定する。既存Milestoneと追加Release機能を結び付けず、DEC-052の端末候補や0.2の旧実装・QEMU試験をLinux実機PASSへ読み替えない。全35項目の設計検討完了を新たなGateにはしない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | Foundation #33について、ユーザーがLinuxのDriverを活用する段階的なPC／ARM64／Tablet等への拡張と、機種別受入で正式対応を決める方式Bを明示承認。対応機種・Driver Matrix・実機実績・版／Gateは別途検証と判断に委ねる。 |
