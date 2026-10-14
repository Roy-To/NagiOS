# Nagi OS

**NAGI — Native Agentic General Interface**

[English](README.md) · [日本語](README.ja.md)

> **実験的な Developer Preview です。** Nagi は独立した OS として開発中
> です。0.1 の対象は QEMU x86-64 参照マシンであり、完成した汎用 OS や
> 物理 PC 対応を示すものではありません。Nagi のプロジェクトライセンスは
> 未選定で、ライセンス付与も行われていません。

## プロジェクト概要

Nagi は、独自カーネルとユーザー空間を持つゼロベースの OS プロジェクトです。
Linux・Windows・macOS は開発ホストとして使いますが、Nagi は Linux
ディストリビューションではありません。現在の実装ラインは Nagi OS 0.1
Developer Preview で、公式ターゲットは QEMU x86-64 です。

通常の GUI・キーボード・ポインター操作を大切にしながら、利用者の意図に
沿った作業を AI が支援するローカルファーストのデスクトップを探求します。
AI はユーザー空間で動く信頼されない provider として扱います。状態を変更
する提案は、決定的な検証、ポリシー、Capability、Executor の境界を通す
設計ですが、エンドツーエンドの統合は開発中です。

この概要は source commit bcb92dc（2026-10-12）を基準に確認しました。
受け入れ結果と制限の正本は
[実装状況](docs/implementation_status.md)です。

## ビジョン

Nagi の開発理念は次のとおりです。

> **安全を既定に。強力な操作は利用者の選択で。**
>
> **制限するのはソフトウェアであり、所有者ではない。**

長期的には、利用者の意図を理解しつつ、マシンの権限を所有者の手元に保つ
AI ネイティブ OS を目指します。AI をカーネルの特権機能にせず、モデルの
確信度によって権限が増えることも認めません。

Local-first / Offline-first は設計の中心です。基本操作をクラウドアカウント、
クラウド LLM、API キー、テレメトリー、リモートストレージに依存させない
ことを目指します。ただし、現在の Preview の全機能・provider がオフライン
受け入れを完了したという意味ではありません。

英語と日本語は対等な第一級のユーザー言語です。Nagi OS 0.3 の Personal
Agent 構想では、ユーザー所有の Profile やモデル非依存の Memory へこの
設計を広げます。これは将来のアーキテクチャであり、0.1 の完成機能では
ありません。

## なぜ Nagi OS か

- **OS を OS として作る:** Linux 上の独自シェルではなく、独自カーネル、
  ABI、サービス、ネイティブデスクトップを持ちます。
- **利用者が権限を保つ:** ソフトウェアの操作権限を Capability と明示的な
  許可で決め、確率的な AI 出力に権限を与えません。
- **ローカルデータを基本にする:** ユーザーオブジェクト、履歴、将来の AI
  Context はローカル所有と復元可能性を重視します。
- **Agent がなくても使える:** AI が利用できない、または無効な場合も、
  通常の GUI・キーボード・ポインター操作を保つことが要件です。
- **英語と日本語を同等に扱う:** 両言語を第一級のユーザー言語とし、翻訳の
  完成度やアクセシビリティは引き続き整備します。

## 主な特徴

