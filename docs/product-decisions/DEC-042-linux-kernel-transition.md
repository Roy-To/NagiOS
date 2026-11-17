# DEC-042 — 独自Nagi KernelからLinuxカーネルへの方針転換

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（Linuxカーネル採用への方針転換）／PROVISIONAL（構成・互換方式・移行計画）  
**最終合意日:** 2026-11-12  
**対象:** Nagi OSの今後のカーネル基盤。適用バージョン・切替時期は未確定  
**保存先:** Private `Roy-To/NagiOS-internal/docs/product-decisions/`

> **2026-11-12 追加確定:** [DEC-043](DEC-043-nagi-userland-independent-os.md)により、Linuxカーネル + Nagi専用ユーザーランド、既存ディストリビューションのOS基盤非採用、個別OSSの選択的利用可能、独立OSとしてAndroid／ChromeOSに対抗する製品目標がCONFIRMED。以下の初回記録中の「ディストリビューション／ユーザーランド未承認」は当時の承認範囲の履歴であり、この範囲ではDEC-043が優先する。具体的な基本ユーザーランド構成、互換方式、Package形式、移行時期、ライセンス詳細はPROVISIONAL。現行実装・Gateは変更しない。

## 決定の根拠と目的

ユーザーは参照会話「独自カーネルを比較検証」で「方針転換する。Linuxカーネルにしよう」と決定し、2026-11-12の本依頼で「独自Nagi KernelからLinuxカーネルに変更する」という確定決定として記録するよう明示した。本記録は本依頼と提供された会話プレビューのユーザー発言に基づく。省略された過去のassistant回答の内容を承認事項として補完しない。

独自カーネルの先進性そのものを目的にせず、Nagiの製品体験を実現する基盤としてLinuxカーネルへ方針転換する。独自カーネル固有の利点が十分見極められていなかったというユーザーの判断を背景とする。「すべての既存仕様がLinux上で無変更で実現できる」「性能・安全性・移植工数の優位性が実証済み」とする決定ではない。

## 確定した決定（CONFIRMED）

1. **カーネルの選択を変更する。** Nagi OSのカーネル基盤は、独自Nagi KernelからLinuxカーネルへ変更する方針で確定する。Linux採用自体を単なる比較候補へ戻さない。
2. **製品判断と実装切替を分ける。** 本DECは方針の正本であり、既存0.1等のコード・実装仕様・ADR・Gateが移行済みであることを示さない。実装の切替は別途、構成・移行設計・承認・受入証拠を必要とする。
3. **承認範囲を限定する。** ディストリビューション、ユーザーランド、既存アプリとの互換方式、移行時期・適用版は今回承認されていない。下記PROVISIONALから採用済みの詳細を推定しない。
4. **記録先はPrivateとする。** Public `Roy-To/NagiOS` に未公開DEC・ロードマップ・本記録の抜粋を転載しない。公開向け説明・仕様変更は公開範囲と内容を別途承認する。
5. **現行ゲートは変更しない。** 0.1 M30、0.2 activation / M21、0.3 Implementation Spec / activation、0.4 Entry Gate / Integration Owner checkpoint、既存Workstream所有権・Acceptanceを本記録で変更・有効化しない。0.2 runtimeは引き続きM30 PASS・明示的0.2 release境界・対象WorkstreamのIntegration Owner checkpointを要し、host foundationからtargetへの昇格では別のtarget受入証拠を要する。現行実装の即時停止・削除・凍結や、Linux移植Workstreamの開始も本依頼から推定しない。

## 旧方針との関係と整合性確認（REVIEW NOTE）

以下は2026-11-12に確認した既存文書との照合である。ユーザーの確定決定と、必要となる技術改訂・将来提案を区別する。

