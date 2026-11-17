# DEC-009 — Nagi-first操作思想とNagi Home / Desktop

**状態:** CONFIRMED（既存Nagi-first／Home・Desktop操作規則、PX-01の対話中心の統合Home B＋将来の高度な適応Home Cという高水準方針）／PROVISIONAL（具体的レイアウト・導線・カード数・Nagi Bar位置・AI停止時の詳細UI・技術・提供版）  
**最終合意日:** 2026-11-17（PX-01を追加。2026-11-15までの既存合意を維持）  
**対象:** Nagi OSの操作思想とログイン後の製品体験。実装バージョン・採用時期は本決定では確定しない  
**関連:** DEC-001〜008、[0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[0.4+ロードマップ](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)

## 目的とユーザー体験

NagiをOS操作の第一の窓口としながら、ユーザーが従来のGUIを含む複数の方法でOSを完全に操作できるようにする。Nagi HomeとNagi Desktopを切り替えても、進行中の会話・Task・Workspace・アプリ状態を失わない。

## 確定した仕様

### 操作思想

- NagiをOS操作の第一の窓口とする（**Nagi-first**）。
- マウス・キーボードによる従来のGUI操作も完全に維持する。
- AIが利用できない場合も、通常のOSとして操作できる。
- AIは非特権であり、正規のOS APIとPermission境界に従う。Nagiであることや会話の文脈だけで追加の権限を得ない。

### Nagi HomeとNagi Desktop

- ログイン後の標準画面は、AIとの対話を中心とする **Nagi Home（B）** とする。
- **Nagi Desktop（C）** は、従来のGUIにNagiが統合された作業画面とする。
- Nagi HomeとNagi Desktopは双方向に切り替えられる。
- ユーザーがNagi Homeからアプリを起動した場合、画面を自動的にNagi Desktopへ切り替える。
- Nagi DesktopのNagi BarからNagi Homeへ戻れる。
- HomeとDesktopの切り替えで会話、Task、Workspace、アプリ状態を失わない。
- NagiへTaskを委任した後は、作業中も原則Nagi Homeを維持し、進捗と必要なTask操作へHomeから到達できる。委任Taskの実行だけを理由にDesktopへ自動切替しない。ユーザーがHomeからアプリを直接起動した場合のDesktopへの切替規則は維持する。

### Workspaceの切り替え（複数ディスプレイ対応・選択C）

- **通常のWorkspace切り替えは、利用中の複数ディスプレイを含むWorkspace全体の切り替え**を基本とする。
- 例外として、**ユーザーが指定したディスプレイは、別のWorkspaceを表示したまま維持**できる。ユーザーは「メイン画面だけ調べ物に切り替えて」のように、特定の画面だけを切り替えることもできる。他の指定外ディスプレイの表示は維持する。
- Nagiによる自然言語・音声指示を使える方向とし、Nagi Barや従来のキーボード・マウスからもWorkspaceを操作できる設計とする。具体的なショートカット・選択UI・音声コマンド文法は未確定。
- 切り替え前後でWorkspaceのウィンドウ配置、アプリ、会話、Task等の状態を維持し、固定ウィンドウやユーザーの手動配置を尊重する（DEC-010）。保持・復元できない状態は正確に示す。
- **Workspaceの画面切り替えのみで、中断済みAI Taskを自動再開・再実行しない。** 切り替え操作が未承認のデータ操作・権限拡大・新規Taskを意味してはならない。
- Nagi HomeとNagi Desktop自体の双方向切り替え、および「前回の続き」による再開とWorkspace切り替えを混同しない。

### Nagi Homeの初期表示

- 初期表示はシンプルな外観とNagiとの対話を中心にする。
- 画面中央に短い挨拶とテキスト・音声の入力欄を配置する。
- 「前回の続き」と「進行中のTask」を控えめなカードとして表示する。
- 会話履歴と作業状態は保持するが、ログイン時に前回の会話全文を自動表示しない。
- 過去のTaskをログイン時に自動再実行しない。

### 「前回の続き」

- ユーザーがカードを選択すると、前回のWorkspaceをNagi Desktop上に復元する。
- 復元可能なアプリ、資料、作業状態を戻し、Nagiが前回の進捗を短く説明する。
- 中断していたAI Taskは、ユーザーが明示的に「再開」するまで実行しない。
- 復元できない状態やアプリ状態は、復元成功として扱わない。部分復元・復元不能をユーザーに正直に示す。

## PX-01 — 対話中心の統合Home B＋将来の高度適応Home C（2026-11-17）

### CONFIRMED — B案を基本とする

Nagi Homeは、既存仕様の「**対話中心でシンプルなログイン後の標準画面**」を保ちつつ、ユーザーが必要に応じて**会話・アプリ・Task・Workspace等の日常操作の入口へ分かりやすくアクセスできる統合Home体験**を基本とする。Homeは会話、作業の再開と進行中Taskの把握・操作の中心として機能し、アプリの本格的なGUI作業はNagi Desktopと自然につながるようにする。入口を統合することは、固定のランチャや常設パネルを採用したり、Homeを情報量の多いDashboardへ変更したりする決定ではない。

この高水準Bは既存の具体的な画面規則を置き換えない。**Homeからアプリを直接起動したらDesktopへ自動切替**し、**委任したTaskの実行だけではDesktopへ切り替えず原則Homeを維持**する。DesktopのNagi BarからHomeへ戻れ、双方向の切替でConversation／Task／Workspace／App状態を失わない。既存のログイン後の短い挨拶・中央のText／Voice入力・控えめな「前回の続き」「進行中のTask」カード、前回WorkspaceのDesktop上での復元、復元の限界の表示と停止Taskの無断再実行禁止も維持する。

### CONFIRMED — 将来C案へ高度化する方向

B案の対話中心・ユーザー主導を保ちつつ、[DEC-010](DEC-010-intent-first-adaptive-ui.md)のIntent-first方針に沿い、**現在のProject・Task・利用目的／状況等に応じて、Home上の情報や操作候補・パネルの見せ方をより適切に調整する高度な適応Home体験**へ発展させる。利用者の明示設定と手動選択をAIの推測より優先し、Homeの構造が予期せず変更されて通常操作が阻害されないようにする。特定の推薦アルゴリズム・新しいTask起動方法・カード生成基盤・版別導入計画をこの決定で確定しない。

### 維持する既存CONFIRMEDと安全境界

- Home／Desktopの切替やHomeでの提案・適応表示は、既存Workspaceのウィンドウ配置・固定ウィンドウ・複数ディスプレイのユーザー指定を勝手に書き換えない。DEC-010の「新規Workspaceでは目的に沿う許可済み自動配置、既存Workspaceは提案とユーザー選択を基本」の区別を維持する。
- AI／Modelが停止・未導入・利用不能でも、通常のDesktop／GUI、キーボード・マウスによるOS・アプリ操作を維持する。Bの統合Homeや将来Cの適応を、通常操作のAI依存化・起動不能の理由にしない。アクセシビリティの先行整備とAI-nativeの将来方向の分離はDEC-097を継承する。
- 「前回の続き」、Workspace切替、HomeのTask入口の表示・選択だけで、停止・復旧待ちのAI Taskや副作用Actionを無断で再開・再実行しない。Task進捗・承認はDEC-011、Workspace復元はDEC-014に従い、Permission／Capability／Policy・Privacy／Memory／Profile・Waybackの既存境界を保持する。
- この決定は製品の高水準な画面体験の方向であり、実装完了・性能達成、0.1／0.2／0.3／0.4+のRelease／Activation／Acceptance Gate、Workstream開始条件を変更しない。

### PX-01の未決事項（PROVISIONAL／OPEN）

- Home／Desktopの具体レイアウト、操作導線・Navigation、情報の配置と優先順位、追加カードの種類・件数、Nagi Barの位置・見た目、画面切替の細かなUI／Animation、アクセシビリティの具体契約。
- Homeのアプリ／Task／Workspaceへの入口の具体機能・提示条件、適応Homeのパネル切替・固定・却下・訂正等のUI、Project／Task文脈の取得・判定と個人化の方法、表示負荷や復元中の動作。
- AI停止時の利用可能状態・代替操作・復旧案内の詳細UIと失敗時の表現、使用技術・Framework・Provider・性能・受入方法、対象端末・実装時期・提供Version。

## 例外・禁止・安全境界

- Nagi-firstは従来GUIの削除や利用制限を意味しない。AIが使えないことを理由に通常のOS操作を停止しない。
- アプリ起動やその他のActionは、ユーザー操作または承認されたTaskに基づき、正規のOS APIと現行Permission / Policyを通す。
- 画面切り替えやWorkspace復元を、AI Taskの再開・Actionの再実行と同一視しない。
- 会話履歴や作業状態の保持は、すべての会話を無期限の長期Memoryとして保存することや、Modelへ会話全文を自動投入することを意味しない。保存・分類・検索・外部共有・忘却・削除はDEC-002〜005等の既存Policy / Permissionに従う。
- このDECは製品体験の決定であり、実装開始、release activation、0.1・0.2・0.3・0.4+の実装Gateや受入条件を変更しない。

## 未確定・暫定案

- Home / Desktopの具体的なレイアウト、文言、Nagi Barの配置、切り替え操作とアクセシビリティ。
- Workspaceの一覧/選択UI、画面ごとの保持指定・解除操作、単独画面と複数画面の切り替え優先順位、フォーカス遷移、固定ウィンドウの表示位置、対応アプリの具体的な状態保持方式。
- 「前回の続き」と「進行中のTask」カードの表示条件、件数、優先順位。
- Workspaceおよび各アプリ状態の保存・復元方式、対応範囲、未対応アプリの表示方法。
- 復元の部分成功・失敗時に提示する詳細、再試行・復旧操作。
- Homeから起動したアプリをDesktopへ切り替える具体的なタイミングと複数ウィンドウの扱い。
- 会話履歴、Task、Workspace状態の保存範囲と保持期間、削除・復旧の詳細。画面切り替えと「前回の続き」に必要な状態連続性を保ちつつ、Memoryとデータ保持の条件はDEC-002〜005に従う。
- 本決定を各リリースへ割り当てる時期と、各リリースの追加Acceptance条件。既存仕様とGateに従って別途定める。

## Roadmapとの関連

- **0.1 / 0.2:** AIが利用不能でも通常のOSを操作できること、非特権AIと既存Permission境界を維持する。新しいHome/Desktop UIや実装を0.1 / 0.2 Gateへ追加しない。
- **0.3:** Personal Agentと音声・Public Actionのロードマップ方針に沿う。既存の0.3 Implementation Spec作成・Activation Gateを維持する。
- **0.4+:** Nagiをユーザー向けの一貫した窓口とする既存方針に沿う。別途Activationが必要であり、本DECは0.4+実装を許可しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-10-29 | Nagi-first、Nagi Home / Desktop、ログイン時表示、Workspace再開時のTask実行・復元境界を確認 |
| 2026-11-04 | 選択Cを確定。通常は複数ディスプレイを含むWorkspace全体を切り替え、ユーザー指定の画面を維持または単独切り替えできる。状態保持・手動配置・Task再実行禁止を継承 |
| 2026-11-15 | Nagiに委任したTaskの作業中は原則Nagi Homeを維持する方針を確定。アプリの直接起動時にDesktopへ切り替える既存規則は維持する。 |
| 2026-11-17 | PX-01でB（対話中心でシンプルさを保ち、日常操作への入口を統合したHome）を基本、将来C（Project／Task等に合わせた高度なHome適応）を高水準の方向としてCONFIRMED。ログイン時Home、Homeからのアプリ起動でDesktop切替、委任Task中のHome維持、Workspace復元、AI不在時の通常GUI、DEC-010の既存Workspace保護を維持。具体UI・技術・提供版はPROVISIONAL。 |

## Foundation #28の後続決定（2026-11-16）

[DEC-097](DEC-097-accessibility-foundation-and-ai-native-long-term-direction.md)により、通常a11y基盤を先行整備し、将来のAI-nativeで正確なポインティングに依存しない操作思想と両立させる方針を確定した。Home／Desktopの切替やナビゲーション等の具体的なa11y方式は引き続き未決定。GUI・keyboard／mouseとAI不在時の通常操作を維持する。
