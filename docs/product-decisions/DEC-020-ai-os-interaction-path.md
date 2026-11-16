# DEC-020 — AIのOS操作経路と計算資源の活用

> **2026-11-12 方針変更:** 本書の「独自カーネルから設計する戦略上の選択」という前提のみ、[DEC-042](DEC-042-linux-kernel-transition.md)によりSUPERSEDED。Linuxカーネルへの方針転換はCONFIRMED。構造化Context / Event、型付きPublic Action、非特権AI、決定論的な認可と通常GUIは維持し、Linux上の強制機構・移行時期は未確定。 旧本文は履歴として保持し、本注記がカーネル選択について優先する。既存実装仕様・ADR・Release / Activation Gateは変更しない。


**状態:** CONFIRMED（設計原則・既定方式）／PROVISIONAL（実装詳細・性能値・適用バージョン）  
**最終合意日:** 2026-11-17  
**対象:** Nagi OSとAIの情報受け渡し・OS操作経路。実装バージョンと採用時期は確定しない  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-002](DEC-002-cloud-ai-data-sharing.md)、[DEC-003](DEC-003-memory-classification.md)、[DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-008](DEC-008-proactive-behavior.md)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-010](DEC-010-intent-first-adaptive-ui.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-013](DEC-013-lightweight-memory-engine-benchmark.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)、[0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[0.2 Platform Foundation仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)、[純正ソフトウェア実装仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[Decision / Generative AI architecture](../architecture/decision-and-generative-ai-architecture.md)、[ADR-0018](../decisions/0018-decision-and-generative-model-architecture.md)

> **2026-11-16 Linux上のPermission強制方針の後続決定:** [DEC-084](DEC-084-nagi-permission-linux-sandbox-enforcement.md)で、Nagi Permission／Capabilityの認可判断とLinux保護機構による実効隔離を併用し、Linux syscall／device／filesystem等の直接経路からの権限迂回も防ぐB案をCONFIRMED。本文の「Linux上の強制機構は未確定」は個別技術・適用境界・実機受入の未決定を示す。型付きPublic Action、GUI fallbackの限定条件、AI非特権の方針は変更しない。

## 背景と目的

Nagiの設計上の問題意識は、PC内部の状態やアプリの操作を人間向けの画面・言語へ変換してAIが解釈し、その後また人間向けの操作経路へ戻す流れでは、避けられる変換や推論負荷が生じ得るというもの。これを減らし、限られた計算資源を必要な判断に集中させることを目指す。

この問題意識は設計目標であり、現行のすべてのLLMやアプリが同じ方式で動くという一般化、あるいは特定の削減効果が実証済みという主張ではない。

## 確定した設計原則

### AI向けの正規な操作経路

- NagiのOS機能と対応アプリは、権限を確認したうえでAIが必要な構造化Contextと、契約上公開されたEventを提供できるようにする。アプリ内部を無差別に読み取らず、Contextの取得前とActionの実行時に既存のPermission / Capability / Policyを評価する。
- OS・アプリの状態変更と機能呼び出しには、型付きPublic Action / Service APIと、既存のIPC・Provider境界を使う。AIは利用者の自然言語から意図を解釈し、OS・アプリの状態は権限確認済みの構造化Contextから得る。スクリーンショットの認識と座標クリックをNagi native操作の基本経路にせず、状態変更は正規の型付き契約を通じて行う。
- Resource ID / Object IDは対象を特定するために使うが、それだけで対象の恒久的な同一性や最新性を保証しない。Action実行時には既存契約に従い、Caller、Session、Permission、Resourceの現行Revision等を確認する。
- OSとアプリの決定論的な処理は、権限とPolicyを強制する通常のOS／アプリServiceが担い、原則としてLLM推論を介さない。LLMは、曖昧な要求の意味理解や必要な計画・判断など、推論が必要な箇所に利用できる。実行の権限と検証は下記「推論と実行の役割分担」に従う。
- 純正アプリと第三者アプリは、原則として同じ公開契約を使う。OS管理機能にSystem Capabilityが必要な場合は、通常のアプリ権限と区別し、既存の明示的な付与・Permission規則に従う。


### Contextを必要な分だけ段階的に取得する

- **初期Context:** 標準では、現在のユーザー意図・実行Taskに関連する情報を、小さく構造化した初期Contextとして提示する。
- **参照と段階取得:** 追加情報が必要な場合はResource ID、型付きmetadata、Context reference等で対象を参照し、目的やTaskに応じて詳細を取得できる構成とする。参照の提示やResource IDの所持だけでは、参照先の取得許可やResourceの恒久的な同一性・最新性を保証しない。
- **取得時の再評価:** 詳細は正規のPublic API／Context Broker等の認可された経路から取得する。各取得時点で現行Permission / Capability / Policyに加えて、情報分類・由来、owner、scope、対象Resource / Revisionを再検証し、不明・競合・不一致なら取得しない。
- **権限境界:** LLM / Agentが追加情報を必要だと判断・要求しても、Permission、Capability、Policy、承認済みTaskのScopeは増えない。追加要求も通常の認可経路で評価する。
- **一括投入を標準にしない:** 全ファイル、全ログ、画面のraw dump、全Memoryを常時または無差別に一括投入する方式を標準としない。
- **Cloud共有との整合:** 段階取得はDEC-002の送信・共有認可を独自に狭めたり広げたりしない。許可済みProvider、情報クラス、Purpose、Resource、Profile、期間・Task、Context budgetの内側で、非機密Memoryを広く提供する既存方針を維持する。各送信時点で現行PermissionとPolicyを評価する。
- **Memory境界:** DEC-003 / DEC-004の分類、由来、User / Profile・Task・Agent等の所有範囲、保存・忘却規則を維持する。要約や参照化で分類・由来・権限を失わせない。
- **資源効率の目標:** 小さな初期Contextと必要時の段階取得で不要なデータ処理・推論を抑え、inference tokens、CPU、RAM、latencyを節約する設計を目指す。効果は未測定であり、性能改善を保証しない。
- **承認済みTaskの例外:** 明示的に承認されたTaskが初期から広いContextを必要とする場合は、承認された対象・目的と現行Permission / Policyの範囲で広い初期Contextを構成できる。段階取得を厳格な禁止とはしない。

### 推論と実行の役割分担

- LLMが作る計画は提案であり、実行権限ではない。計画に含まれるAction提案は、型付きPublic Actionとして検証する。
- OS側は既存のPermission / Capability / Policy、該当する承認済みTaskのScope、対象Resourceと現行Revisionなどを実行時に確認し、検証を通過したActionだけを正規Executor / Provider経由で実行する。
- AIの提案に基づくActionも、OSから明示的に開始される決定論的操作も、同じ既存の権限経路とPublic Action / Service契約に従う。決定論的であることやAIを使わないことは権限を増やさない。
- LLM / Agentは非特権主体とし、Kernelやprivileged APIを直接呼び出さず、既存の決定論的な検証・認可・実行境界を迂回しない。
- Activity / Task状態には実際の実行結果を反映し、成功・部分成功・失敗を正確に示す。部分的な副作用を全体成功として扱わない。
- LLMが無効・不在・利用不能でも、通常のOS機能と操作を利用できる。

### 人間が使うGUIの継続

- マウス、キーボード、従来のDesktop / GUI、Accessibilityを含む人間向け操作を維持する。AIが使えない場合も通常のOS操作を可能にする。
- Nagi nativeの通常経路を、AIによるスクリーンショット認識と座標クリックにしない。統合済み機能には公開Action / Service APIを用いる。
- Public Action連携のないアプリに対するfallbackの許可条件はDEC-027に定める。型付きActionを優先し、fallbackも明示許可、限定Scope、既存のPermission／Policyに従う。

### イベント駆動を既定とするAI動作

- NagiのAI / LLM動作の既定はイベント駆動とする。対象は現行のPermission / Policy等の範囲で扱えるOS、アプリ、ハードウェアのイベントを契機とする。この原則自体は、全イベントに対する共通購読契約が存在することを定めない。
- 許可されたイベントは、まず非LLMの決定論的なルール、状態更新、通常のService処理で扱う。LLMは、曖昧さの判断や自然言語の理解・対話など推論が必要な範囲に用いる。
- 明示的なユーザー対話中、またはユーザーが承認した継続Taskの実行中には、継続推論があり得る。その場合も現在の対話・Taskの目的、Scope、Permission / Policyを越えない。
- イベント駆動は、すべてのEventを監視・収集したり、無条件でLLMに渡したりすることを意味しない。イベント検出のみでは、無条件の推論、通知・画面割込み、Action実行を認めない。通知はDEC-008、Action実行はDEC-001と既存の型付きPublic Action / Service APIに従う。
- LLMが無効・不在・利用不能な場合も通常のOS機能と基本GUIを使える状態を保つ（上記「人間が使うGUIの継続」）。

### カーネルと権限の境界

- 独自カーネルから設計することは、NagiのAIとOSの境界を最初から組み立てるための戦略上の選択であり、既存OSでは同じ利益を一切実現できないという技術的主張ではない。既存OSでも、構造化API、Permissionを強制するService、型付きIPC等により一部の改善は可能である。
- LLM / Agentは非特権主体とし、OSのPermissionやPolicyを回避させない。LLMをカーネルへ埋め込まず、LLMだけのRaw Kernel API、非公開Service経路、権限評価を迂回する隠しAPIを設けない。

### 限られた計算資源の活用

- Deterministicな処理をService側へ委ね、モデルの推論は必要な判断に集中させることで、軽量なローカルモデルでも有用なAI体験を成立させる方向を目指す。省メモリ、省電力、低遅延、不要なLLM呼出しとトークン消費の削減は設計目標であり、達成済みの効果や性能保証ではない。
- 1B–4B程度のモデルは軽量モデルの候補例にすぎず、必須サイズ、性能保証、既存の0.2モデル契約の変更を意味しない。モデル選択・Provider・Routingは、版管理された実装仕様とユーザー合意に従う。

### 8GB RAM級端末での実用的なNagiとローカルAI連携（2026-11-16 CONFIRMED）

- **製品の差別化目標:** Nagiは、RAM 8GB級の比較的限られた資源の端末でも、OS・Nagi純正アプリの一般的な日常操作を軽快に利用しながら、適切な小型・量子化ローカルLLMを介した**実用的なアプリ横断AI連携**を成立させることを明確な設計・評価目標とする。単にOSが8GBで起動するという目標より強い。ただし、これは**将来達成すべき目標**であり、現状の8GB実機で快適に動作するとの試験済み主張・全Taskでの性能保証ではない。
- **AI統合と従来操作の両立:** 純正アプリはNagi Platformの構造化Context、型付きPublic Action／Service API等から必要な場合にAI連携できる設計とし、一般の閲覧・入力・編集・再生など決定論的な基本操作はLLM推論なしでも実行可能とする。アプリ個別に大型LLMを常駐させることや、入力・画面描画・ファイル操作を毎回LLMへ直列依存させることを前提にしない。純正・第三者の公開契約と既存Permission／Policyは同じ。
- **8GBでの推論経路:** モデル選択・入替え可能なNagi Model Runtime／Routerを活用し、端末条件と利用者の明示設定に合わせて、軽量モデル、量子化、必要時のload/unload、必要最小限のContext、単一推論、低優先度の背景処理などの選択肢を評価する。モデルのサイズが小さいことだけで日本語理解、構造化計画、Tool／Action選択の品質を満たすとは断定しない。既存0.1／0.2のGranite 4.2 3B等のモデル選択契約を本節で変更しない。
- **前景応答を優先:** メモリ不足・CPU／GPU負荷・I/O逼迫時は、OSと純正アプリの前景操作を守り、許可されたAI処理をthrottle、queue、縮小、停止／model unload等で調整する方向とする。Taskの権限や中断・再開条件は既存のJobs／Model Runtime／Action Policy／DEC-022に従い、ユーザーによる明示停止を勝手に再開しない。NagiのProfile／Canonical Memory／習慣は、モデルや端末の変更により失われない。
- **端末能力と実測が前提:** RAM 8GBだけで体感性能を定義しない。CPU、GPU／NPU、メモリ帯域、ストレージ、電力・熱、同時に開くアプリ、モデルの重み・KV cache・推論用バッファを含む実負荷を計測し、基準端末・実利用Scenarioを定めて検証する。0.1／0.2の8GB QEMU参照構成は参考であり、8GB物理端末での性能実証とは区別する。

**未確定（PROVISIONAL）:** 「軽快」「実用的」の定量受入閾値（入力・画面更新・スクロール遅延、AI初回応答・完了時間、RAM・swap・電力・温度等）、代表ハードウェア、アプリ同時起動のScenario、モデル・量子化の選択表、初期パッケージ構成、モデル常駐／Unload・切替閾値、同時推論数、実装担当・Release割当ては後の実測・Implementation Specで決める。すべてのAI機能が8GBで同等速度を発揮することは確定しない。

### 8GB RAM級性能試験を0.3完成前から開始する（2026-11-17 CONFIRMED）

**CONFIRMED — 開始時期と目的:** Nagiの8GB RAM級での快適性・実用性は、0.3の完成を待たず、既存の権限・開発Gateが許す範囲で先行して段階的に性能試験を開始する。単なるOS起動やLLMモデルのメモリ収容をもって達成とはしない。通常の前景OS／GUI／純正アプリ操作の応答性と、ローカルAIの実際のTask速度・品質が両立するかを検証し、0.3までの設計・最適化判断へ結果を戻す。

**CONFIRMED — 検証の考え方:** AI無効時の基本操作をbaselineとして、利用可能な純正アプリとローカル推論の共存を段階的に比較する。軽量・量子化モデル、Contextの削減、load/unload、背景処理の抑制などを検討し、RAM使用量だけでなく、入力／画面応答、推論開始・完了時間、日本語・英語の理解／Action選択の正しさ、Task成功・失敗を区別する。既存の前景優先、Permission、承認済みTask、モデル非依存Memoryを維持する。

**CONFIRMED — 証拠の区別:** host-only PoC、Linux image／QEMU、物理実機の結果はそれぞれ区別し、使えない機能は未測定と明記する。先行評価だけで0.3 runtime機能・Target AcceptanceのPASS、8GB実機での快適動作達成、既存0.1／0.2／0.3 Release・Activation Gateの解除を主張しない。実装と計測は割当済み作業範囲・path ownership・安全制約に従う。

**PROVISIONAL — 未確定:** 初回着手日と担当、試験用ハードウェア（x86／ARM64／Raspberry Pi等）、OSイメージの成熟度、1〜2B／3〜4B級などの候補モデル・量子化・Context、同時起動アプリ・Taskシナリオ、計測工具、RAM／swap／入力遅延／tokens/s／初回応答／完了率／消費電力の合格値、成果物・適用Releaseは別途決める。8GBでの目標達成は未実証。

### インストール済み複数モデルの軽量フォールバックと切替（2026-11-16 CONFIRMED）

- **ストレージ保存とRAM常駐を区別:** モデル管理機構は、Standard／Lite等、複数の利用許可済みモデルと量子化版を端末のモデルストアに導入・保持し、必要なモデルを選択してload／unloadできる構成を目指す。複数のモデルがインストール済みであっても、同数を常時RAM／VRAMへ常駐させる必要はない。ストレージ消費、ロード時間、KV Cacheや実行時バッファのRAM消費は別々に扱う。
- **運用原則（自動化を目指し、手動も残す）:** 通常はユーザーが許可した標準モデルを利用し、端末のメモリ圧迫、前景アプリの負荷、Taskの重さ・推論能力要件に応じて、利用可能な軽量・量子化モデルへ適応的に切り替える体験を設計目標とする。自動切替が安全・十分な品質・実測性能を満たせない段階でも、ユーザーがModel Managerから手動でモデルを選択・切替できる経路を維持する。切替によって作業が却って遅くなる場合も考慮し、モデルの入替えだけを唯一の負荷対策にしない。
- **ユーザー選択と品質を優先:** ユーザーがモデルを固定したり自動切替を無効にしている場合、最適化のために勝手に別モデルを起動しない。許可された軽量候補が必要なTaskの日本語理解・構造化計画・Tool／Action選択・品質条件を満たさない場合は、単に小さいことを理由に危険な実行をせず、待機・負荷削減・処理分割・ユーザーへの確認など既存Task契約内の代替手段を検討する。資源不足を理由に無断でCloudモデルへ切り替えず、DEC-002の事前許可を維持する。
- **切替と継続性:** 異なるモデル間で推論内部状態・KV Cacheをそのまま引き継げると想定せず、通常は安全な推論／Task境界での切替を候補とする。すでに実行済みのActionを、切替だけを理由に再実行しない。Taskの進捗・Pending Action・Permission／Policy・Revision等を再確認し、Nagi Profile、Canonical Memory、Preferences、Habits、ユーザー設定を保持する。瞬断のない完全シームレス切替は保証しない。操作中の体験を不必要に妨げないことを目指す。
- **既存のRelease境界:** 0.1／0.2のGranite 4.2 3B標準モデル、Model Manager／Router契約、DEC-013のHardware-Adaptive Memory Engine、DEC-022のTask中断・再開境界、既存のRelease／Activation Gateをこの製品方針だけで変更しない。モデル・実装・検証は該当する正式な仕様・受入手順で確定する。

**未確定（PROVISIONAL）:** 自動切替機構の初期版への搭載可否と実装順序、ユーザー設定の具体的UI、切替の安全境界・中断／再開方法、ロード・アンロード時間、再利用可能なContextの範囲、頻繁な切替を防ぐ条件、RAM／CPU／GPU等の閾値、候補モデルと量子化方式、常駐数、品質評価方式、性能目標と達成値、対応端末とRelease割当て。初期実装が手動切替のみとなる可能性は残す。

### 適応型・必要最小限の推論資源管理

- **既定の資源配分:** Nagiは、推論を必要としない決定論的なOS／アプリ処理にLLMを使わず、必要な判断については端末の実測状況と利用可能な推論能力に応じて、**必要十分なローカル推論**を優先する。モデルの大きさそのものより、Taskの要件と品質、応答性、CPU／GPU／NPU、RAM、メモリ圧迫、消費電力を総合的に考慮する設計方針とする。
- **適応的な拡張:** 現在利用できるローカルモデルでは必要な判断品質を満たしにくく、かつ実行環境・ポリシーが許す場合に、より高性能なローカルモデルの利用を選択肢とする。クラウドAIの利用は別経路であり、DEC-002に定めるProvider、Purpose、情報クラス、Resource、Profile、期間・Task等の事前許可と、送信時のPermission／Policy評価を満たす場合に限る。ローカルで不足したことだけを理由に無許可でCloudへ切り替えない。
- **ユーザー設定の優先:** ユーザーが選択・固定したモデル、利用可能なモデルの範囲、自動切替の可否、クラウド利用方針を尊重する。自動最適化の都合で明示的な設定を上書きしたり、許可されていないモデル／Providerを起動したりしない。適切な推論経路がない場合は、品質低下や実行不能を正直に示し、通常のOS機能を維持する。
- **切替コストを含む最適化:** モデルのload／unload、初回応答までの待機、RAM／VRAM占有、複数モデルの常駐、Context Adapterの変換コストも資源配分の判断対象とする。小型端末で複数モデルを常時常駐させることは必須とせず、切替によってかえって効率が悪化する場合を考慮する。
- **モデルとユーザーデータの分離:** Modelの選択や切替が、Nagi Profile、Canonical Memory、Preferences、Habits、Task metadata等の正本を変更・破壊したり、モデルの権限を拡張したりしてはならない。Model固有の一時Cacheは既存のModel Runtime／Context Adapter契約に従って扱う。
- **効果の検証:** 必要十分な推論経路を選ぶことで、使用RAM、CPU／GPU負荷、消費電力、推論Token、待機時間を減らすことを目標とするが、各Taskの品質・失敗率も併せて実測する。適応型方式の優位性や具体的な削減値は未検証であり保証しない。

### 人間の操作応答性を優先するAI資源配分

- **標準優先順位:** Nagiの資源配分では、ユーザーが直接操作している前景UI／アプリの応答性を標準で優先する。AIの推論・検索・索引構築・承認済みバックグラウンドTaskは、前景の操作を不必要に遅延させない範囲で利用可能なCPU／GPU／NPU、RAM、ストレージI/O、電力等の資源を適応的に利用する。「余剰資源」は厳密な固定量ではなく、現在の負荷と前景作業の要求を踏まえた利用可能な予算を指す。
- **動的調整:** ユーザー操作の負荷が増えた場合は、既存のJobs、Model Runtime、OS Resource管理の通常契約を通じて、可能な範囲でAI側の並列度・優先度・資源使用を抑える。操作が落ち着いた場合は、承認済みで実行可能なAI Taskに利用可能な資源を再配分できる。資源不足による調整は、ユーザーが明示的にPause／CancelしたTaskや、未承認・中断済みTaskの自動再開を意味しない。Taskの再実行、外部への副作用、権限の拡張も許可しない。
- **ユーザーの明示的な優先度変更:** ユーザーは特定のAI Taskを優先して完了させるなど、資源配分の優先度を明示的に変更できる。優先度変更が前景アプリの応答性や電力・熱・待機時間に影響する可能性がある場合は、理解可能な方法で影響を示すことを目指す。通常の既定優先順位を選び直せるようにする。
- **安全・権限境界:** ユーザーの優先指定があっても、システムの安全性、機能維持に必要な処理、Kernel／System Serviceの保護、リソース上限、Permission／Capability／Policy、データ整合性などの制約を回避しない。AI／Model Runtimeにスケジューラを操作する特権を直接与えない。資源管理の実行判断は既存のOS／Service側で行い、AIは必要な範囲で正規APIから要求する。
- **品質・継続性:** AI Taskの進捗、待機、資源制約による遅延、失敗を区別し、Activity／Task状態へ事実に即して反映する。性能調整で未完了Taskを完了済みと偽らない。対象のAI Taskが長期に待機する場合の公平性・再開条件は別途設計し、既存のTask承認と中断／復旧規則を維持する。
- **測定方針:** 体感応答性だけでなく、入力応答遅延、画面更新の安定性、AI Task完了までの時間、CPU／GPU／NPU・RAM負荷、ストレージI/O、電力と温度を実測対象として検討する。優先度制御の具体的な性能値や改善効果は未検証とする。

### 効率の評価

- UI・言語の変換往復を減らすことは設計目標とする。トークン量、応答Latency、CPU時間、Memory使用量、消費電力、Actionの成功・失敗率を、対応するシナリオで測定して有効性を確認する。
- 現時点でこれらの指標の削減量や達成値は確定していない。測定前にトークン・電力・総計算資源が減ると断定しない。数値目標、比較Baseline、Acceptanceへの追加は別途決める。

### 別会話中の`builder` Taskと前景応答性（2026-11-05 CONFIRMED）

[DEC-015](DEC-015-builder-role-review.md)の「会話優先・`builder`は状況に応じて継続（選択C）」に従う。ユーザーが開発Task中に別の会話・前景操作を始めても、Taskを失敗・取消・完了にせず状態を保持し、Nagiの新しい対話と前景UIの応答を優先する。許可された非LLMのビルド／テストなどは利用可能なResource BudgetとOS Job／安全境界が許す範囲で継続してよい。単一Modelの推論は順番に処理してよく、独立Agentや複数Modelの同時推論を初期の必須要件としない。

**Resource待機の継続と明示停止は区別する。** 単に前景負荷でQueue／Throttle／一時待機した有効な承認済みTaskは、資源回復後に現行Scope、期限、Permission／Policy、Resource Revision、BudgetとCancel状態を再検証して安全に続行できる。一方、ユーザーが明示Pause／Cancel／Stopしたもの、未承認・承認待ち・権限失効・状態不明のもの、クラッシュ／再起動後の復旧待ちTaskは、資源回復だけを理由に自動再開・再実行してはならない（DEC-022）。Role・Model切替で権限は拡大しない。

前景会話と`builder` Taskの状態・Context・進捗を分けて記録・表示する。具体的なQueue制御、LLM推論の切替点、Resource Budget、待機表示・公平性と性能値はPROVISIONALであり、Release／Activation Gateを変更しない。

## 例外・禁止・安全境界

- 構造化Contextへの追加は無制限なMemory / app-stateアクセスを意味しない。取得対象はPurpose、Principal、Task、Resource、Permission等に応じて制限し、既存のプライバシー・Retention規則を守る。
- Modelが提案したAction、AI利用の有無、Nagi製アプリであることだけでPermissionやCapabilityを付与しない。
- AI向けの機械可読APIは、通常アプリや第三者が使えない非公開の特権APIを意味しない。System Capabilityは既存仕様で明示的に定義・付与する。
- 本決定は、0.1 M30、0.2 M21、0.3 Implementation Spec / Activation gate、Workstreamの所有権、既存Acceptance条件を変更せず、実装開始・Release activationを許可しない。

## 未確定・暫定案

- Context / Event schema、公開するOS・アプリ・ハードウェアイベントの種類と取得・購読条件、Resource / Revision IDの具体形式、IPC wire format、Public Action / Service APIの範囲、versioningとmigration。
- 初期Contextの量・schema、Context budget、優先順位・relevance、参照表現、追加取得の発火条件・回数、UI、失敗・cache失効時の挙動。
- 推論scheduler、Intent Router / Executorの内部方式、イベント集約・優先順位付け、LLMを使う判断・起動条件（閾値）、継続推論の制御などの具体的な運用方式。
- 前景操作を検知する信号、CPU／GPU／NPU等のスケジューリング方式、優先度・resource budgetの具体値、throttle／suspendの条件、Task再開・公平性と飢餓回避の詳細、ユーザー向け優先度UIと実測指標の合格値。
- DEC-027に定めたfallbackの具体的なUI automation／Accessibility方式、適格条件、許可UI、制御値、状態検証・監査方式と適用Version。
- 対応するローカルモデルの種類、サイズ、品質とHardwareの要件。1B–4Bは比較対象候補であり、採用は未確定。
- ベンチマークのTask set、Baseline、再現手順、計測ツール、メモリ・電力・Latency・Token等の数値目標と性能保証。実測結果は未取得。
- Modelの自動切替判断、Taskの難度評価、品質閾値、各種Resource budget、Model常駐数、経路の切替頻度／Fallback順序、ユーザー設定UI、計測方法の具体値と対応Version。
- Model / Agent、Context broker、各OS Serviceの具体的な配置と、各Releaseへの割当て。

## Roadmapとの関連

- **0.1:** M30と既存Workstream規則を優先する。本決定は0.1 Acceptanceを変えない。
- **0.2:** M21 SDK、Public Action / Service API、IPC、Permission / Capabilityの既存DeliverableとAcceptanceを維持する。新しいRelease Blockerは追加しない。Model Runtimeは既存仕様どおり非特権とする。
- **0.3:** Personal Agentは、既存Implementation SpecとActivation gateが通過した後、許可されたContextと通常の型付きPublic Action / Service APIを使う方向とする。本決定だけでは実装を開始しない。
- **0.4+:** Agentの種別・役割によらず、既存の委譲・Permission境界を維持し、非公開Shortcutを設けない。

## PDF Reader / Editorとの整合（2026-11-16）

[DEC-061](DEC-061-first-party-pdf-reader-editor.md)により、PDFの通常閲覧・編集はAIに依存しない。表抽出・要約・文書比較・引用等のAI支援は承認済み目的・Scopeと元資料の権限・機密分類内で、正規Public APIによる取得・Revision確認・Action検証を通す。Cloud送信はDEC-002に従い、前景の基本体験を維持する。 具体的な連携契約・適用版はPROVISIONAL。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-17 | 8GBでの通常操作＋ローカルAI連携の性能試験を0.3完成前に先行開始する方針をCONFIRMED。前景応答と推論速度・Task品質を比較。端末・モデル・日程・閾値・担当はPROVISIONAL。既存Gateとhost／target／実機の証拠区別を維持。 |
| 2026-11-16 | 複数の利用許可済み量子化モデルをストレージにインストールし、必要時だけRAMへロードする方針を具体化。資源圧迫時の軽量モデルへの自動切替を目指し、難しい段階では手動切替を確保。ユーザーの固定指定、Task品質・権限、Memory継続、安全な切替境界を優先し、シームレス性・閾値・適用版は未確定。 |
| 2026-11-16 | 8GB RAM級で純正アプリの日常操作と軽量量子化ローカルLLMによる実用的アプリ横断連携を両立する製品目標を確認。単なる起動成功でなく前景応答性とAI Taskの品質を実測する。具体的なモデル・閾値・達成実績・提供版は未確定。 |
| 2026-11-05 | 別会話中の`builder`作業継続Cを確定。Nagiの前景会話優先、Scope内の安全なバックグラウンド処理、Resource待機と明示停止／クラッシュ復旧待ちの区別、Task／Context分離を記録。具体的Scheduler・Budgetは未確定 |
| 2026-11-05 | AI資源配分は前景の人間操作の応答性を標準優先とし、AIが利用可能な余剰資源を適応的に使用する方式Aで確定。ユーザーの明示的なAI Task優先度変更を認める一方、OS安全・Permission・Task承認境界は維持する。実装方式・閾値・性能保証は未確定。 |
| 2026-11-05 | 適応型・必要最小限の推論資源管理を既定方針として確定。端末状況・モデル切替コストを考慮し、ユーザーのモデル設定とDEC-002のCloud事前許可を尊重する。最適化閾値・性能効果・実装版は未確定。 |
| 2026-11-04 | 人間向けUIを介する変換往復を減らし、AI用の機械可読なContext / Event / Action経路を正規のOS契約として用意する方針を確認。GUI、権限境界、既存Release gateを維持し、効率向上は測定で検証する。 |
| 2026-11-04 | イベント駆動をAI / LLM動作の既定とし、決定論的処理を優先すること、明示的対話・承認済み継続Taskでの推論、イベント監視・通知・Actionの境界を確定。具体的なスケジューラ、イベント種類、数値、性能保証、適用バージョンは未確定として分離。 |
| 2026-11-04 | Contextは目的・Taskに必要な情報だけを段階的に取得し、各段階で既存Permission / Capability / PolicyとPrivacy境界を維持する方針を確定。取得APIや具体的な段階設計は未確定。 |
| 2026-11-04 | 標準の初期Contextを小さく構造化し、Resource参照から詳細を段階取得する方針を明記。各取得で分類・由来・owner・scopeを再評価し、DEC-002の共有範囲を変えないこと、承認Taskの広い初期Context例外、効果未測定の資源効率目標を記録。 |
| 2026-11-04 | 決定論的に処理できる作業は通常Serviceが原則LLMなしで実行し、LLMは曖昧さの理解や計画・判断に使う方針を確定。Action提案の型付き検証、既存権限・承認済みTask・対象Revisionの実行時確認、正規Executor経由、OS起点操作も同じ権限経路、結果の正直なActivity表示、AI不在時の通常操作を記録。 |
| 2026-11-04 | 自然言語による意図理解と、権限確認済みContextによるOS状態取得を区別し、座標クリックを基本経路にしない点を明確化。 |
| 2026-11-12 | DEC-042により独自カーネル選択の前提だけを限定置換。Linux採用方針は確定、製品・安全境界は維持、実装詳細と切替時期は未確定。旧本文・履歴は保持。 |
| 2026-11-16 | DEC-061の純正PDF Reader / Editor方針への参照を追加。既存の公開API・AI非必須・権限境界と整合させ、実装版・既存Gateは変更しない。 |

## 2026-11-16 — Nagi PowerによるOS全体の管理

[DEC-067](DEC-067-nagi-power-deterministic-resource-management.md)でNagi Power（仮称）を採用。LinuxカーネルとNagi専用システムサービスが、AI／LLM推論を必須とせず、OS全体の電力・熱・負荷・背景処理を決定論的に管理する。AIタスクは通常の資源契約によるthrottle／queue／suspend／必要時model unloadの対象となる。本書の前景優先、安全限界内のユーザー優先指定、明示停止の尊重、資源待機と復旧待ちの区別、Action二重実行防止を維持する。具体方式・閾値・効果・対応端末・実装版・Gateは未確定。

変更履歴: 2026-11-16 — DEC-067への役割分担を追記。既存の承認・権限・版境界は変更しない。