| 参照元 | 整合性・変更範囲 |
|---|---|
| DEC-017 開発者環境 | 「独自カーネルの強みを活かす」というカーネル選択の前提だけを本DECがSUPERSEDEDとする。AI coding優先、Rust SDK / C ABIの製品方針、段階的Public API統合、portable codeの再利用、1.0開発者・配布目標は撤回しない。LinuxバイナリやGUI toolkitの無変更互換性は引き続き未承認。 |
| DEC-020 AIとOSの操作経路 | 「独自カーネルから設計する戦略上の選択」だけを本DECがSUPERSEDEDとする。構造化Context / Event、型付きPublic Action、決定論的な認可・実行、非特権AI、通常GUI、前景応答性優先は維持する。Linux上の具体的な強制機構は未確定。 |
| DEC-001 / 013 / 014 / 023 / 027 | Task承認・Scope、Canonical Memoryの所有・由来・忘却、Workspaceの保護・選択的移行・現行権限再評価、作業・データ中心の体験、限定GUI fallbackを撤回しない。カーネル変更だけでデータ移行完了・権限継承・全面互換・Task自動再開を認めない。 |
| DEC-007 / DEC-021 | REVIEW NOTEとCONFIRMED／PROVISIONALの区別を保つ。既存版境界・Gateを維持し、Linux移行を特定版へ割り当てない。 |
| Private 0.3 Revised Roadmap | Profile / Memory / Learning / Model portability、単一Personal Agent、Public Action、offline / Permissionの製品方向は引き継ぐ。Section 26および34の仕様化・activation境界は変更しない。DEC-013の正式Memory Engineを本決定だけで0.5以降へ延期しない。 |
| Private 0.4+ Roadmap | 初期Nagi単体のbuilder Role、Scope・証拠・Stop・委譲狭窄の方向は引き継ぐ。Section 2 Entry Gate、初回Role workflow受入と後続専門Agentの別Gateを維持する。Linux採用をAgent Team開始許可にしない。 |
| Private 0.5–1.0 Master Roadmap | 高水準の1.0品質・開発者／配布目標と既存製品境界を引き継ぐ。版別範囲・順序はPROVISIONALのまま。Linux移行の時期・番号・1.0公開日を新たに確定しない。 |
| Private / Public AGENTS、公開0.1 / 0.2実装仕様 | 「not Linux-based」「production Linux runtime禁止」「Nagi Kernel」等の現行実装前提と、新しい製品方針には明示的な差分がある。本DECを理由に現行実装規則を無断で外さない。公開可能な変更範囲、適用版、移行ADR・仕様・Gateを別途整合させてから対象実装を切り替える。 |
| 公開ADR-0001 / 0002 / 0005 / 0007 / 0010等 | hybrid kernel、channel IPC、ext2 semantic layer、UEFI handoff、handle / VMO / capabilityは独自実装の具体契約。Linux対応の採否・置換・adapterを別途設計し、技術ADRとして明示する。本DECで既存ADRを丸ごとSUPERSEDEDにしたり、Linuxの機構を既存Capabilityと同一視したりしない。 |

独自カーネルを製品の必須前提とする点は今回変更済みであり、同じ方針をもう一度決め直す必要はない。一方、既存実装規範との不整合は実装移行前の解消事項として残す。旧文書は履歴を保ち、DEC-017 / 020には日付付きの限定置換注記を追加する。

## 主な影響と必要な移行手順（REVIEW NOTE／手順案はPROVISIONAL）

これは実装開始命令・完了条件の承認ではない。後続の移行計画で範囲・担当・受入条件を具体化するための整理である。

1. **Privateで構成と切替境界を設計する。** Linuxの版・更新保守、ユーザーランド、起動・サービス構成、対応target、アプリ互換の対象、既存資産の再利用範囲を比較する。現行0.1をどこまで継続するか、どの版・枝で切り替えるか、段階移行かをユーザー判断に戻す。既存実装・作業中差分は無断削除しない。
2. **権限・信頼境界を再設計・検証する。** NagiのPrincipal / Session / Task / Resource / Revision、grantの狭窄・取消・期限、Provider認証、brokerの外からのアクセス拒否をLinux上でどう強制するかを技術ADRにする。Linux capabilitiesとNagiのobject capability / Permissionは同義ではなく、UIDやroot、一般的なプロセス隔離だけで同等性を保証しない。AIを非特権とし、モデル出力から任意shell実行や権限拡大へ直結させない。
3. **基盤・APIの対応表を作る。** boot / UEFI / image、独自scheduler・memory・driver、process / handle / VMO / channel IPC、VFS・network・display・audio等について、廃止・置換・再利用・adapterの候補を分ける。Linuxの既存基盤を使う範囲とNagi Serviceの責務を明示し、型付きPublic Action / Context / Eventと認可契約を保つ。すべての低水準契約がそのまま移植可能とは扱わない。
4. **SDK・アプリ・配布を検証する。** PAL / relibc、Rust target・C ABI、IDL / IPC、manifest・package・署名・更新、Servo / Mesa・Model Runtime等の依存とpatchを棚卸しする。Linuxアプリが動くこととNagi Platform統合・sandbox・承認が成立することを分ける。既存アプリ、GUI toolkit、バイナリ・source互換の対応範囲を個別に実証する。
5. **保存データと復旧を設計する。** Profile、Canonical Memory、Workspace / checkpoint、Activity / Task、app data・鍵・設定・開発Projectについてschema・識別子・所有・由来・暗号化・忘却を保護する。旧grantを復活させず現行Policyを再評価し、隔離検証、競合時の分離保持、移行元保護、失敗・中断・結果不明、実在するrollback範囲を定義する。Workspace Exportを全OS移行・任意app完全復元の保証にしない。
6. **公開と実装の承認を分離して整合させる。** 公開内容が別途承認された後、必要最小限のPublic ADR / AGENTS / Implementation Spec / Workstreamと検証契約の変更案を作る。既存Gateはその所有者・ユーザーが明示的に改訂するまで有効。本DECだけでM30を免除したり既存target証拠をLinuxへ流用したりしない。
7. **受入証拠を新targetで作り直す。** 再現可能なbuild / image / boot、権限拒否・IPC認証・隔離・失効・取消・資源制限、永続化・復旧・update、GUI・offline / AI fallbackを対象環境で検証する。host-only実験と移行後Nagi targetを明確に区別し、開発hostへの処理逃避をguest成功と呼ばない。旧Nagi KernelでのPASSは当時の証拠として保持するが、新Linux targetのPASSではない。性能・保守負担・移植容易性の利点は比較測定する。

