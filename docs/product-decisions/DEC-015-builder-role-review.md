# DEC-015 — プログラミングRole「builder」と段階的な実行方式

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（Role名 `builder`／実行方式C：初期Nagi Role切替・将来専門Agent対応）／PROVISIONAL（移行条件・実装詳細）  
**最終合意日:** 2026-11-05  
**適用候補:** Nagi OS 0.4（実装・Activation未許可）  
**関連:** [DEC-024（誤用訂正）](DEC-024-external-jev-reference-and-builder-label-correction.md)、[DEC-016（実行方針）](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-075（アプリ試用・導入）](DEC-075-builder-app-trial-and-installation.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)、[Decision/Generative Architecture](../architecture/decision-and-generative-ai-architecture.md)

## 重要な訂正（2026-11-05）

ユーザーが「TypeSafe AIのJevのような役割」と例示したものを、アシスタントが「Jevというプログラミング専門Agent」として誤解・命名したことを訂正する。TypeSafe AIのJevは型付き判断のための外部モデルであり、開発Agentの名前ではない。今後、内部のコード修正・ビルド・テスト担当に**Jev**という製品名を使わない。

プログラミング担当の**Role名は `builder`（小文字）で確定**する。コード作成・修正・ビルド・テスト等の開発作業を担当するRoleであり、TypeSafe AIのJevとは別の概念である。**初期はNagi自身が `builder` Roleへ切り替えて実行する（選択Cで確定）**。初回から独立した開発Agentを必須とする旧二体案は採用せず、必要になれば同じRoleを独立専門Agentが実行できる構造に拡張する。

## 実行方式C（2026-11-05 CONFIRMED）

- **初期構成:** ユーザーの一貫した窓口・実行主体はNagi一体とし、プログラミング作業を受けた際に同じAgentの`builder` Roleを使う。別のAgent identity、専用の常駐プロセス、別LLMの存在を要件にしない。
- **Task単位の分離:** `builder`作業には独立して追跡できるTask状態、許可されたResource／Action範囲、予算・期限、作業領域、差分・テスト証拠、Activity／復旧情報を持たせる。Nagiと同じAgentであることを、権限の分離を省略する理由にしない。実効権限はOS／Providerが現行Policy・Permissionを検証する。Role切替そのものは権限を生まない。
- **開発実行の既存要件:** [DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)の承認Scope内での修正・テスト・有限Retry、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)の隔離編集と安全な条件付き反映、[DEC-019](DEC-019-builder-work-data-cleanup.md)の状態別整理、[DEC-022](DEC-022-builder-interrupted-task-recovery.md)の中断復旧を`builder` RoleのTaskに適用する。Git commit／push、リリース・本番操作は別途の明示認可が必要。builderが新規アプリを作った場合の試用・採用・正式導入は[DEC-075](DEC-075-builder-app-trial-and-installation.md)に従い、Role切替やTask承認から導入権限を導かない。
- **将来の拡張:** ローカルResourceと正式なAgent／Session／Delegation境界が満たせる場合、`builder` Roleを別の専門Agentへ担当させることも可能にする。その実行時は別Principal／Sessionと範囲付き委譲・Stop伝播・監査を適用する。専用LLMや同時並列実行は自動的な要件ではない。
- **旧案との差分:** 旧「初回0.4はNagi＋専用Build Agent二体、Child Taskを別Agentに委譲」は本決定で置き換える。初期構成で独立したChild AgentやAgent間Delegationを必須としない。Nagi内のTask分離は維持するが、Agent間のセキュリティ分離を実装済みと表現しない。

**PROVISIONAL:** Role実行時のRuntime／Session切替機構、Task IDと履歴の具体的な実装、モデル選択、独立Agentへの移行時期・条件、具体的な数値Budget／Acceptanceは後続の実装仕様で決める。0.3 Acceptanceと0.4 Entry Gate／Integration Owner checkpointは変更しない。

## アプリの共有・公開権限との分離（DEC-036）

