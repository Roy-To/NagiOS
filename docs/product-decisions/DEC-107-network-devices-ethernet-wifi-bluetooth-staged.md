# DEC-107 — Network Devices：Ethernet／Wi-Fiを基本、Bluetoothを段階的に対応

**状態:** CONFIRMED（Foundation #18の方式B：Ethernet・Wi-Fiを基本機能とし、Bluetoothは段階的対応）／PROVISIONAL（実機・chipset・network stack・Bluetooth対象・認証・省電力・実装版・受入）  
**最終合意日:** 2026-11-16  
**適用候補:** Nagi OS Foundation #18。特定のReleaseは未確定  
**関連:** [Foundation Checklist #18](NAGI_OS_FOUNDATION_CHECKLIST.md#f18)、[DEC-106](DEC-106-device-manager-local-remote-unified-ui.md)、[DEC-066](DEC-066-device-connectivity-lan-first.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-068](DEC-068-nagi-sleep-resume-recovery.md)、[Roadmaps](roadmaps/README.md)

## 目的とユーザー体験

Nagi OSで日常利用する有線・無線ネットワークの基本機能を揃え、Bluetoothについては接続先・実機・用途の幅を考慮し段階的に対応する。

## 確定した仕様（CONFIRMED：方式B）

1. **EthernetとWi-Fiを基本機能とする。** Nagi OSの一般的なネットワーク接続手段として、有線Ethernetと無線Wi-Fiの双方を基本の製品対象に含める。Ethernetのみを初期の唯一の製品対象にしない。
2. **Bluetoothは段階的に対応する。** Bluetoothは製品対象から除外しないが、ペアリング、接続・再接続、入力／音声／データ等の用途や機器対応を一括で初期要件にせず、段階的に整える。
3. **Linux機器管理の基本方針を継承する。** DEC-106のLinux既存driver活用と、Nagi側のDevice Service／UIによる機器・状態・Permission管理方針の下で検討する。個別技術や実装手順は未採用とする。
4. **LAN上のNagi端末連携とは責務を区別する。** DEC-066の「Nagi端末同士の初期接続ターゲットは同一Wi-Fi／LAN」は維持するが、Wi-Fiを基本機能とすることだけでBluetoothの具体方式、端末間の自動ペアリング、同期や遠隔接続を採用したことにはしない。

## 例外・禁止・安全境界

- 対応driverやradioが存在することと、すべてのEthernet／Wi-Fi／Bluetooth chipsetの正式サポート・動作保証は区別する。
- ネットワーク資格情報、pairing情報は既存Secrets／Permission／Privacyを守り、端末を検出・接続しただけでデータ転送やAI Actionを許可しない。
- ネットワーク再接続やスリープ復帰は現行権限・Trust等を再検証し、接続失敗・未対応機器を成功と表示しない。
- 既存Release／Activation／Acceptance Gate、QEMU／Linux target・実機受入の条件を変更しない。ネットワークの実装完了は別途検証する。

## 未確定事項（PROVISIONAL／OPEN）

- 最初の有線／Wi-Fi chipset・driver・firmware対応と実機試験、radio制御・省電力。
- Wi-Fiの認証方式、資格情報保存、接続設定・再接続・失敗時UI。
- Bluetoothの初期対応範囲（入力・音声・データ等）、pairing・暗号化・認証・解除の具体方式。
- Linux側network stack／service採否、Nagi UI／Permissionとの境界、sleep後の復帰・接続不能への対応。
- 適用release、導入順序、対応機種、性能目標、target acceptance。

## Roadmapとの関連

Foundation #17（Device Manager）、#23（電源）、#24（Network Service）、#32（Device Connectivity）、#33（Hardware Support）と連動する。現時点で「基本製品対象」とすることは初期Releaseの必須出荷条件・全機器対応の保証ではない。0.2／0.3／将来Roadmapと既存Gateを維持する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #18のB方式を明示承認。Ethernet・Wi-FiをNagi OSの基本機能とし、Bluetoothは段階的対応とする。具体stack／driver／対応端末／Bluetooth機器用途／適用版は未確定。 |
