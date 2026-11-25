# Nagi OS 0.1 Developer Preview — 公開範囲B（草案）

> 公開ドキュメントの草案です。正式な受入記録でも、ダウンロード可能な
> イメージが完成したという告知でもありません。配布物名とqcow2のファイル名は
> 未決定です。

## 対象範囲

0.1の公開範囲Bは、Nagiのデスクトップと標準アプリのNotes、Files、Settings、
Calendarです。NagiはLinuxカーネル上で動くNagi独自のユーザーランドで、
汎用Linuxディストリビューションを土台にしません。この範囲の決定は、
アプリの全動作やリリース受入が完了したことを意味しません。
[ロードマップ](../../ROADMAP.md)と
[DEC-123](../product-decisions/DEC-123-kernel-unification-and-linux-0.1.md)を
参照してください。

## 現在の開発イメージを起動する

この草案の基点となるリポジトリには、固定されたリリースqcow2、
`SHA256SUMS`、リリース用ソースタグ、build manifestがありません。以下は
準備済みの**開発イメージ**を起動する手順であり、公開範囲Bの配布物を
インストール・起動する手順ではありません。

`./nagi doctor` が示す依存関係を満たすホストで、`preflight.json` と検証済みの
起動入力を含むTask24実行ディレクトリを用意し、リポジトリのルートで実行します。

```sh
./nagi doctor
python3 tools/linux-boot/make_dev_bundle.py path/to/prepared-task24-run
./nagi run
```

既定のバンドル出力先は `out/linux-dev/` です。`./nagi run` はQEMUの開発イメージを
起動し、バンドル内の書き込み可能なDataイメージを再利用します。ホストQEMUの
要件とOVMFの候補パスは [`nagi.toml`](../../nagi.toml) にあります。プロジェクトの
参照環境はx86-64、q35、UEFI、仮想CPU 4基、RAM 8 GiBです。リリース用の正確な
起動コマンドとfirmwareの梱包方法は、リリースmanifestとともに固定する必要が
あります。ホストのQEMU/OVMF要件は、ゲストイメージにこれらを同梱する意味では
ありません。

リリースイメージ、配布先、checksum、ソースrevision、build manifest、最終起動手順は、
成果物が固定された後に記載します。この草案ではファイル名を決めません。現在の
開発手順は[開発者ガイド](../developer-preview/README.md)を参照してください。

## 既知の制限

- 0.1には一定時間後の自動ロック（idle lock）がありません。
- 0.1の対象はQEMU x86-64参照環境です。物理PCや他の仮想環境への対応は約束しません。
- アプリ名は選択された範囲を表し、計画された全ワークフローや連携の完成を保証
  しません。各動作を利用する前に、リリース固有の受入記録を確認してください。
- Albert/Servoブラウザー、ローカルAI、音声、意味検索、A/B起動と復旧は0.1の範囲外です。
- Nagiは実験段階のソフトウェアであり、本番環境向けのセキュリティ保証を提供しません。
  [セキュリティポリシー](../../SECURITY.md)を参照してください。
- 最終qcow2と、イメージ固有の依存関係・カーネル・firmwareの通知情報は、この草案には
  ありません。[THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md)と
  [LICENSE_SCOPE.md](../../LICENSE_SCOPE.md)を参照してください。ソースの公開やこの草案は、
  コンパイル済みイメージの再配布許可を示しません。

## 不具合の報告

再現可能な製品または開発ツールの不具合は、
[GitHub issueフォーム](https://github.com/Roy-To/NagiOS/issues/new/choose)から報告して
ください。ソースrevision、ホスト/QEMU環境、最小の再現手順、個人情報を除いたログを
添えてください。資格情報、token、個人情報、ユーザー名を含むローカルパス、チャットIDは
投稿前に削除してください。

セキュリティ脆弱性の疑いは、**公開issueに投稿しないでください**。
リポジトリの **Security** タブから **Advisories → Report a vulnerability** を選び、
[SECURITY.md](../../SECURITY.md)の手順で非公開に報告してください。

## ライセンス

Nagiソースのライセンスは[LICENSE_SCOPE.md](../../LICENSE_SCOPE.md)に定めた範囲に
限られます。第三者のコンポーネントやアセットには各々の条件が適用されます。
現行の通知ファイルはソース準備の記録であり、完全な部品表やコンパイル済みイメージの
再配布許可を示すものではありません。