`builder` Roleの選択・Task承認・ソース作成は、成果物の第三者共有、外部送信、一般公開を許可しない。共有・公開は[DEC-036](DEC-036-builder-app-sharing-and-publication.md)に従い、対象と公開範囲の選択、内容・Privacy／Permission確認、実際の送信・公開に対する明示承認を独立に扱う。

## Role切替トリガーとユーザー表示 — 選択C（2026-11-05 CONFIRMED）

- **Intentに基づく自動切替:** Nagiは依頼の目的を解釈し、コードの作成・修正・ビルド・テストなどのプログラミング作業が必要と判断した場合、初期構成では同じNagi Agentの`builder` Roleを自動的に選んでTaskを準備・処理する。ユーザーがRole名を知らなくても利用できる。
- **見える状態表示:** `builder` Roleを使うTaskについて、ユーザーに「builderで作業中」など、現在の作業RoleとTask状態が分かる表示を行う。別の独立Agentが動いていると誤認させない。Nagi Home／Desktopから必要な進捗・承認・停止操作へ到達できるようにする（DEC-010／011）。
- **手動指定と通常Nagiへの復帰:** ユーザーは自然言語等で`builder`を明示指定でき、通常のNagiとしての対話へ戻すこともできる。Role選択の変更だけで、進行中Taskが自動的に取消・再起動・完了したことにはしない。Pause／Cancel／再開が必要なら既存Taskの状態と承認規則に従って扱う。
- **権限・承認の独立性:** 自動／手動のRole切替は、Workspace・Resource・Actionの実行権限、利用可能なModel／Cloud Provider、承認済みTask Scopeを拡大しない。ファイル変更・コード実行等はDEC-001／011／016／018およびOSの現行Permission／Policyを通過する。切替前の承認を別Taskへ無条件流用しない。
- **ユーザー体験:** 会話相手は一貫してNagi。開発が必要なら役割に応じた作業進捗を示し、作業後はNagiが差分・試験証拠・結果を報告する。日常操作を妨げる不要な確認や、切替のみを理由にした強制的な画面変更は求めない。

**PROVISIONAL:** 自動判定の閾値／例外、明示的Role指定の入力方式、ステータスの配置・文言・アイコン、Role変更中のTaskの並行性／切替状態遷移などの実装詳細は未確定。リスク別承認・権限制限・0.4 Gateは変更しない。

## 別会話中の`builder` Task継続方式 — 選択C（2026-11-05 CONFIRMED）

ユーザーが`builder`の開発Task実行中にNagiとの新しい会話・別の作業を始めた場合は、**前景会話を優先しつつ、承認済み`builder` Taskの状態を保持して可能な範囲で継続する**。

- **対話を優先:** Nagiは新しいユーザー発話へ応答できることを優先し、`builder`の長時間推論が会話・前景UIを占有し続けないようにする。初期の単一Agent／単一Model構成では、LLM推論を必要に応じて逐次化・切替する。複数LLMの並列推論や独立Agentの同時稼働は初期0.4の必須要件ではない。
- **Taskの状態保持:** 新しい会話への切替だけでは、承認済み`builder` Taskを取消・失敗・完了扱いにしない。Task状態、作業領域、証拠、Activity、許可Scope等を保持し、Nagiの会話Contextと混同しない。
- **安全な範囲で継続:** 承認済みScope内のビルド／テスト等の非LLM処理は、利用可能なOS Job／Resource管理と安全要件が許す場合、バックグラウンドで継続してよい。推論や競合する処理は端末Resource、ユーザー設定、前景負荷に応じてQueue／Throttle／一時待機させる。これはAgent／Modelの無制限並列実行を認可しない。
- **Resource待機からの条件付き継続:** 前景負荷による**資源待機のみ**で、Taskが依然として有効・承認済み・取消されておらず、現行Permission／Policy、Resource Revision、Budget、期限等を満たす場合は、利用可能な資源が戻った際に安全な処理を継続できる。資源回復だけで再承認が必要なActionや新しい副作用を自動認可しない。
- **自動再開しない状態:** ユーザーが明示的にPause／Cancel／StopしたTask、権限切れ・追加承認待ち・競合／状態不明のTask、クラッシュ／再起動後の復旧待ちTask（DEC-022）は、単に会話が終わった・資源が空いたことを理由に勝手に再開・再実行しない。必要なユーザー選択・再承認・整合性検証を優先する。
- **ユーザーへの可視化:** `builder`の作業中、資源待機、承認待ち、明示停止、中断復旧待ち、部分成功／失敗をTask／Activity上で区別して示す。別会話中も必要なら進捗・停止操作にアクセスできる（DEC-011）。成果は事実に基づいてNagiが報告する。

