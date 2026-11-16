# DEC-089 — Nagi Volume ManagerとLinux Mount／Hotplugの責務分担

**状態:** CONFIRMED（Foundation #13 B案：Nagi独自Volume ManagerがVolume発見・論理識別・認可・安全な取り外し／切断時の状態管理を担当し、Linuxのblock／mount／hotplug機構を活用する）／PROVISIONAL（自動mount条件・初期対応媒体／Filesystem・Volume ID方式・safe eject手順／受入・適用版）
**最終合意日:** 2026-11-16
**適用範囲:** Foundation Checklist #13「Volume / Mount」（検討束B3）。Linuxカーネル＋Nagi専用ユーザーランドでの内部／外部Volume、Removable Mediaの発見・接続・切断・アクセス管理
**関連:** [DEC-079 System Image／User Data](DEC-079-system-image-readonly-ab-data-separation.md)、[DEC-088 用途別Linux FilesystemとNagi Storage API](DEC-088-linux-filesystem-nagi-storage-api.md)、[DEC-020 非特権AI／Permission](DEC-020-ai-os-interaction-path.md)、[DEC-023 作業・データ中心](DEC-023-work-data-centric-os.md)、[DEC-025 分散正本／Resource参照](DEC-025-cross-app-data-reference-and-updates.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[0.1実装仕様 §16–18](../Nagi_OS_0.1_Codex_Implementation_Spec.md)、[0.2実装仕様 §10・23](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[純正Files仕様 §51](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[Roadmaps](roadmaps/README.md)

## 目的

USBメモリ・外付けSSD等を接続した際に、Nagi Files／Settings／アプリから適切に発見・利用・取り外しできる体験を提供する。Linuxのデバイス検出・block・mountの低レイヤーを利用しつつ、ユーザーに見せるVolumeの論理Identity、アクセス権限、利用／取り外しと障害状態の責務はNagi側に持つ。

## 確定した製品方針（CONFIRMED）

1. **B案：Nagi Volume ManagerとLinux標準機構を組み合わせる。** Linuxのblock／mount／hotplug等で媒体やFilesystemを扱い、Nagiの正規Volume ManagerがVolumeの発見・表示・利用準備・認可・mount管理・取り外し／切断状態を統括する。NagiアプリやAIがLinuxのmount先を勝手に管理しない構成とする。
2. **接続・取り外し・障害をNagiが把握する。** 接続された媒体を検出し、正規Files／Settings等から確認できるようにする。自動検出は、無条件の自動mount・全媒体のread/write許可・未知の内容の自動実行を意味しない。
3. **Volumeの論理識別をデバイスパスから分離する。** Linuxのデバイスパスやmount先が変更されても、識別情報が信頼できる範囲で同じ媒体／Volumeとして扱える設計を目指す。ただし、clone／reformat／ID競合・変更等における恒久的同一性を保証せず、不確実な場合は同一と黙って判断しない。Nagi Resource／Object IDとVolume IDも混同しない。
4. **User／Profile／App／Taskの現行Permissionと所有権を維持する。** 接続したVolumeの利用・参照・変更・ejectはNagi Identity／Session／Permission／PolicyとLinuxの隔離を通して評価する。接続しただけでAIやアプリが他のProfile／外部Volumeを無制限に読めるようにしない。通常操作は正規Storage／Files／Volume APIを経由する。
5. **安全な取り外しを支援する。** 使用中のファイル／未完了の書込み・保存待ちやmount状態を確認し、必要な書込み完了／flush・mount解除等を正規経路で調整する。完了を確認できない場合に「安全に取り外せる」と表示しない。処理の待機・停止／再試行・利用者への通知／判断は実際の状態とデータ保全を優先する。
6. **突然の切断・故障を正確に扱う。** 保存済み・部分成功・失敗・結果不明や利用不能の状態を区別し、Volumeが見えない間も以前のHandle／認可／Resource参照を無条件に有効としない。再接続で中断Task／副作用Action／不確かな書込みを勝手に再実行せず、現在のIdentity／Permission／Resource Revision・Volume同一性を再検証する。

> **2026-11-16 Persistence後続決定:** [DEC-090](DEC-090-common-persistence-contract-distributed-authority.md)により、保存完了・未保存・部分成功・結果不明の共通Persistence契約をNagiが担うB案をCONFIRMED。本DECのsafe eject／予期しない抜去・再接続では、実際のdurable write／App保存状況を区別して報告する。flush・unmount・既存Handle／二重実行防止の具体手順とLinux実媒体受入は未決定。

## 安全境界・既存正本との整合

- **DEC-088:** Linux標準Filesystemの用途別選定とNagi Storage／Files APIを維持。本DECはVolumeの発見・mount／取り外し・媒体切断の管理を追加するものであり、FSの種類・暗号化方式・実際の保存耐久性を確定しない。
- **DEC-079:** System Imageのread-only運用、User Data分離、A/B更新／Recovery、Model Store論理分離を維持。Volume管理導入でSystemへの通常書込みや任意媒体からの信頼されないOS更新を許容しない。
- **DEC-020／023／025:** 現行Caller／Profile／Permission、Providerの分散正本、Resource ID／Revision／Provenanceを維持。Volume接続・ID所持・Files表示は、参照許可、Owner／System Capability、AI Action実行の許可を意味しない。
- **FilesystemとTrust:** 接続媒体・Filesystemのparseやmountは信頼境界。Linuxの保護機構／隔離を活用し、悪意ある／破損した媒体を信頼済みと扱わない。具体的なread-only mount・隔離・スキャン・許可UIは後続設計。
- **保存と再接続:** Filesystem-levelのflush／eject成功、アプリの未保存編集、Workspace Checkpoint／Wayback、データバックアップはそれぞれ別の保証。#14 Persistenceでdurable commitの契約を具体化する。
- **既存仕様:** 旧独自kernelの固定Storage構成とmount案は参考資産であり、Linux generic hotplug／複数Volume／安全な取り外しが既に実装された証拠ではない。
- **Release:** 0.1／0.2／0.3以降のRelease／Activation／Acceptance Gate、担当・着手条件・提供端末・版は変更しない。旧guestやhost-only試験をLinux target／外部実媒体のPASSに読み替えない。

## 未確定事項（PROVISIONAL／OPEN）

- 最初に対応する内蔵／外付け／USB等のVolume種別、Filesystem互換、read-only／read-write条件、暗号化外部Volumeの解除、初期mount／自動mount条件。
- Linuxのdevice／hotplug通知とVolume ID・Provider ID／Nagi Resource IDの具体的な対応、同名／複製Volume・媒体交換・hotplugの競合解消。
- Linux mount namespace、mount対象／権限・所有者、同一Volumeを複数User／Profileが使う場合のScope、信頼できない媒体の隔離と認可UI。
- Safe Ejectの書込み完了・open handle・使用中App／Taskの通知・停止・unmount／power-offの具体順序、待ち時間／失敗・強制切断の扱い。
- 接続断時の未保存／部分書込み・古いhandle／retry・再接続／Volume再識別、実機／Linux targetの耐障害・データ保全試験、対応版。

## 技術設計への引渡し（REVIEW NOTE）

代表的なLinux targetと実媒体で、(a)接続・発見とNagi Filesへの表示、(b)同一Volumeの再接続・device path変更／ID衝突、(c)認可されたUserと無権限User／App／AIからのアクセス拒否、(d)書込み中のeject要求・使用中Handle・flush失敗／unmount失敗、(e)予期せぬ抜去・媒体破損・再接続時の再検証・重複実行防止を分けて検証する。Linuxのmountが可能であることとNagiのVolume管理が完成したことを混同しない。

## Roadmapとの関連

B3 #13の高水準製品決定としてFiles・Device Manager・Storage／Persistence／Recoveryへ接続する。具体的なLinux driver／mount実装や対応Filesystems／端末の決定、Linux実機Acceptance、提供版・新しいRelease blockerは今回承認しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #13「Volume / Mount」でB案を承認。Linux block／mount／hotplugを利用しNagi独自Volume Managerが発見・論理識別・権限・安全な取り外しと切断時状態を統括する。具体mount方針・対応媒体・安全な取り外し手順・Linux target実装／試験・版は未確定。 |
