# Third-Party Rust App Quick Start

This is a host-only Developer Preview flow for building the standalone `Hello Nagi` Rust sample outside the NagiOS source tree and checking its app manifest with the existing host package tool.

The copied sample uses an out-of-tree Cargo manifest pinned to public NagiOS revision `87b7dea4061be2572526181ab559d63037f823d0`. The SDK is not published to crates.io (`publish = false`). The app directly depends only on `nagi-sdk`; Cargo also fetches the SDK's transitive public dependencies. The checked-in `Cargo.toml` remains path-based for the existing offline host artifact flow.

## English

### 1. Get the sample and copy it out of the source tree

Until this workstream is integrated, use its public review branch to obtain the sample. Copy the directory so the app is no longer inside a NagiOS checkout:

```sh
git clone --depth 1 --branch codex/0.2-third-party-app-dx https://github.com/Roy-To/NagiOS.git nagios-sdk-source
cp -R nagios-sdk-source/samples/hello-nagi ./hello-nagi
cd hello-nagi
cp Cargo.out-of-tree.toml Cargo.toml
cp Cargo.out-of-tree.lock Cargo.lock
```

After the sample is integrated, `main` can replace the review branch in the clone command.

In PowerShell, replace the copy and directory commands with:

```powershell
Copy-Item -Recurse .\nagios-sdk-source\samples\hello-nagi .\hello-nagi
Set-Location .\hello-nagi
Copy-Item .\Cargo.out-of-tree.toml .\Cargo.toml -Force
Copy-Item .\Cargo.out-of-tree.lock .\Cargo.lock -Force
```

### 2. Build and inspect the dependency source

```sh
cargo build --locked
cargo tree --locked --depth 1
```

The top-level dependency should be `nagi-sdk` from the pinned `https://github.com/Roy-To/NagiOS.git` revision. There should be no path dependency back to `nagios-sdk-source`.

### 3. Validate the app manifest

Build and run the existing `nagi-pkg` host tool from the public source checkout. From the copied sample directory, return to the source checkout and validate the copied manifest:

```sh
cd ../nagios-sdk-source
cargo run --manifest-path tools/nagi-pkg/Cargo.toml --locked -- manifest validate ../hello-nagi/manifest.json
```

Expected output starts with `PASS nagi-pkg manifest validate`. This checks manifest syntax and host-side semantic rules; it does not create an installable package.

## 日本語

### 1. サンプルを取得してソースツリー外へコピー

この workstream が統合されるまでは、公開レビュー用ブランチからサンプルを取得します。コピー後のアプリは NagiOS の checkout の外に置かれます。

```sh
git clone --depth 1 --branch codex/0.2-third-party-app-dx https://github.com/Roy-To/NagiOS.git nagios-sdk-source
cp -R nagios-sdk-source/samples/hello-nagi ./hello-nagi
cd hello-nagi
cp Cargo.out-of-tree.toml Cargo.toml
cp Cargo.out-of-tree.lock Cargo.lock
```

サンプル統合後は、clone コマンドのブランチを `main` に置き換えられます。

PowerShell では、コピーとディレクトリ移動に次を使えます。

```powershell
Copy-Item -Recurse .\nagios-sdk-source\samples\hello-nagi .\hello-nagi
Set-Location .\hello-nagi
Copy-Item .\Cargo.out-of-tree.toml .\Cargo.toml -Force
Copy-Item .\Cargo.out-of-tree.lock .\Cargo.lock -Force
```

### 2. ビルドして依存元を確認

```sh
cargo build --locked
cargo tree --locked --depth 1
```

直接依存の `nagi-sdk` が、公開 Git リポジトリ `https://github.com/Roy-To/NagiOS.git` の固定 revision を参照していることを確認します。`nagios-sdk-source` への path dependency はありません。

### 3. アプリ manifest を検証

公開ソース checkout から既存の host 用 `nagi-pkg` を実行します。コピーした sample のディレクトリから source checkout に戻り、manifest を検証します。

```sh
cd ../nagios-sdk-source
cargo run --manifest-path tools/nagi-pkg/Cargo.toml --locked -- manifest validate ../hello-nagi/manifest.json
```

出力の先頭が `PASS nagi-pkg manifest validate` なら成功です。これは manifest の構文と host 側の意味検証です。インストール可能な package の生成・導入を示すものではありません。

## Scope and gate status / 対象範囲と gate

- This flow demonstrates an out-of-tree host build and manifest validation only. / この手順が確認するのは out-of-tree の host build と manifest 検証だけです。
- It does not install or launch a guest app, connect the SDK to the target runtime, or test authority enforcement. / guest app の導入・起動、target runtime との接続、authority enforcement は行いません。
- The existing `app-sdk-contract` `PASS` is host-side only. Nagi 0.1 M30 is still `PARTIAL`, the 0.2 activation checkpoint is closed, and full M21 acceptance remains incomplete. / 既存の `app-sdk-contract` の `PASS` は host 側のみです。Nagi 0.1 M30 は `PARTIAL`、0.2 activation checkpoint は未開放で、M21 全体の acceptance も未完了です。
- `nagi-pkg manifest validate` validates metadata; third-party package installation and runtime acceptance are not provided by this checkpoint. / `nagi-pkg manifest validate` は metadata を検証します。この checkpoint では third-party package の導入や runtime acceptance は提供しません。
