# DEC-110 — Audio System：OSS音声基盤とNagi Audio Service、実測に基づく部分最適化

**状態:** CONFIRMED（Foundation #21の条件付きB：Linux／OSS音声基盤の再利用、Nagi独自Audio Service、低遅延・省メモリ・省電力の実機測定と不足箇所の個別最適化方針、ALSA／PipeWire採用、交換可能な音声AI、Nagi Voiceの優先評価方針）／PROVISIONAL（具体構成・個別音声モデル、実装・対応機器・指標・Release・受入）\
**最終合意日:** 2026-11-25\
**適用候補:** Nagi OS Foundation #21。適用Release未確定\
**関連:** [Foundation Checklist #21](NAGI_OS_FOUNDATION_CHECKLIST.md#f21)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-068](DEC-068-nagi-sleep-resume-recovery.md)、[DEC-109](DEC-109-input-system-cross-device-linux-foundation.md)、[0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[Roadmap索引](roadmaps/README.md)

## 目的とユーザー体験

PC、タブレット、スマートフォンにまたがるNagi OSで、音楽・動画などの一般的な再生／録音、マイク入力、アプリごとの音量・出力経路・混合、Nagiとの音声会話を支える。既存OSSの成熟したオーディオ処理を使いながら、Nagi固有の音声優先順位・権限・AI連携をOSサービスとして設計する。

## 確定した製品方針（CONFIRMED：条件付きB）

1. **基盤はLinux／OSSを再利用する。** 音声のデバイス入出力、ミキシング、ルーティング等には適切なLinux機構と既存OSSを積極的に活用し、全面独自の音声エンジンを標準方針としない。ALSA／PipeWire等は候補であって個別採用確定ではない。
2. **Nagi固有の管理はAudio Serviceに置く。** Nagi Audio Service等のNagi所有の契約を通じて、音声入出力の機器選択・切替、一般アプリの再生／録音、アプリ間の音声優先度、マイクの利用許可、Nagi Voice／音声会話連携を扱う。アプリの通常再生／録音を毎回LLM推論に依存させない。
3. **低遅延・省メモリ・省電力は実機で測る。** PC・タブレット・モバイル等の対象端末と実利用シナリオについて、体感だけでなく音声遅延、メモリ負荷、電力等の実測を行い、採用OSS構成が製品要求を満たすかを評価する。目標数値・合格値・測定方法は本決定では固定しない。
4. **不足する部分だけ最適化／置換を検討する。** 実測で要件不足が判明した場合、OSSの設定・処理経路の調整、Nagi側の部分的最適化、必要なら限定的な独自実装・置換を検討する。計測前の全面自作や、計測を理由としない独自化を必須としない。
5. **Nagiの既存安全・通常操作契約を維持する。** 再生・録音・アプリ操作はAI不在でも利用でき、Nagi AI／Agentは非特権のままPermission／Policyに従う。マイクなどの収録機器へのアクセス、音声内容の共有・外部送信・記録を、単なるデバイス認識やAudio Serviceの存在から自動許可しない。

## 例外・禁止・安全境界

- 音楽の停止・音量調整・録音開始や音声出力切替等のアプリへの指示は、Nagiの正規Service／Public Actionと現行User／Profile／Permission／Policyの経路で行う。音声入力を得たこと自体でアプリやデータの無条件操作権限は生じない。
- 録音・音声会話に使うマイクの使用状況・同意・取り消しを尊重し、音声内容を無許可でAI Memory・外部Cloud・別アプリへ流さない。
- 機器の接続、再接続、Bluetooth音声やスリープ復帰で、以前の入力／出力設定と許可の現行状態を再確認する。対応ハードウェア／codecがない場合、利用可能だと偽らない。
- Linux既存基盤やOSS採用は既存Linuxディストリビューションのrootfs／ユーザーランド一式の採用ではない（DEC-043）。
- 既存Release／Activation／Acceptance Gate、実機候補、Desktop firstを変えない。本製品決定を音声／Bluetooth実機受入の完了や性能達成と混同しない。

## 未確定事項（PROVISIONAL／OPEN）

- ALSA、PipeWire、Bluetooth音声、plugin等の個別OSS選定とライセンス、構成、固定revision／patch、常駐コスト。
- Nagi Audio Serviceの具体的プロセス・API・stream／session／権限model、mixing／routing・volume・priority・ducks／interruptionのUI・詳細ポリシー。
- Voice Runtime／ASR／TTSとAudio Serviceの境界、発話中の割り込み、echo cancellation・音声処理の採用範囲、通常音楽／動画と同時利用時のUX。
- USB・Bluetooth・内蔵audio、出力／入力device切替、sleep後の復旧、対応codec／firmware・実機matrix。
- PC／タブレット／モバイルの評価シナリオ、遅延・メモリ・電力・音切れ（underrun）の指標・baseline・定量受入条件と測定結果。
- 実装順序・担当・Release／実機受入、OSS不足時にどの部分を独自化するかの後続判断。

## Roadmapとの関連

Nagi独自のOS体験とLinux／OSS再利用を両立する既存方針（DEC-043）、AIなしで使える通常操作と前景優先（DEC-020）、Nagi Sleep（DEC-068）と整合する。0.3の音声・Personal Agent構想への接続を想定するが、特定版の必須機能・受入条件・リリース時期・新しい実装Gateをここから追加しない。

## 2026-11-25 — ALSA・PipeWireの採用方針と交換可能な音声AI

**CONFIRMED（技術選定・製品方針）:** Linux上の音声デバイス入出力の基盤としてALSA、音声stream／mixing／routingの主要基盤として[PipeWire](https://pipewire.org/)を採用する方向を確定する。Nagi Audio ServiceはNagi独自の公開契約・機器選択・音量・音声優先度・マイクPermission・Voice連携を担い、低レイヤー音声処理は適合OSSを活用する。Audio Serviceと音声認識／合成のVoice Runtimeを分離し、ASR／TTSは交換可能なOSS推論基盤・利用条件に適合するモデルを活用する方向とする。DEC-008／011の明示開始の音声会話、Text切替・訂正・停止、将来の明示有効化によるハンズフリー操作と、DEC-108／109の入力手段から独立した意味的操作体系を継承する。音声取得や認識はAuthority・本人認証・重要操作の最終承認の代替ではない。通常再生／録音の非LLM依存、非特権AI、Permission／Policy、Cloud送信の別許可、控えめな通知、前景UIと音声の優先、実測に基づく部分最適化を維持する。本追補はALSA／PipeWireの採否について従前の未確定記述に優先する。

**PROVISIONAL（具体設計・受入）:**

- ALSA／PipeWireの固定版・feature・依存とライセンス、Rust接続・process／IPC・session管理・権限の実効強制、WirePlumberの採否とNagi Audio Serviceとの責務分担、BlueZ／Bluetooth codec・USB／内蔵audio、必要なplugin／DSP等は別途評価する。画面共有・カメラへのPipeWire活用は別の検討事項であり、今回の音声採用から機能・権限の追加を推定しない。
- whisper.cpp、sherpa-onnx、Piper、Kokoro等は比較候補に留める。個別ASR／TTS／VAD／keyword検出・モデル・言語・音声品質、学習データ・重み・推論エンジンのライセンスと再配布条件、固定版は未確定。日英先行と将来10言語の目標（DEC-127）を尊重し、OS言語対応と音声モデル対応は分けて評価する。0.3改訂Roadmap §34.6／VOICE-01のoffline経路・fallback・取消を維持する。
- アプリ別音量、会話時のduckingと終了時の復元、会議中の通常音声通知抑制、音声割込み、echo cancellation、マイク使用表示・取消・収録／文字起こしの保存、Wake Word／常時待受は具体検討事項とする。無断収録・監視・学習・外部送信を認めない。通常の音声提案で自発的に割り込まないDEC-008を変更しない。
- 実機／QEMU等の条件を分け、低遅延・RAM・CPU・電力／熱・音切れ、機器切替／sleep復帰、AI負荷との共存、認可取消・障害時復旧を比較する。既存 `user/nagi-audio` の旧カーネル経路をLinux向けに統合・移行する具体方法、性能指標・合格値・実装担当・版は未確定。既存Release／Activation／Acceptance GateとOwner／path ownershipを変更せず、依存追加・実装開始・動作確認済みを意味しない。

## 2026-11-25 — Nagi Voiceの優先評価方針

**CONFIRMED（評価方針と製品方向）:** 交換可能なローカル音声認識・音声合成・発話検出を組み合わせ、Nagi独自の対話・意図解釈・操作・許可された個人適応に接続する方向を採用する。音声AIは日英を先行評価し、将来10言語の製品目標（DEC-127）への拡張を見据える。低スペック端末で通常操作とローカルLLMが共存できる軽量性、自然な音声会話、OSの前景応答を重視する。Nagiらしい標準音声とユーザーの好みに合わせた声質・話速・話し方を将来の方向として検討する。対応するローカルproviderと機器ではオフライン会話、明示開始、Textとの切替、認識訂正、音声停止・Task状態保持を目指す。DEC-008／011／020／060／097／108／109、DEC-110の既存安全境界と0.3改訂Roadmap §34.6／VOICE-01を維持する。

**CONFIRMED（優先評価する対象の選定。製品依存としての正式採用ではない）:**

| 領域 | 優先評価対象 |
|---|---|
| ASR | [whisper.cpp](https://github.com/ggml-org/whisper.cpp) と多言語 Whisper baseを比較基準とする |
| ASR比較・高速化 | Whisper tiny／small、[SenseVoiceSmall](https://huggingface.co/FunAudioLLM/SenseVoiceSmall)等 |
| TTS | [Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M)を第一評価候補とし、必要に応じてPiper、VOICEVOX、Supertonic等を比較 |
| VAD | [Silero VAD](https://github.com/snakers4/silero-vad)を第一評価候補 |
| 共通音声推論基盤 | [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)を有力評価候補 |

**PROVISIONAL（具体実装・受入）:**

- 個別モデル・推論エンジンの正式採用、固定版・量子化・provider接続、配布条件、コード・重み・辞書・音声データそれぞれのライセンス、言語／声質の対応、TTSの日本語フロントエンドとsherpa-onnxとの実際の互換性は未確定。候補を一律に同梱・常駐させる要件ではなく、日英の実用品質が不足する場合は代替モデルを比較する。独自音声モデル開発、標準音声の選定、声質・話速・個人適応の具体仕様と提供時期は別途判断する。
- push-to-talk／明示開始を出発点とし、VAD・streaming／部分認識・発話終了・TTSの先行再生・割込み／取消・echo cancellation・誤認識訂正、必要時load／unload、モデルの逐次実行・適応的切替等を比較する。Wake Word・常時待受・音声保持は未決定。音声認識結果や話者推定だけで本人認証や実行権限を与えず、重要操作の既存最終承認を維持する。マイク使用・音声／文字起こしの保存・学習・Cloud送信は既存の個別認可／Privacyに従う。無断録音・声の無断複製・通常提案の自発的音声割込みは認めない。
- DEC-116の8GB級基準を参照し、host／QEMU／実機を区別してASRの言語別認識品質（日本語CER等、英語WER等）、固有名詞、雑音／日英混在、発話終了から認識確定・応答開始までの遅延、TTSの自然さ・発音・先頭音声遅延、VADの誤検出／見逃し、RAM／CPU／GPU・電力／熱・音切れ・LLM／通常UIとの共存、認可取消・停止・fallbackを測る。具体数値・適用機器・評価データ・合格基準・版は未確定。候補の公称メモリ量や対応言語をNagi上の実測・品質保証として扱わない。
- 既存 `user/nagi-audio/src/speech.rs` のprovider契約・push-to-talk・マイク認可・表示・取消等の基礎を活用する案を検討する。現行の限定された言語enumや旧target経路が多言語／Linux音声会話の完成を意味するものではない。新規依存追加・Workstream着手・実装済み・特定Releaseへの新規必須要件を意味せず、既存Release／Activation／Acceptance Gate、Owner／path ownershipを維持する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #21の「条件付きB」を明示承認。Linux／OSS基盤を基本とし、Nagi Audio Serviceで音声優先度・機器切替・マイク権限・Voice連携を管理。低遅延／省メモリ／省電力を実機で測定し、要求に不足する箇所だけ独自最適化または部分的な置換を検討。ALSA／PipeWire等の個別採用・性能数値・機器・版は未確定。 |
| 2026-11-25 | ユーザーがALSA＋PipeWire採用、Nagi独自Audio Service、交換可能な音声認識・合成基盤と将来のハンズフリー連携を承認。具体構成・追加OSS・モデル・性能・提供版はPROVISIONAL。 |
| 2026-11-25 | ユーザーがNagi Voiceの技術評価方針を承認。whisper.cpp／多言語base、Kokoro、Silero VAD、sherpa-onnx等の優先評価を記録。個別モデルの正式採用・ライセンス・実機品質／性能・実装・提供版はPROVISIONAL。 |