## 未確定・暫定事項（PROVISIONAL）

- ディストリビューションを採用するか、独自にLinux基盤を構成するか。特定distributionの採用は未決定。
- Linux kernel revision、LTS / upstream方針、patch / module、更新保守・再現可能な供給・ライセンスと配布条件の整理。
- libc、init / service manager、shell、device manager、display / compositor、filesystem、package manager等のユーザーランド構成。
- IPC / handle / Capability / sandboxをLinuxへ対応させる実装、broker / serviceの責務、security threat modelと同等性の受入条件。
- Linux / POSIX / Windows等の互換目標、native binary実行・再compile・adapter等の方式、GUI toolkitと既存アプリの対応範囲。
- 現行0.1等との並行方針、移行時期・適用版・Workstream・target hardware、残す独自コードと廃止範囲、データ移行・rollback方式。
- 新targetのAcceptance・性能指標・数値、工数・日程・release commitment、Public公開範囲と公開時期。

これらは候補を含む未決事項であり、Linux採用と同時に承認された構成ではない。

## 関連文書と確認基準

- Private確認基準: `Roy-To/NagiOS-internal` main `8b111b33fbbee0bd5f997f6e3132ee4460e20f05`。README、OPERATING_RULES、TEMPLATE、AGENTSと上記DEC・Roadmapの関連節を確認。
- Public確認基準: `Roy-To/NagiOS` main `48b4eeb12a4a6674badd0a530149570ff4c30d30`。公開仕様・AGENTS・ADR・Gateはこの時点の規範として照合。公開側へ本記録を複製していない。
- [決定索引](README.md) / [運用規則](OPERATING_RULES.md)
- [DEC-017](DEC-017-third-party-developer-ecosystem.md) / [DEC-020](DEC-020-ai-os-interaction-path.md) / [DEC-021](DEC-021-roadmap-0.5-1.0.md)
- [0.3 Revised](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md) / [0.4+](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md) / [0.5–1.0](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)
- Public参照: `AGENTS.md`、`docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`、`docs/Nagi_OS_0.2_Codex_Implementation_Spec.md`、`docs/Nagi_OS_0.3_Architecture_Roadmap.md`、`docs/decisions/`、`.dev/workstreams.json`、`docs/implementation_status.md`。参照先は移行時に再確認する。0.2仕様の重複配置 `docs/0.2/Nagi_OS_0.2_Codex_Implementation_Spec.md`、Section 2.1 / 2.2 / 29 / 32（0.2-M00）も整合対象とする。

> **2026-11-16 後続決定（Foundation #1）:** [DEC-077](DEC-077-linux-kernel-lts-stable-policy.md)で、Nagi製品の標準Linuxカーネル系列をLTSとし、開発・先行検証では最新Stableの利用を許容する方針をCONFIRMEDとして追加。以下の初回記録にある「LTS / upstream方針は未確定」は2026-11-12時点の履歴であり、系列の標準方針に限りDEC-077が優先する。具体版・更新運用・patch／module・移行時期・Gateは未確定／変更なし。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-12 | ユーザーの確定決定として、独自Nagi KernelからLinuxカーネルへの方針転換をCONFIRMEDで記録。DEC-017 / 020の独自カーネル選択前提だけを限定置換。構成・互換方式・時期はPROVISIONAL、Private保存と既存Gateを維持。 |
| 2026-11-12 | 追加確定指示をDEC-043として記録。初回記録の構成未確定欄を限定的に更新する日付付き優先注記を追加し、初回内容と既存Gateを保持。 |