| 領域 | 現在の証拠 | 範囲と制限 |
| --- | --- | --- |
| 独立 OS とカーネル | M0–M18 は、それぞれの受け入れ範囲で PASS と記録されています。QEMU 参照マシンで Nagi のカーネルとユーザー空間が起動します。 | マイルストーン PASS は 0.1 全体の完成を意味しません。物理ハードウェアは 0.1 の対象外です。 |
| ネイティブデスクトップ | M10 の実 QEMU 受け入れは PASS。保存済み画像には Calculator、Notes、Files、Terminal のパネルがあります。 | 範囲を限定したデスクトップ受け入れ画面です。完成したアプリ群やインストーラーではありません。 |
| Albert ブラウザー | M18 の target CI は PASS: [run 37601664873](https://github.com/RT-NISH/NagiOS/actions/runs/37601664873)。HTTPS ページ 3 件、サイト権限、Clipboard、かな IME、Upload、ユーザー操作による Download を確認しました。 | 本番認証、通知表示、Servo StorageManager の永続化は未完了です。漢字変換は延期されています。 |
| Capability-based security | カーネルの Handle、IPC、Capability 検査に受け入れ済みの基盤があります。 | 本番セキュリティを保証するものではありません。アプリや AI 操作に対する認可統合は作業中です。 |
| Local-first とローカル AI | 実 QEMU で Granite 4.2 3B を Nagi 内で動かし、日本語の schema 制約付き応答を生成した記録があります。 | M20 は PARTIAL。モデルのインストールや通常の遅延起動サービスとの統合は未完了で、完成した AI チャットや Agent ではありません。 |
| Wayback と Activity Ledger | M15 のゲスト受け入れでは、永続履歴、バージョン、復元、Undo を確認しました。M22 には範囲を限定した History / Activity Ledger の証拠があります。 | 本番 Activity Ledger や AI 操作との一般的な統合は部分実装です。 |
| 英語と日本語 | M10 デスクトップに日本語テキスト経路があり、M29 QEMU 受け入れでは日本語 Settings の選択と再起動後の復元を確認しました。 | プロセス間の言語伝播、全体の翻訳、アクセシビリティ受け入れは未完了です。 |
| Personal Agent | Nagi 0.3 の設計資料では、Profile、モデル非依存 Memory、Learning、Capability を考慮した Agent 実行を構想しています。 | 将来ロードマップであり、0.1 の実装としては扱いません。 |

## アーキテクチャ

~~~text
UEFI / OVMF
    ↓
Nagi Loader
    ↓
Nagi Kernel — 実行・メモリー・IPC・権限
    ↓
nagi-init / Supervisor
    ↓
ユーザー空間サービス · Nagi ABI · SDK / PAL / relibc
    ↓
ネイティブアプリ · Albert ブラウザー (Servo)
~~~

カーネルは Linux ではなく Nagi 独自です。ファイルシステム、ネットワーク、
デスクトップ、オーディオ、パッケージ、AI ポリシーはユーザー空間に置く
設計です。Albert のブラウザーエンジンには Servo を使います。Preview の
一部サービスはまだ init 内で動くため、この図は目標とする境界を示しており、
本番レベルのサービス分離を意味しません。

詳細は
[0.1 実装仕様](docs/Nagi_OS_0.1_Codex_Implementation_Spec.md)と
[アーキテクチャ概要](docs/architecture/README.md)を参照してください。

## 現在の開発状況

source commit bcb92dc（2026-10-12）時点の概要:

| マイルストーン | 状態 | 概要 |
| --- | --- | --- |
| M0–M18 | PASS | カーネル・デスクトップ基盤と M18 Albert のゲストシナリオを含む、定義済みの受け入れを通過しています。範囲は各記録を確認してください。 |
| M19 | PARTIAL — 現在のマイルストーン | 範囲を限定したゲスト Search、Browser History 永続化、Files Search の証拠があります。本番アプリ向け Search grant、Files の Trash/Restore 統合、通常の Browser History producer 更新は未完了です。 |
| M20 | PARTIAL | ローカル Granite のゲスト推論は成功しています。[ホスト CI run 37702418216](https://github.com/RT-NISH/NagiOS/actions/runs/37702418216) は推論前の User Data 書き込み timeout で停止しました。モデルのインストールと通常サービスへの統合は未完了です。 |
| M21–M26 | PARTIAL | Validator、provider 境界、範囲を限定したゲスト実験があります。本番の一般的な Agent 操作、Semantic Search 統合、ページ要約、マイクから操作までの音声体験は受け入れ未完了です。Qwen 推論と Gemma 利用も受け入れられていません。 |
| M27–M30 | PARTIAL | 範囲を限定した Recovery、統合、日本語 Settings、Release bundle の確認記録があります。統合受け入れ、認証済み更新、再配布レビューは未完了です。M30 は PASS ではありません。 |

記録済みの M30 Release Manifest では
<code>m30_acceptance=NOT_EVALUATED</code> のままです。個別 QEMU シナリオや成果物の
checksum 検証だけで 0.1 リリース準備完了とはなりません。実行 ID、source
revision、制限事項は
[docs/implementation_status.md](docs/implementation_status.md) と各
workstream の記録を参照してください。

## スクリーンショット

以下は QEMU ゲスト受け入れ時の既存画像で、モックや動画の切り出しでは
ありません。各画像が示すのは、キャプションに記したチェックポイントです。

| M10 — ネイティブデスクトップ | M18 — Albert / Servo |
| --- | --- |
| ![Calculator、Notes、Files、Terminal を表示する M10 QEMU デスクトップ](docs/assets/screenshots/nagi-m10-qemu-desktop.png) | ![Nagi 内の Albert ブラウザーで HTTPS ページを表示した M18 画面](docs/assets/screenshots/nagi-m18-qemu-browser.png) |
| 固定パネルの受け入れ画面で、完成したアプリ群ではありません。 | 2026-10-01 撮影。2026-10-07 の M18 全操作 CI 受け入れより前の画像です。ページ描画の静止画で、全ブラウザー操作を示すものではありません。 |

### M29 — 日本語 Settings

![QEMU ゲストで日本語 Settings を選択した M29 画面](docs/assets/screenshots/nagi-m29-settings-ja-jp.png)

言語選択と同じ User Data を使った再起動後の復元に対応する受け入れ画像です。
全 UI の翻訳やアクセシビリティを示すものではありません。

この準備作業では動画を撮影・公開していません。
[QEMU 動画撮影計画](docs/oss/DEMO_RECORDING_PLAN.md)では、再確認した
ゲスト動作を撮影する手順をまとめています。

## ロードマップ

- **Nagi OS 0.1 — Developer Preview:** 独立 OS の起動、最初のネイティブ
  デスクトップ、ブラウザー、ストレージ、セキュリティ、ローカル AI の
  基盤を作ります。0.1 は開発中で、M19–M30 は PARTIAL です。
- **Nagi OS 0.2 — Platform Foundation:** Identity、Permissions、IPC、
  Settings、Model Runtime、Update、アプリ契約、Search、Activity/Wayback
  などを安定させる計画です。仕様上、0.1 M30 PASS と明示的な開始判断が
  前提です。ホスト基盤だけではゲスト実行時の受け入れになりません。
- **Nagi OS 0.3 — Agentic System:** ユーザー所有の Profile / Memory、
  モデル移行性、Capability を考慮した操作、Wayback による Undo を備える
  Personal Agent の将来アーキテクチャです。0.1 の機能ではありません。
- **Nagi OS 1.0:** 正式な公開 OSS リリースの長期目標です。日程は未定で、
  プロジェクトライセンスも未選定です。

[ROADMAP.md](ROADMAP.md)、
[0.2 実装仕様](docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)、
[0.3 Architecture Roadmap](docs/Nagi_OS_0.3_Architecture_Roadmap.md)も
参照してください。

## はじめ方

Nagi 0.1 の対象は QEMU x86-64、UEFI/OVMF、q35、仮想 CPU 4 基、RAM 8 GiB
です。pinned Rust toolchain とリポジトリに記載されたホスト依存関係を
使ってください。物理 PC 対応は 0.1 の完了要件ではありません。

POSIX 開発ホストでは、チェックアウトのルートから次を実行します。

~~~sh
./nagi doctor
./nagi fetch
./nagi build
./nagi test
./nagi image
./nagi run
~~~

fetch は pinned third-party source の取得にネットワーク接続と大きな
ディスク容量を要する場合があります。すべての任意モデルを取得するわけ
ではありません。build と test はホスト互換パッケージを対象とし、QEMU
マイルストーン受け入れの代わりにはなりません。Windows PowerShell の手順、
ホスト準備、トラブルシュートは
[Developer Preview ガイド](docs/developer-preview/README.md)を参照して
ください。現在のコマンドは <code>./nagi --help</code> で確認できます。

公開リポジトリを閲覧できても、ソースの利用・再配布権が付与されたことには
なりません。作業や共有の前に[ライセンスの状態](#ライセンスの状態)を
確認してください。

## コントリビューション

[CONTRIBUTING.md](CONTRIBUTING.md)、[AGENTS.md](AGENTS.md)、
[現在の実装状況](docs/implementation_status.md)、変更を担当する
workstream を先に確認してください。受け入れ証拠は source revision と
結び付け、確認済みの動作、部分実装、将来計画を分けて記録します。

再現可能な QEMU の不具合報告、範囲を絞った修正、ドキュメントの改善、
証拠や未完了項目を明らかにするレビューが役立ちます。コードを送る前に、
下記のライセンス状態とコントリビューション手順を確認してください。

## ライセンスの状態

Nagi の正式なプロジェクトライセンスは**未選定で、付与されていません**。
リポジトリが公開されていることは OSS ライセンスではなく、Nagi のコードや
ドキュメントを利用、複製、改変、再配布する許可を意味しません。候補ライセンス
を付与済みのものとして扱わないでください。

サードパーティのソース、フォント、モデル重み、音声データ、画像にはそれぞれ
別の条件があります。[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) と
[Legal ドキュメント](docs/legal/README.md)を確認してください。正式な公開
OSS リリースは、ライセンスとリリースのレビュー完了後、Nagi OS 1.0 を
目標としています。