**PROVISIONAL:** LLM推論の具体的な中断／保存／再開点、QueueとJob Schedulerの実装、資源予算・優先度・待機条件、ユーザー向け表示細部、機種別の同時処理の可否と性能値は後続の実装／Acceptanceで決定する。既存0.3 Acceptance、0.4 Entry Gate／Integration Owner checkpointを変更しない。

## 追加指示・別の開発依頼の扱い — 選択C（2026-11-05 CONFIRMED）

`builder`による承認済み開発Taskの実行中に新しい指示があっても、Nagiは**依頼の対象・意図・既存Taskとの関係・承認Scope・Resource競合**を確認し、同じTaskへの変更、別Task、要確認の変更を区別する。ユーザーの窓口は引き続きNagiとし、Task状態や承認履歴を混同しない。

1. **同じ開発の小さな追加・修正:** 既存Taskと同じ目的・対象・許可Action／Resource範囲・期限／Budget内で対応でき、重大なリスク・競合がないなら、Nagiは新しい指示を既存Taskの計画・検証項目へ反映してよい。進行中のBuild／Testの成果が古い計画に基づく場合は、そのまま最終証拠に使わず、必要な再検証を行う。追加指示によって過去のTask承認を無制限に書き換えたり、範囲を広げたりしない。
2. **別件の開発依頼:** 既存の開発目的から独立した依頼は、新しいTaskとして識別・記録し、初期0.4のNagi一体・逐次Role実行の範囲では待機／Queueを基本とする。既存TaskのWorkspace・Context・権限・承認・成果を別Taskへ無断流用しない。新Taskの実行前には必要な範囲と権限を提示し、既存DEC-001／011／016に従って承認を得る。
3. **大幅変更・競合・優先順位変更:** Task目的、対象ファイル、Resource／Action Scope、検証条件やリスクが大きく変わる場合、同一Resourceへの競合がある場合、または既存Taskを止めて新Taskを優先する必要がある場合には、Nagiが影響、残作業、選択肢を説明し、必要なユーザーの選択と追加承認を求める。判断が曖昧なら推測して破壊的な変更を進めず、確認待ちとする。ユーザーが明示した優先順位はDEC-020のResource／Safety制約内で扱う。
4. **実行中Taskの保護:** 新しい指示の受信だけで既存Taskを取消・自動再実行・無条件中断しない。変更により現在の処理が不適切・競合となる場合は、安全な境界で待機／停止し、隔離Workspace、未反映Patch、差分、Test証拠、Activity／Task履歴を保護する。反映前に対象Resourceの現行Revisionと競合、Permission／Scope、原本保護を再確認し、古いPatchでユーザーの新しい変更を上書きしない（DEC-018／022）。
5. **透明性:** Nagiは元Taskへ追加した内容、新しいQueue Task、承認／優先順位の確認待ちを区別して表示し、Task変更後の進捗・残課題・検証結果を正確に報告する。完了判定はDEC-016のリスク別条件に従い、Git commit／push・本番反映を自動許可しない。

**PROVISIONAL:** 同じTaskとみなす具体的な判定基準、更新された計画・承認のVersion管理、Queue順序・Priority UI、競合時の停止点・Task分岐形式・数値Budget／Retryは実装仕様で確定する。初期から複数Agent／複数LLMの並列実行を要求せず、0.3 Acceptance・0.4 Entry Gate・Integration Owner checkpointを変更しない。

