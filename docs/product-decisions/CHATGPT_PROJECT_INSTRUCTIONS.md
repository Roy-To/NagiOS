# Nagi_chat — ChatGPT Project Instructions (copy/paste)

**Purpose:** Copy the text under `## Paste into project instructions` into the Nagi_chat project's instruction setting. Saving this Markdown on GitHub does not update ChatGPT's project configuration.

**Updated:** 2026-11-15 ([DEC-050](DEC-050-public-specification-repository.md): the source of truth moved from the private repository to the public `Roy-To/NagiOS`).

## Paste into project instructions

Nagi OSの仕様検討・実装相談では、チャット履歴やChatGPT Memoryではなく、公開GitHubリポジトリ `Roy-To/NagiOS` の次の文書を「ユーザーが決めた製品仕様」の正本とする。
`Roy-To/NagiOS/docs/product-decisions/README.md`
`Roy-To/NagiOS/docs/product-decisions/OPERATING_RULES.md`

会話開始時や仕様変更前に、該当する最新の DEC-*.md と roadmaps/*.md を実際に読み、内容を確認する。読めなかった場合は読めたと偽らず明記し、Memoryからの推測で未確認の仕様を補わない。

ユーザーが仕様や構想を明確に決定したら、CONFIRMED と PROVISIONAL を分けて該当するDECを更新または新設し、README索引と変更履歴を更新して、`Roy-To/NagiOS` にコミットする。コミットを確認できた場合だけ「保存済み」と報告する。書き込めない場合は「未保存」として、貼り付け可能な差分を提示する。決まっていない点は勝手に確定しない。

記録には、パスワード・APIキー・トークンなどの認証情報、個人情報、ユーザー名を含むローカルパス、チャットの識別子を書かない。それ以外の製品仕様・ロードマップ・設計は公開してよい。

非公開リポジトリ `Roy-To/NagiOS-internal` は旧保存先であり、新しい決定を記録しない。

**基本ルール：チャットは相談場所、公開GitHubリポジトリは仕様の保管場所。**

## Installation / verification

1. ChatGPTのNagi_chatプロジェクトの「プロジェクトの指示」を上記本文に置き換える(このファイルを置いただけでは反映されない)。
2. 新しいNagi_chat会話で「製品決定索引を公開GitHubから読んで、DEC-050の内容を教えて」と依頼し、実際に参照できることを確認する。
