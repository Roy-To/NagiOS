# DEC-087 — Audio System：OSS音声基盤とNagi Audio Service、実測に基づく部分最適化

**状態:** CONFIRMED（Foundation #21の条件付きB：Linux／OSS音声基盤の再利用、Nagi独自Audio Service、低遅延・省メモリ・省電力の実機測定と不足箇所の個別最適化方針）／PROVISIONAL（ALSA／PipeWire等の具体採用、実装・対応機器・指標・Release・受入）  
**最終合意日:** 2026-11-16  
**適用候補:** Nagi OS Foundation #21。適用Release未確定  
**関連:** [Foundation Checklist #21](NAGI_OS_FOUNDATION_CHECKLIST.md#f21)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-068](DEC-068-nagi-sleep-resume-recovery.md)、[DEC-086](DEC-086-input-system-cross-device-linux-foundation.md)、[0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[Roadmap索引](roadmaps/README.md)

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

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #21の「条件付きB」を明示承認。Linux／OSS基盤を基本とし、Nagi Audio Serviceで音声優先度・機器切替・マイク権限・Voice連携を管理。低遅延／省メモリ／省電力を実機で測定し、要求に不足する箇所だけ独自最適化または部分的な置換を検討。ALSA／PipeWire等の個別採用・性能数値・機器・版は未確定。 |
