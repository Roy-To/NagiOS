# NDP-03 — Level 0雛形generator pilot

NDP-02の契約（[contract/](contract/README.md)）に従い、明示した新規出力先へ
Level 0雛形（`Cargo.toml` / `manifest.json` / `src/lib.rs` / `README.md` /
`UNSUPPORTED_APIS.md`）を生成するhost専用のPython 3.9標準library実装。
内容は `contract.py::preview` が作り、`generate.py` は書込み可否と書込みだけを担当する。

生成したcrateのbuild・実行・依存取得・署名・install・package化は行わない。
生成直後のManifestを実validatorで検査することもしない（別の明示手順）。
host上の結果はNagi target受入ではない。

## 使い方

```sh
# 生成予定の表示（何も書かない。出力先の検査は生成時と同じ）
python3 tools/nagi-developer-platform/generator/generate.py plan --root <checkout> \
  --app-id com.example.level-zero --package-name level-zero \
  --name-en "Level Zero" --name-ja "レベルゼロ" --output-dir samples/level-zero
# 生成（新規directoryのみ）
python3 tools/nagi-developer-platform/generator/generate.py create ...同じ引数
```

`--root` は出力を受け取るcheckout（`sdk/rust/Cargo.toml` が必要）。既定はこのrepo。
結果はJSON1行（`created` / `planned` / `failed`）。終了値は成功0、失敗1。
結果に絶対pathは含めない。Python APIは `generate.plan(...)` / `generate.generate(...)`。

## 書込み規則（`level0.json.writePolicy` の実装）

- 出力先は `output_dir`（repo相対）の**まだ存在しない**directory。既存なら空でも、
  fileでも、symlink（dangling含む）でも拒否。上書き・mergeはしない。
- 親directoryは既存の実directoryのみ。無ければ作らず拒否（`output_parent_missing`）。
- root以下の各階層をno-follow（`lstat` + `O_NOFOLLOW|O_DIRECTORY`）で辿り、symlinkは拒否。
- directoryは `mkdir(dir_fd)`、fileは `O_CREAT|O_EXCL|O_NOFOLLOW` で作成し、競合を検出する。
- 失敗・中断（KeyboardInterrupt含む）時は、この実行が作成したfile/directoryだけを
  逆順に削除。第三者が途中で置いたfileは消さず、`left_behind` に残りを報告する。
- 「作成したもの」は名前ではなく、作成したhandleから `fstat` で記録した
  (dev, inode, type) で識別する。cleanup時に一致しないもの（第三者による削除・差し替え）は
  削除せず `left_behind` に入れる。
- POSIXにはhandle指定のunlinkが無いため、fileは確認後すぐ消さず、出力先の隣に排他作成した
  空の隔離directory（`.<name>.generator-cleanup-<random>`）へ `rename` で移し、移した実体の
  identityが記録と一致した場合だけunlinkする。確認と移動の間に差し替えられた第三者の実体は
  `link`（既存名を上書きしない）で元の名前へ戻して `left_behind` とし、戻せなければ
  隔離directoryに残してそのpathを報告する。隔離directoryは空なら最後に削除する。
- directoryは `rmdir`（空のときしか消えない）で削除し、事前にidentityも照合する。
- 作成直後のdirectoryが空でない場合（作成とopenの間の差し替え）は `output_raced` で中止し、
  そのdirectoryは作成物として扱わない。
- 保護対象: `.git`、`sdk`、`tools/nagi-pkg` 配下への出力は拒否（`PROTECTED_TREES`）。
- 入力・contractは変更しない。出力は決定的（同じ入力・同じ`output_dir`なら同一bytes）。

## 仮定（PROVISIONAL）

いずれもCONFIRMEDではなく、owner合意は未取得。差し替え箇所を併記する。

- 出力先の親directoryを自動作成しない。「新規directory 1つのみ作る」を最小・安全側の解釈とした
  （差替え: `generate._preflight`）。
- 出力先はSDKを含むcheckout（`--root`）内のrepo相対path。SDK参照は契約の
  `sdkReference`（repo内path依存）に従い、checkout内に `sdk/rust/Cargo.toml` が
  実在しなければ拒否する。SDKの外部配送方式は未確定で、変更は
  `contract.py::sdk_dependency` と `level0.json.sdkReference` で行う。
- `PROTECTED_TREES` の3項目は暫定のデータ。共有workspace・Package側の正式な保護範囲は
  各ownerの決定待ち（差替え: `generate.PROTECTED_TREES`）。
- Unix系hostのdirectory fd / `O_NOFOLLOW` を前提とする。使えないhostは `unsupported_host`
  で拒否する（native Windowsは対象外、`validation/validate.py` と同じ制限）。
- `--root` 自体が（例: macOSの `/tmp`）symlinkを含むpathの場合、`realpath` で解決してから
  使う。root以下はno-followで検査する。
- cleanupの残存競合（意図的な同一uidの妨害は対象外とする前提）: 隔離directory内の名前を
  第三者が狙って差し替える場合、および空directoryが `identity確認→rmdir` の間に第三者の
  空directoryへ差し替わる場合は防げない（失われ得るのは空directoryのみ）。
  file削除を一切行わない保守方針へ切り替える場合は `generate._cleanup` を差し替える。
- 生成後のManifest検証・hostでのbuild/testは、PT-05等の別Stageで明示的に行う前提。
  このgeneratorはそれらを呼ばない。

## 検証

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/nagi-developer-platform/generator -p 'test_*.py'
```

テストは実際の `generate.py` を呼び、一時directoryにだけ書く。生成・決定性・入力不変・
CLI、無効ID／未対応SDK版／出力先衝突／path逸脱／symlink（出力先・祖先・SDK）／
書込み失敗・競合・中断時の掃除、作成済みfile/directoryが第三者に差し替えられた後の失敗
（identity確認前・確認直後の両方）で第三者の実体が残ることを確認する。

`test_generated_manifest_passes_real_validator_when_available` は
`validation/validate.py` の実validator（`out/ndp-host/target/debug/nagi-pkg`）が
準備済みの場合だけ実行し、未準備なら**skip**する（skip時はManifestが実validatorで
未検査であることをメッセージに出す）。validatorの準備は
[validation/README.md](../validation/README.md) を参照。常時実行のschema照合は
必須root fieldの存在と未知fieldの有無のみを見る構造検査で、semantic validatorの代替ではない。

## 未実施

実validatorによるManifest検証（本worktreeではvalidator未準備のためskip）、生成物の
Rust build/test、SDK正式配布方式との接続、Nagi target上の動作は未実施・未証明。
