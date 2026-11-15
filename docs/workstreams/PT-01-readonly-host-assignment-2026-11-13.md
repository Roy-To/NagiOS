# PT-01 — 読み取り専用診断・host-only 初期担当登録（2026-11-13）

> **2026-11-15 公開移管:** [DEC-050](../product-decisions/DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

## 根拠と現在の状態

ユーザーの2026-11-13依頼: PT-01の実装担当を調整し、MacのLinux移行本線を優先継続。可能な限り別の既存開発環境を使用し、新規worktree・cloneを無断作成しない。既存担当との変更競合を確認し、担当・変更範囲・統合先・検証条件を登録してから実装する。初期機能は読み取り専用ソース診断とhost-onlyテストに限定する。

DEC-045 (参照時 blob 96521c111673e13f7fd5102cadef3b8098d0d79f)、product-decisions/README、OPERATING_RULES、AGENTSを確認した。この記録は限定初期割当の進行記録であり、製品受入、公開、release gate変更ではない。

**状態: 調整担当割当済み / 実装担当・実装path・統合先の確定は未完 / 実装未開始 / host-only試験未実行。**
未完項目を登録したことを実装開始条件充足とは扱わない。

## 担当と変更範囲

- 調整・引渡し責任者: Dot。
- 初期診断・環境適合確認担当: DotのPT-01担当。担当範囲は既存環境・既存仕様の読取、所有範囲の照合、本登録文書のみ。
- 製品コード実装担当: 未確定。新チャットが必要な場合はLuna Max・通常スピードを実指定し、担当引受け・実開始を別に確認する。
- 現時点の書込み範囲: この文書のみ。MacのLinux移行コード、共有Cargo設定、lock、Git状態、ビルド生成物は変更しない。
- 実装対象候補: PT-01のhost-side診断ロジックと専用host-onlyテスト。CLI言語、Schema、具体的pathはDEC-045で未確定のため、既存接続・所有範囲を確認して次の改訂で固定する。PT-02〜06は開始しない。

## 作業環境と競合

接続一覧では既存Mac miniと既存Dot cloud computerを確認。他のWindows/Linux開発環境は今回の一覧では確認できない。両環境のdiscoveryはdeadline_reachedであり、任意のソースや試験能力が利用可能とは推定しない。

既存Dot cloudを優先候補とする。現在のcloud作業領域には2026-10-10のSDKレビュー資料が存在するが、最新PT-01ソース・承認済み実装checkout・実装先branchの存在証拠ではない。レビュー資料を最新本体へ読み替えず、勝手なclone・worktree作成・環境新設は行わない。

MacのLinux最小起動、init、共通ビルドを中断・再配置せず、本PT-01担当はそれらの作業場所・生成物へ書込まない。Private仕様照合担当は現在read-onlyで、本登録pathへの書込予約なしと回答。DEC-045自身は編集しない。全既存実装担当とのpath競合照合は未完のため、実装path確定前に追加確認する。

## 統合先と開始条件

- 本登録の正本: Roy-To/NagiOS-internal の main、docs/workstreams/PT-01-readonly-host-assignment-2026-11-13.md。
- 実装統合先: Private開発範囲を予定するが、具体的repository path / branch / Integration Ownerの引受けは未確定。公開リポジトリへの転記・push・PR・CI公開はしない。
- 次の担当業務: 既存開発環境の実在ソース・許可path・branchと所有者を確認し、他担当と重ならない実装・出力範囲、統合先、資源条件を本記録へ確定する。その読返し確認後に限定実装を開始する。
- 確認責任者: Dot。
- 確認内容: 実装担当の引受け、既存環境とinputの実在、変更path競合なし、integration owner、host-only実行条件。
- 確認時期/条件: 実装開始前。未確認のまま実装・試験済みと報告しない。

## 初期検証条件

1. 診断入力は明示選択された固定ソースまたは権利上問題のない専用fixtureのみ。PC全体、ホーム、外部SSD、他project、秘密情報を探索しない。
2. 入力は読取専用。入力のbuild script、任意コマンド、依存取得・インストール・ネットワーク送信を行わない。診断対象自体をビルド/実行しない。
3. 元入力不変、symlinkの対象外逸脱拒否、不正パス・壊れた定義・不正エンコーディング・過大入力の安全な処理をhost-onlyで検証する。
4. Windows/Linux各fixture、同一固定入力での決定的結果、根拠と未検証事項、未知APIを対応済みとしない分類を確認する。秘密文字列・ソース本文・不要な絶対パスを出力しない。
5. テスト出力は担当専用の確認済み出力先へ分離し、既存容量・時間・プロセス監督条件を継承する。具体的資源条件は実行前に既存正規記録から確定する。
6. host-only成功と実アプリ移植成功、Linux image boot、Nagi target acceptanceを区別する。VM/QEMU・インストール・公開は初期PT-01範囲外。
7. 新たな拒否・結果不明・権限不足は該当工程のみ停止し、別担当/環境で迂回しない。

## 今回の結果

仕様・既存環境の読取と担当調整を実施。実装・host-only試験・製品統合は未実施。保存はcommitと読返し確認が成立した場合にのみ完了とする。


## 継続照合と既存担当への引渡し（2026-11-13）

- 最新DEC-045を再取得。参照blobは 35d4fba2d85ff6ad3be37e887cf498ca96fcf9e8。非AIの決定論的診断を基盤とし、LLMは補助とする後続方針を確認。PT-02〜04の設計追加は本担当の実装範囲を拡大しない。
- Publicの既存正式registry `.dev/workstreams.json` (blob 004bd09b6504ab7c5272d04d20640118c24d8b9b) を読取照合。`app-sdk-contract` は Codex App SDK contract workstream、owner branch `codex/0.2-app-sdk-contract`、所有範囲 `sdk/**`, `tools/nagi-pkg/**`, own State。共有 `tools/nagi-cli/**` はdevelopment-foundation/diagnostics/CI等と重なるためPT-01で変更しない。`sdk`一般枠はunassignedであり、既存App SDK contract所有者を無視してよい意味ではない。
- App SDK State (blob c072e8e43bf9a663c9e89ef8832b7c0223188d3c) の検証版は 0117bf51830fc77863b3468f0e8406b3eab4817b、2026-09-26のhost-side契約成果。現行Linux runtime実装済みや今回のhost-only試験成功の証拠ではない。
- 既存チャット「SDK サンプルを実装」 を正規履歴で特定。既存cloud /workspace、最終sanitized candidate c6d867a749f10968d8245a8906f2689b05e27365、未公開review待ちの記録。これを最新SDK checkoutの証拠としない。
- 当該既存担当へ、元成果保全を維持し、既存環境・ソース版/path・所有範囲・統合先・PT-01引受け可否を読み取り専用で確認する依頼を送信し、受理と新turnのinProgressを確認した。実装はまだ許可せず、正本登録の未完開始条件を引き継いだ。新チャット、clone/worktree、工具導入、環境新設、SSDアクセス、Mac本線変更は指示していない。
- 現段階は「準備/診断依頼の受理」。具体的診断実行記録・引受け結果・現在の環境実在は返答待ち。実装/host-only試験開始数は0。次の確認担当はDot、確認対象は当該担当の診断結果、確認条件は応答到着時。


## 限定実装の確定割当（2026-11-13、後続優先）

本節は上記「実装担当・path未確定」を以下の限定範囲で更新する。過去の未確認記録は履歴として保持する。

- 実装担当: 既存チャット「SDK サンプルを実装」。診断ターンで実読取診断を完了、続くターンで「この限定実装担当として受諾」と回答済み。
- 実行環境: 当該担当の既存cloud Linux x86_64。現物確認済みソース /workspace/NagiOS、既存branch work、固定HEAD 5b183660eee9babc1478fd05386e412026a0b87a。取得・branch切替・新環境・clone/worktree追加を行わない。旧SDK sample worktree/commit/ZIPは現在local不在のため復元しない。
- 専有変更範囲: tools/nagi-porting/** のみ。診断ロジック、版付きreport schema、専用Windows/Linux合成fixture、host-only tests、専用試験出力と結果記録をこの下にまとめる。初期は既存Python 3標準ライブラリを使用し依存導入しない。
- 競合確認: 当該pathは不存在、追跡ファイルなし、正式registryに専用予約なし。SDK sample担当自身の変更範囲とも重ならない。広いintegration ownerの tools/** 予約との調整は当該owner本人への確認が未完。この新規pathのみの専有割当を予定するが、本人確認と正本追記までは実装開始を保留する。共有SDK・CLI・Cargo・lock・registry・.git・Mac本線・他担当ファイルを変更しない。
- 入力: この範囲に作成する許諾可能な合成fixtureだけを初期診断対象とする。診断対象のbuild script・任意コマンド・依存取得・ネットワーク・全PC探索は禁止。source dataは読取専用で、fixture作成と診断実行を分離して不変確認する。
- 統合受取owner: Dot。候補統合先: Private Roy-To/NagiOS-internal、既存mainの tools/nagi-porting/**。これは候補であり、当該統合ownerによる受領とその具体的書込み権限の確認は未完。現段階は非公開候補の引渡しまで。採用commit・push/merge・公開・Public CIをこの割当から実行しない。SDK契約変更が必要になった場合はCodex App SDK contract workstreamとの調整へ戻し、重複SDKを実装しない。
- 工程完了条件: 同じ実装コードで、Windows/Linux入力、決定性、根拠/未検証表示、入力不変、symlink拒否、パス不正・破損・encoding・過大入力の安全な失敗、秘密/ソース本文/不要な絶対path非出力をhost-onlyで検証し、実コードのhash、変更一覧、各コマンド・exit・試験件数・未検証事項を引き渡す。PT-02〜06、AI推論、VM、target runtimeは含めない。

### PT-01専用の軽量実行管理

既存記録にこの新規PT-01の数値budgetはなかった。Mac等の別工程の許可・上限を転用せず、通常の担当調整として以下を初回軽量試験の暫定監督条件案とする。既存の許可済み数値制限の事実ではない。不足が判明した場合は固定上限とせず、必要量と根拠を報告し、既存制限を維持して実行前に見直す。上位の既存制限が判明した場合はそちらを優先し、緩和しない。

- host-only試験は1プロセスずつ。各試験コマンドのwall time上限60秒。
- PT-01専有範囲の追加ファイルと生成物合計は32 MiB以内。入力は小さい合成fixtureを使い、large-input検証にも上限を適用する。ディスク全体・SSDを走査せず、この専有範囲だけを測る。
- 既存の標準機能でtimeout、終了状態、stdout/stderr、exit codeを記録。開始した自分の試験process/sessionだけを監督し、timeout時はそのprocess groupに限定して終了・子processの回収を確認する。無関係なprocessを停止しない。
- 上限に達したら関連試験を停止して原因を報告する。許可範囲内の小さい入力・単体試験で問題を絞ることは可。無断上限引上げや失敗結果の偽装は不可。
- コマンド結果不明なら既存session結果を回収し、重複実行しない。実装のsource書込みとhost-only試験は本節の保存・読返し確認後に開始する。

本節時点では割当・受諾・診断完了まで確認。製品コード実装とhost-only試験は開始前であり、成功を先取りしない。


### 開始状態の明確化
6402c3bfの登録段階でコード実装・試験は開始していない。広域Integration Owner本人との競合調整、Private候補統合先の受領・権限確認が未完であり、その確認を開始条件として追跡する。初回60秒/32MiBは暫定監督案で、既存の制限として扱わない。


## PT-02/PT-06準備との重複回避照合（2026-11-13）

- Private Issue #8（PT-02）と #9（PT-06）の本文・コメントを正規取得。両者はopen、assigneeなし、PREPARATION / NOT STARTED。Issue作成や設計保存を実装開始・評価実施とは扱わない。
- 参照済み準備文書: docs/porting-toolkit/PT02_SDK_SURFACE_AUDIT_2026-11-13.md（blob c5240648f66cbe2ad9a0e037fa3719b95d4cc779）、docs/porting-toolkit/PT06_MIGRATION_EVALUATION_TEMPLATE_2026-11-13.md（blob 6b830a81f16ab6b56689ae48efbc0bbff0b5d7d5）。PT-01でこれらの準備を重複作成・編集せず、生成器や実アプリ比較測定は開始しない。
- Public固定ref 48b4eeb12a4a6674badd0a530149570ff4c30d30 の sdk/rust/Cargo.toml（blob d09a2c3b7e634e96583533f7e3e7ec1cf23922b6）を独立に再取得し、publish=false、nagi-modelの相対path依存を確認。samples/hello-nagi/Cargo.toml（blob 7cfd8a2721a1753e9133231c48dd1a84ccd4b64b）も ../../sdk/rust 参照を確認。
- 外部アプリ向けのSDK配送・固定参照方式はSDK Owner確認が必要。公開済みCrates.io配布やGit依存の取得経路を推定しない。一方、PT-01の読取専用静的診断にはSDKのbuild/配送を不要な開始前提として追加しない。未知・未確認のSDK機能は未対応/未検証と区別する。
- 最新DEC-045 blob b8596496d39d9e3b0b3de5c919d5bd229776f7ab を確認。非AIの決定論的処理が基盤、ローカルAIは小作業補助、大きな実装は開発者が選ぶ外部開発AIまたは人間という方針。外部へのソース共有の自動許可ではない。RAM 16GBを性能実測・正式最小要件と扱わない。
- SDK担当へ上記根拠と境界を引渡し、応答で受領・重複作成なし・PT-01をSDK build待ちにしないことを確認。コード/試験/環境/SSD/Git操作は未開始と本人が回答した。
- 広域Integration Owner本人確認とPrivate候補統合先の受領条件は引き続き未充足。関連担当への共有は完了したが、当該承認や実装開始の証拠には読み替えない。
