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
- fileは `O_CREAT|O_EXCL|O_NOFOLLOW` で作成し、作成したhandleから `fstat` で
  (dev, inode, type) を記録する。書込みはそのhandle経由で行う。
- **何も削除しない。** `unlink` / `rmdir` / `rename` / `replace` は一切呼ばない
  （テストのsetUpで全テストに対し強制）。失敗・中断時は、この実行が作成したものを
  その場に残し、`created` / `left_behind` に報告する（`removed` は常に空）。
  POSIXにはhandle指定のunlinkやhandleを返すmkdirが無く、並行する差し替えがあると
  削除対象が自分の作成物だと証明できないため、自動cleanupをやめて安全側に倒した。
  失敗後の出力先は手動で確認・削除する（再実行は `output_exists` で拒否される）。
- 作成したfile/directoryのhandleは、最後の照合と仕上げが終わるまで閉じない。
  開いている間はinodeが再利用されないので、記録したidentityとの一致は同一物の証明になる。
- directoryは `mkdir(dir_fd, 0o700)` で非公開に作り、no-followでopenした実体が
  「実directory・所有者が自分・modeがちょうど0o700・空・その名前の現在の実体と同一」の
  全てを満たす場合だけ受け入れる。満たさなければ（mkdir→open間の差し替えなど）
  `output_raced` で中止し、その中へは何も書かず、作成物としても扱わない
  （`left_behind` に名前だけ報告）。
- 全fileの書込み後、出力先が要求したpathに存在し、各名前の実体が保持中のhandleと同一で、
  各directoryの中身が作成物と過不足なく一致することを照合する。違えば `output_raced` で
  失敗にし、修復・削除はしない。照合に通った後でだけ、保持中のhandleで
  directoryのmodeを `0o755 & ~umask` に広げる。
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
- 失敗時は自動削除しない（`level0.json.writePolicy.onFailure` =
  `never-delete-leave-created-objects-and-report`）。部分出力が残る代わりに、
  第三者の実体を消す経路を無くした。将来、作成物を証明付きで消せる仕組み
  （例: handle指定の削除を持つNagi側API）ができたら差し替える（差替え: `generate.generate`
  の失敗処理と `writePolicy.onFailure`）。
- 検出できない残存競合: 同一uidのprocessが、こちらのmkdir直後に作成directoryを別名へ移し、
  **同じmode 0o700の空directory**を自分で作って置いた場合は、POSIXのmkdirがhandleを
  返さないため自分の作成物と区別できない（`test_residual_mode_matching_swap_...` で
  挙動を固定）。この場合も削除・上書き・既存内容の変更は起きず、起き得るのは
  「要求したpathに置かれた空directoryへ雛形の新規fileを書き、そのmodeを0o755に広げる」
  ことだけ。同一uidのprocessは元々そのdirectoryを自由に操作できる権限を持つため、
  これは安全境界の越境ではないと仮定する。より強い保証が必要なら、ownerの判断が要る。
- 生成後のManifest検証・hostでのbuild/testは、PT-05等の別Stageで明示的に行う前提。
  このgeneratorはそれらを呼ばない。

## 検証

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/nagi-developer-platform/generator -p 'test_*.py'
```

テストは実際の `generate.py` を呼び、一時directoryにだけ書く。生成・決定性・入力不変・
CLI、無効ID／未対応SDK版／出力先衝突／path逸脱／symlink（出力先・祖先・SDK）／
書込み失敗・競合・中断時に何も削除せず報告すること、作成済みfile/directoryの差し替えを
成功前に検出すること、作成handleが最後の照合まで閉じられないこと、mkdir→open間の
directory差し替え（空・既定mode／中身あり／group書込み可／symlink、出力先とsrcの両方）を
拒否してその中へ書かないことを確認する。

`test_generated_manifest_passes_real_validator_when_available` は
`validation/validate.py` の実validator（`out/ndp-host/target/debug/nagi-pkg`）が
準備済みの場合だけ実行し、未準備なら**skip**する（skip時はManifestが実validatorで
未検査であることをメッセージに出す）。validatorの準備は
[validation/README.md](../validation/README.md) を参照。常時実行のschema照合は
必須root fieldの存在と未知fieldの有無のみを見る構造検査で、semantic validatorの代替ではない。

## 未実施

実validatorによるManifest検証（本worktreeではvalidator未準備のためskip）、生成物の
Rust build/test、SDK正式配布方式との接続、Nagi target上の動作は未実施・未証明。
