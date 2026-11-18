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
  「実directory・所有者が自分・modeがちょうど `0o700 & ~umask`（通常は0o700）・空・その名前の現在の実体と同一」の
  全てを満たす場合だけ受け入れる。満たさなければ（mkdir→open間の差し替えなど）
  `output_raced` で中止し、その中へは何も書かず、作成物としても扱わない
  （`left_behind` に名前だけ報告）。
- 全fileの書込み後、出力先が要求したpathに存在し、各名前の実体が保持中のhandleと同一で、
  各directoryの中身が作成物と過不足なく一致することを照合する。違えば `output_raced` で
  失敗にし、修復・削除はしない。照合に通った後でだけ、保持中のhandleで
  directoryのmodeを `(0o755 & ~umask) | 0o700` に広げる。
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
- 同時に動く別プロセスによる敵対的なファイルシステム競合は**脅威モデル外**とする
  （下の「脅威モデルと限界」）。差替え: 将来、排他的な作業領域での作成＋非置換の公開方式
  （`generate.generate`）。
- 生成後のManifest検証・hostでのbuild/testは、PT-05等の別Stageで明示的に行う前提。
  このgeneratorはそれらを呼ばない。

## 脅威モデルと限界

このhost pilotが想定する脅威は、**generator実行前から存在する**状態（既存の
file・directory・symlink、範囲外path、不正な入力）と、実行中の通常の書込み失敗・中断。
実行中に**同時に動く別プロセスが出力先やその祖先を敵対的に操作する競合（TOCTOU）は
脅威モデル外**で、防御を保証しない（owner判断、本pilotでは対策しない）。

限界の例（保証しない）:

- preflightで保持した親directoryのfdは、別プロセスがその親を移動・改名しても追従しない。
  許可root外へ移動された場合、生成は移動先で続行され得る（末尾の `verify_published()` が
  検出すれば `output_raced` で失敗にするが、書込み済みの物は残置され、範囲外への書込みを
  防ぐものではない）。
- 同一uidのprocessがmkdir直後に作成directoryを別名へ移し、**同じmode 0o700の空directory**を
  置いた場合、POSIXのmkdirがhandleを返さないため自分の作成物と区別できず、その空directoryへ
  書込み、modeを0o755に広げ得る（`test_residual_mode_matching_swap_...` は現在の挙動を
  記録するだけで、保証ではない）。
- 上記でも削除・rename・既存内容の上書きは行わない。mkdir→open間の差替えのうち
  mode／所有者／中身／symlinkで判別できるものと、書込み後の照合で判別できるものは
  best-effortで `output_raced` にするが、全ての競合の検出は主張しない。

### directory modeの厳密判定とumask（NDP-A002、PROVISIONAL）

`mkdir(0o700)` の結果は `0o700 & ~umask` になるため、受け入れ判定を固定の `0o700` にすると
制限的なumask（例: `0o277` → 実際は `0o500`）で正常な作成が `output_raced` になっていた。
現在の扱い（いずれも仮定で、CONFIRMEDではない）:

- umaskは `generate()` の冒頭で1回だけ読み、mkdir後の期待modeを `0o700 & ~umask` として
  判定する（mkdirと判定が同じ値から導かれ、食い違わない）。期待modeと一致しない実体
  （group/world書込み可への差し替えなど）・所有者違い・空でない・symlinkは従来どおり
  `output_raced` で拒否する。**受け入れる前にmodeを変更しない**ので、差し替えられた
  directoryを `fchmod` で「正常に見せかける」ことはない。
- 受け入れた後、保持中のhandleで `fchmod(0o700)` して初めて中へ書き込む。
  書込み中のdirectoryは常にちょうど `0o700`。
- 公開後のdirectory modeは `(0o755 & ~umask) | 0o700`（umaskを尊重しつつ、所有者から
  自分の出力のrwxを奪わない）。通常のumask（`0o022` / `0o077`）では従来と同じ。
- umaskに所有者の読取りbit（`0o400`）が含まれる場合（例: `0o777`）は、mkdirが所有者自身も
  `open` できないdirectoryを作りhandleに束縛できないため、何も作らず
  `unsupported_umask` で拒否する（process全体のumaskを一時変更する方式は採らない）。
- 限界: 期待modeと一致する同一uidの差し替え（上記）は、umaskを変えても判別できない。
  `0o077` や `0o277` では期待modeが `0o700` / `0o500` で、差し替えをmodeで区別する余地は
  通常のumaskと同じかそれ以下。umask取得は「設定して即復元」で、他threadが同時にumaskを
  使う場合は保証しない。差替え箇所: `generate._claim_directory` / `generate.generate`。

多人数・常駐・CIなど他processが同じ作業領域を操作し得る環境で使う場合は、専用の作業領域を
排他的に確保する等の運用か、ownerによる要件変更（排他管理方式の採用）が別途必要。

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
