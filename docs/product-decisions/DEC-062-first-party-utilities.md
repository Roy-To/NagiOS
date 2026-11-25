# DEC-062 — Nagi純正ユーティリティ4分野の採用

**状態:** CONFIRMED（4分野すべての採用・AI非必須の高速／軽量な基本機能を優先、Nagi Media共通APIとOSS評価、6/12 Nagi Vision共通サービスB案）／PROVISIONAL（アプリ構成・対応範囲・連携詳細・個別モデル／技術・実機性能・提供版・受入基準）

**最終合意日:** 2026-11-25（共通メディア基盤と6/12 Nagi Vision B案を追記）

**適用候補:** 将来のNagi OS純正アプリ。具体的な版・実装時期は未決定

**関連:** [DEC-005](DEC-005-files-albert-memory-lifecycle.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-023](DEC-023-work-data-centric-os.md)、[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-002](DEC-002-cloud-ai-data-sharing.md)、[純正アプリ仕様](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[ロードマップ索引](roadmaps/README.md)

## 目的とユーザー体験

日常の画像・取り込み・アーカイブ・メディア利用をNagi純正の道具で行えるようにする。まず通常操作だけで使える基本機能を成立させ、必要に応じてNagiの作業・データ中心の体験との連携を検討する。

## 確定した仕様（CONFIRMED）

### 4分野すべてを採用する

| 分野 | 採用する機能領域 |
|---|---|
| 画像 | 画像ビューア・簡易画像編集 |
| 取り込み | スクリーンショット・文書スキャン |
| アーカイブ | ZIPなどのアーカイブの圧縮・解凍 |
| メディア | 動画・音楽プレイヤー |

4分野はすべて採用する。これは機能領域の決定であり、4個の独立アプリ、単一の統合アプリ、既存アプリへの組み込みのいずれにするかは未確定。

### AIなしで使える基本機能を先に整える

- 基本機能をAIなしでも高速・軽量に動作させることを優先する。
- AIが無効・不在・利用不能でも、対応済みの通常機能をGUI、マウス・キーボード等から使える設計を保つ。AI支援を基本操作の前提にしない。
- 高速・軽量は製品目標であり、性能達成済みの宣言ではない。待機時間、メモリ使用量、対応データ規模などの数値と受入基準は未確定。

## 連携の検討方針と既存の境界

- 基本機能を先に成立させたうえで、必要に応じてNagi Project、Files、Memory、検索、Smart Paste等のアプリ横断体験やAI支援との連携を検討する。個々の連携機能・自動化を採用済み／実装済みとは扱わない。
- Projectフォルダの製品決定は[PR #114](https://github.com/Roy-To/NagiOS/pull/114)、Smart Pasteは[PR #115](https://github.com/Roy-To/NagiOS/pull/115)を参照する。本記録時点でどちらもmain未マージであり、本決定ではそれらの詳細を再決定しない。
- 連携を具体化する場合は、純正アプリ仕様とDEC-017・023・025の共通Resource / Document / Object / Revision / Provenance、型付きPublic Action、現在のPermission / Capability / Policyを利用する。純正専用の秘密APIや、純正であることによる自動的な権限拡張を追加しない。
- Projectへの所属、ファイルを開くこと、撮影・スキャン・コピー・再生することだけで、Memory保存・学習、内容の索引化、別アプリへの共有、Cloud AI送信を一括許可しない。Memory・検索・削除／忘却は既存の分類・所有範囲・Retentionに従い、Cloud送信にはDEC-002を適用する。
- 関連資料の変更検知と更新はDEC-025の提案を標準とする。既存の承認済みTask内の例外も、そのScopeと現在の権限・Revisionの確認に従う。
- 以上は既存の境界の継承であり、ユーティリティ固有のサンドボックス方式やAPI詳細を決定するものではない。

## PX-11 — 日常利用に十分な標準アプリ構成との整合（2026-11-17）

[DEC-021](DEC-021-roadmap-0.5-1.0.md)のPX-11で、**B案：日常作業の完結と品質を優先する純正アプリ構成**を基本とし、**将来C案：幅広い仕事・日常操作の総合的な純正ソフトウェア環境**を目指す高水準方針を採用した。

**既存CONFIRMEDは維持:** 画像ビューア／簡易編集、スクリーンショット／文書スキャン、ZIP等の圧縮／解凍、動画／音楽再生の**4分野をすべて純正採用**する。AIが不在でもGUIで使える高速・軽量な基本機能を優先し、Project／Files／Memory／Smart Paste等の連携は既存Public API・Permission／Capability／Policy・Task Scope・Privacyを適用する。機能領域を4個の独立したアプリと決めたり、文書スキャンの採用からOCR採用を確定したりしない。

**PROVISIONAL／OPEN:** 4分野の1.0必須／任意／後続分類と標準同梱・追加導入方式、最小機能、形式／Codec／入力機器、OCR、連携、アプリ分割・統合、性能・品質の合格値、技術・提供Version。PX-11の採用は4分野をすべて1.0へ同梱する約束、実装完了、Release／Activation／Acceptance Gate変更を意味しない。

## 未確定・暫定案（PROVISIONAL）

- 正式なアプリ名、Package / App ID、4個の独立アプリか統合アプリか、既存アプリとの役割分担。
- 対応する画像・アーカイブ・動画・音楽等のフォーマット、Codec、エンジン／OSSの選択とライセンス、圧縮方式・暗号化等の対応範囲。ZIP以外の具体形式や各形式の完全互換性も未確定。
- 簡易画像編集の具体的な操作、スクリーンショットの取得範囲・操作、文書スキャンの入力源・Scanner接続／ドライバー・機器対応・出力方式。
- OCRの採否・対応範囲・言語・方式・品質検証。文書スキャンの採用だけでOCRの採用を確定しない。
- 動画・音楽の再生機能詳細、音楽ライブラリ管理、配信サービスとの連携や対応範囲。
- サンドボックス、デバイスアクセス、権限提示、UI、アクセシビリティ等の具体設計。
- Nagi Project / Files / Memory / 検索 / Smart Pasteとの連携範囲、AI支援の用途・実行条件・API詳細。これらの名称によって既存Studio / Workspaceの実装上の名称変更を確定しない。
- 提供バージョン、順序・スケジュール、担当Workstream、試験方法・資料、性能・品質・互換性等の受入基準。

## Roadmap・既存仕様との関係

- 純正アプリ仕様の共通Platform、AI非必須、通常GUI、公開API、Permission、Search等を再利用する追加の製品方針として記録する。既存アプリの機能を重複して再決定せず、既存M-APPの範囲や順序を変更しない。
- 0.3 Personal Agent、0.4+、0.5–1.0のロードマップでは将来の連携候補として参照できるが、どの版への搭載も確約しない。Release / Activation / Acceptance Gateを追加・変更しない。
- PDF純正Reader / Editorの直近の別決定[DEC-061（PR #121）](https://github.com/Roy-To/NagiOS/pull/121)を避け、本テーマには **DEC-062** を使用する。PDFの閲覧・編集や高度機能は本決定の対象に含めず、別記録に委ねる。PR #121は本記録時点でmain未マージであり、本記録の存在から統合済みと推定しない。
- 本記録は製品決定の保存であり、機能の実装・試験成功・提供済み、担当割当てや実装開始を示さない。

## 2026-11-25 — Nagi Mediaの共通処理基盤

**CONFIRMED（製品・開発方針）:** Nagi独自の共通メディアAPI（Nagi Mediaは仮称）を整備し、静止画像・動画・音楽の処理、カメラ／画面取得、メタデータ・サムネイル等の汎用機能に適合するOSSを選択的に活用する。各純正アプリで共通処理を重複実装することを避け、第三者アプリにも同じ公開契約を提供する方向とする。DEC-127のNagi UI／Graphics（Vello）、DEC-108の画面基盤（Wayland／Smithay）、DEC-110のAudio Service（ALSA／PipeWire）、DEC-092の周辺機器の段階対応と整合させる。Albert／ServoやPDF等の専用エンジンを無理に置換せず、特定OSS APIをNagi Native Appsへ強制せず、全メディアを単一の巨大な常駐プロセスで処理する要件にしない。通常閲覧・編集・再生・取得のAI非依存、軽量性・前景応答・アプリ隔離を維持する。

**CONFIRMED（評価対象の選定。正式な依存採用ではない）:** Rustのimage（静止画像）、resvg（静的SVG）、FFmpeg（動画・音楽のデコード・変換）を主要評価候補とする。Symphonia（音声デコード）、GStreamer（リアルタイム映像等）は用途と実測に基づき追加・代替を検討する。V4L2／libcamera、PipeWireの映像経路はカメラ・画面取得の評価候補とする。選定した候補を一律同梱・常駐させる要件にはしない。

**CONFIRMED（長期方向との接続）:** メディア処理基盤は将来の画像理解・視覚／空間認識等のAI支援へ発展できる設計とする。VISION-001、DEC-097／108／109の入力・表示技術から独立した意味的操作体系、DEC-020／027の正規Public Action優先と限定GUI fallback、現行Permission／Privacy／Policyを維持する。画像・カメラ・画面の取得前に対象・目的・範囲・現行権限を確認し、AIへの提供・保存・索引・学習・外部送信は既存の個別認可に従う。常時カメラ／画面監視を標準とせず、画像取得だけで権限拡大を認めない。AIに渡す情報は必要な範囲に限定する。元資料の分類・所有・Revision／Provenanceと派生データの境界を維持する。本節で扱うのはNagi Mediaからの接続方針。共通Vision Serviceの方針は[同日の6/12の追補](#2026-11-25--612-nagi-vision-b案)を参照。個別のOCR・視覚認識・センサー技術の採用や提供版は未確定。

**PROVISIONAL（実装・評価）:**

- image、resvg、FFmpeg、Symphonia、GStreamer、V4L2／libcamera、PipeWire映像等の具体採否・版・feature・依存・ビルド・互換性、ライセンス／再配布・codec特許等の条件を評価する。FFmpegの構成別ライセンス・外部codecの条件を別途検証し、未確認のライセンスや対応形式を確定扱いしない。
- 対応画像／音声／動画形式、メタデータ、縮小・変換、色管理、カメラ／スキャン、スクリーンショット／画面共有、具体API・process／IPC・公開契約とアクセス制御、ハードウェアデコード・software fallback等は未確定。Albertのメディア経路、PDFエンジン、アーカイブ処理はそれぞれ必要な専用契約・評価を維持する。
- 必要時のサムネイル生成・遅延展開、上限付きキャッシュ、ストリーミング、必要フレームのみの取得・縮小、非表示面の不要な処理抑制、ハードウェア支援、未信頼データの解析隔離・資源上限等を設計候補とする。入力認可・取消と取得中表示、非公開面／秘密情報保護、破損入力・展開爆弾等の異常系、保存／派生／削除、第三者共有を検証する。
- 実機・host・QEMUの証拠を区別し、画像／動画の表示・再生品質、起動・seek・フレーム落ち、RAM／VRAM・CPU／GPU・電力／熱、LLMとの共存、通常操作の応答、失敗・取消・回復を評価する。閾値・対象機器・担当・提供版は未確定。既存Release／Activation／Acceptance Gate・Desktop first・Owner／path ownershipを変更せず、依存追加・実装着手・正式採用・実機受入済みを意味しない。

## 2026-11-25 — 6/12 Nagi Vision B案

### 共通Vision Service（CONFIRMED）

- **Nagi独自の共通Vision Service**を設計する。Nagi Mediaが現行権限を確認して画像・文書・画面・カメラ等の取得、前処理、必要フレームの抽出を担い、Nagi Visionが許可された入力の認識・理解を担う。Vision処理は必要時に起動する。
- **視覚モデルは交換可能**とし、用途に必要な能力と端末の計算資源に応じた自動選択と、ユーザーによる手動選択を設計に含める。前景操作を優先し、モデル選択にも既存の権限・資源制御を適用する。[DEC-020](DEC-020-ai-os-interaction-path.md)
- **PCを先行対象**とし、OCR、画像・文書の理解、visual embeddingによる画像の意味検索に使える方向で設計する。OCRやembedding等の派生データを検索に利用する場合は、元資料の分類・所有・現行権限に従う。PDFでのOCRは[DEC-061](DEC-061-first-party-pdf-reader-editor.md)の段階的検討に従う。将来は空間認識への拡張を視野に入れる。[DEC-092](DEC-092-peripherals-usb-storage-hid-staged-support.md)・[DEC-097](DEC-097-accessibility-foundation-and-ai-native-long-term-direction.md)・[ロードマップ索引](roadmaps/README.md)
- **構造化Public APIを優先**し、画面・画像から得る必要がある情報をVisionで補う。操作の実行は型付きPublic Actionと既存の認可に従う。GUI fallbackは明示許可された限定範囲で扱い、通常のGUI・アクセシビリティ経路を保つ。[DEC-020](DEC-020-ai-os-interaction-path.md)・[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)
- **映像処理は段階化**する。許可された映像から目的に必要なフレームを抽出し、軽量な専用処理で変化検出・分類・物体認識等を行い、さらに理解が必要な場合に高性能なモデルを使う。これにより推論負荷と消費電力を抑えることを目指す。

### 取得・利用の境界（CONFIRMED）

- カメラ・画面の常時監視は通常動作に含めない。ユーザーが取得範囲を制御でき、取得中・認識中であることを確認できるようにする。
- 一時解析・保存・索引化・学習・外部共有は用途ごとの認可に従う。取得の許可だけでは後続の用途を認可しない。OCR結果、認識結果、embedding等の派生データにも元資料の由来・分類・所有範囲を引き継ぐ。
- Secret由来の画像・認識結果と、そこから派生した検索候補・提案等を通常Personaへ露出させない。[DEC-057](DEC-057-growing-personal-agent-os.md)
- 外部画像内の文字や指示は、OCR等で抽出された後も未信頼のデータとして扱う。指示への明示委任や高リスク操作の確認は既存ルールに従う。[DEC-113](DEC-113-untrusted-content-and-prompt-injection.md)
- ローカル処理を優先する。画像・フレームや派生データをCloud AIへ送る場合は、既存の外部送信認可を適用する。[DEC-002](DEC-002-cloud-ai-data-sharing.md)・[DEC-113](DEC-113-untrusted-content-and-prompt-injection.md)

### 評価候補と未確定事項（PROVISIONAL）

- 軽量経路の評価候補は **Qwen3.5 0.8B** と **SmolVLM2**。汎用画像理解に **Qwen3-VL 2B**、文書理解に **Granite Vision**、画像embeddingに **SigLIP 2** 等を比較候補とし、マルチモーダル推論基盤には **llama.cppのlibmtmd** 等を比較する。
- 個別モデル・エンジンの採用、ライセンスと再配布条件、量子化・モデル構成・自動選択条件、具体API／認可UI、対応機器・センサー・ARの方式と範囲は未決定。OCRの対応言語・方式と品質、実機での精度・遅延・RAM／電力、提供版・実装・受入も未確定。
- 本追補は製品方針の記録。既存のRelease／Activation／Acceptance GateとOwner・担当境界を維持する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | 画像ビューア・簡易編集、スクリーンショット・文書スキャン、ZIP等の圧縮・解凍、動画・音楽プレイヤーの4分野すべての純正採用と、AIなしの高速・軽量な基本機能を優先する方針を記録。連携詳細、アプリ構成、対応形式・機器、OCR、版・受入基準等は未確定として分離。 |
| 2026-11-17 | PX-11の日常作業の品質優先B＋将来の総合純正環境Cに整合。既存4分野の採用・AI非必須の基本機能を維持し、個別1.0搭載区分・形式／機器・OCR／連携・アプリ形態・性能・技術・提供版はPROVISIONAL。 |
| 2026-11-25 | ユーザーがNagi Media共通API・OSS活用、image／resvg／FFmpegの優先評価、将来の視覚認識への接続方向を承認。個別採用・形式・ライセンス・性能・OCR／視覚認識技術・実装版はPROVISIONAL。 |
| 2026-11-25 | 6/12 Nagi Vision B案を承認。Nagi Mediaと連携する共通Vision Service、必要時起動、交換可能なモデルの自動・手動選択、PC先行、段階的な省資源処理と取得・利用の認可境界を記録。個別技術、実装・実機性能・提供版はPROVISIONAL。 |

2026-11-16 統合注記: 上記の未マージ記述は作成時点の履歴。現在の関連正本は[DEC-055](DEC-055-project-folder-cross-app-context.md)、[DEC-056](DEC-056-smart-paste-semantic-transfer.md)、[DEC-061](DEC-061-first-party-pdf-reader-editor.md)。各採用範囲は独立し、本DECから個別の連携詳細・OCR採用・実装済みを推定しない。