## 過去に議論したBuild／Execute作業要件（安全要件を引き継ぐ）

- 承認済みのWorkspace、Task、対象Resource・Actionの範囲で調査・コード修正・Build／Test等を行い、検証できる成果物を返す。
- Nagiをユーザーの相談・計画・承認・報告の窓口とし、初期はNagiが `builder` Roleを実行する。
- 子Taskを使う場合も、委譲Scopeは親を越えず、副作用は型付きPublic ActionとProvider Policy・Permission検査を通す。
- Stop、Task／Activityの相関、provenance、再現可能な検証は維持する。
- 従来の「Nagi＋独立開発Agent二体」を必須とする前提は撤回する。初期は1ユーザー／1端末・NagiのRole切替とTaskごとの分離を基本にし、並列性や別Agentへの委譲は後続Gateで扱う。

## 別物として維持する判断モデルArchitecture

- [ADR-0018](../decisions/0018-decision-and-generative-model-architecture.md)のTier 0 Deterministic Fast Path、Tier 1 DecisionProvider、Tier 2 Generative／Reasoningの分離は変更しない。
- Jevはあくまで外部の参考製品であり、Nagi OSへ採用済みという意味ではない。DecisionProviderは差し替え可能とし、専用モデル未導入でも既存ローカルGenerativeProvider／LlmDecisionAdapter等の経路を持つ。
- 判断モデルのconfidenceは実行権限を生まず、OS側のValidator／Policy／Permission／Executorが実行を制御する。

## 未確定事項・安全境界

- Role名 `builder` と初期Nagi内Role切替（C）は確定済み。将来の独立専門Agent導入条件、具体的Runtime、モデル選定・予算は未確定。
- 上記DEC-016／018／019／022のTask分離・安全要件をNagi内のRole切替で実装する具体方式と、その将来の独立Agentへの移行方式。
- 本訂正は0.3 Acceptance、0.4 Entry Gate、Integration Owner checkpoint、既存Workstream所有権と実装許可を変更しない。Publicリポジトリに未公開仕様を転載しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-04 | 旧記録：アシスタントが外部製品名Jevを開発Agentの名称と誤って扱い、初回0.4の専門Agentと記載した |
| 2026-11-05 | ユーザーの訂正を受け、Jevという内部名称を撤回。開発担当の仮ラベルをBuilderに変更し、Agent／Roleの位置づけをREVIEW REQUIREDとした |
| 2026-11-05 | ユーザーがプログラミング担当Role名を `builder`（小文字）と確定。独立Agent化や0.4初回構成は未確定のまま維持 |
| 2026-11-05 | 実行方式Cを確定。初期はNagi自身のRole切替でTask／作業領域／Permission scopeを分離し、将来は同一Roleを独立専門Agentにも実行可能にする。旧初回二体構成を置き換え |
| 2026-11-05 | builderへの切替トリガーと表示方式Cを確定。Intentに応じた自動Role選択、RoleとTaskの状態表示、ユーザーによる明示指定・Nagi通常対話への復帰を認め、Role切替とTask取消・Permission付与は区別する |
| 2026-11-05 | 別会話中のbuilder Task継続方式Cを確定。Nagiの前景会話優先、Task状態保持、安全な非LLM処理の条件付き継続、Resource待機からの条件付き継続とユーザー停止／クラッシュ復旧待ちの自動再開禁止を区別 |
| 2026-11-05 | 追加指示・別の開発依頼の対応方式Cを確定。同一Task内のScope内軽微変更は計画調整、別件は独立TaskとしてQueue、大幅変更／競合／優先変更はユーザー確認と追加承認。作業中成果とRevision・権限・監査を保護 |

## 関連決定の参照注記（2026-11-21）

- [DEC-113](DEC-113-untrusted-content-and-prompt-injection.md) — `builder`が読む外部資料は作業対象のデータとして扱う。ユーザーが外部手順の利用を明示した場合は依頼範囲内で扱い、高リスク操作には同決定の確認条件を適用する。
