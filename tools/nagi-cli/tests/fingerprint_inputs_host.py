"""Host-only input coverage probes; run with a compiled fingerprint module harness.

No build, fetch, VM or real cache mutation. Executables/environment overrides
exist only in each temporary fixture's child process. --baseline asserts the
old equal-digest bug; the default asserts the candidate detects each change.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def run_git(args, check=True):
    # Git's inherited repository/config selectors override -C. Keep all fixture
    # writes inside the intended temporary root even on a contaminated host.
    env = {k: v for k, v in os.environ.items() if not k.upper().startswith("GIT_")}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null")
    subprocess.run(["/usr/bin/git", "-c", "core.hooksPath=/dev/null", *args],
                   env=env, check=check)


def run(binary, baseline, additional_only=False, cargo_for_selectors=None, config_only=False):
    cases = ["NAGI_FONT_DIR", "NAGI_LIBCXX_ARCHIVE", "NAGI_LLAMA_BUILD",
             "NAGI_LLAMA_SOURCE", "NAGI_TARGET_LD", "RUSTUP_TOOLCHAIN",
             "CARGO_HOME", "registry_bytes", "llvm_bytes", "patched_llama_bytes", "compiler_bytes"]
    additional = ["NAGI_WHISPER_BUILD", "NAGI_WHISPER_SOURCE",
                  "NAGI_M25_WHISPER_PCM_FIXTURE", "NAGI_M25_WHISPER_EXPECTED_TEXT_FILE",
                  "NAGI_ACCEPTANCE_PACKAGES", "NAGI_FILES_SEARCH_PACKAGE"]
    cases = additional if additional_only else cases + additional
    if cargo_for_selectors:
        cases = ["compiler_config"] if config_only else cases + ["compiler_config"]
    rows = []
    for case in cases:
        with tempfile.TemporaryDirectory(prefix="fingerprint-inputs-") as temporary:
            temp = Path(temporary)
            root = temp / "repo"
            root.mkdir()
            tools = temp / "bin"
            tools.mkdir()
            (tools / "git").symlink_to("/usr/bin/git")
            for name, text in [("rustc", "rustc 1.90.0-nightly\nrelease: 1.90.0-nightly\ncommit-hash: fixture\nhost: x86_64-unknown-linux-gnu\nllvm-version: 20"),
                               ("cargo", "cargo 1.90.0-nightly")]:
                executable = tools / name
                executable.write_text("#!/bin/sh\nprintf '%s\\n' '" + text + "'\n")
                executable.chmod(0o700)
            run_git(["init", "-q", str(root)], check=True)
            (root / ".gitignore").write_text("/out/\n/third_party/generated/\n")
            (root / "third_party").mkdir()
            (root / "third_party/sources.lock").write_text(
                '[sources.registry]\nsource_hash = "sha256:fixture"\nvendored_path = "third_party/generated"\n'
                '[sources.llvm_libcxx]\nversion = "fixture"\nvendored_path = "out/cache/llvm-libcxx-fixture"\n')
            run_git(["-C", str(root), "add", "."], check=True)
            run_git(["-C", str(root), "-c", "user.name=Fixture",
                            "-c", "user.email=fixture@example.test", "-c", "commit.gpgsign=false",
                            "commit", "-qm", "fixture"], check=True)
            # Retain ordinary host environment; isolate build selectors in child.
            env = {k: v for k, v in os.environ.items()
                   if not k.startswith(("NAGI_", "CARGO_", "RUST", "OVMF", "GIT_"))}
            env.update(PATH=str(tools) + ":/usr/bin:/bin", RUSTC=str(tools / "rustc"), CARGO=str(tools / "cargo"))
            if case in ["RUSTUP_TOOLCHAIN", "CARGO_HOME"]:
                env[case] = "fixture-a"
                changed = None
            elif case == "compiler_config":
                env.pop("RUSTC", None)
                env["CARGO"] = str(cargo_for_selectors)
                other = tools / "rustc-other"
                other.write_text((tools / "rustc").read_text() + "\n# distinct compiler bytes\n")
                other.chmod(0o700)
                # Ancestor configuration is outside root's tracked config list.
                changed = temp / ".cargo/config.toml"
                changed.parent.mkdir()
                changed.write_text('[build]\nrustc="bin/rustc"\n')
            elif case == "compiler_bytes":
                changed = tools / "rustc"
            else:
                relative = {"registry_bytes": "third_party/generated/src/lib.rs",
                            "llvm_bytes": "out/cache/llvm-libcxx-fixture/libcxx/src/fixture.cpp",
                            "patched_llama_bytes": "out/cache/llama-cpp-nagi/src/fixture.cpp"}.get(case)
                changed = root / relative if relative else temp / "selected" / "bytes"
                changed.parent.mkdir(parents=True, exist_ok=True)
                changed.write_bytes(b"fixture-A")
                if not relative:
                    env[case] = str(changed if case in ["NAGI_LIBCXX_ARCHIVE", "NAGI_TARGET_LD", "NAGI_M25_WHISPER_PCM_FIXTURE", "NAGI_M25_WHISPER_EXPECTED_TEXT_FILE", "NAGI_FILES_SEARCH_PACKAGE"] else changed.parent)
                # A constant marker is deliberately insufficient evidence.
                (changed.parent / ".nagi-source-marker").write_text("constant marker")

            def fingerprint():
                p = subprocess.run([str(binary), str(root)], env=env, capture_output=True,
                                   text=True, timeout=10)
                assert p.returncode == 0, (case, p.returncode, p.stderr, p.stdout)
                return json.loads(p.stdout)

            first = fingerprint()
            repeated = fingerprint()
            assert first["compatibility"] == repeated["compatibility"], case
            if changed is None:
                env[case] = "fixture-b"
            elif case == "compiler_config":
                changed.write_text('[build]\nrustc="bin/rustc-other"\n')
            elif case == "compiler_bytes":
                changed.write_text(changed.read_text() + "\n# changed compiler bytes, same version output\n")
            else:
                changed.write_bytes(b"fixture-B")
            second = fingerprint()
            equal = first["compatibility"]["digest"] == second["compatibility"]["digest"]
            assert equal == baseline, (case, "digest equality", equal)
            assert first["schema_version"] == second["schema_version"] == 1
            # Neither the constant marker nor selected contents are serialized.
            assert "constant marker" not in json.dumps(second)
            assert "fixture-B" not in json.dumps(second)
            rows.append({"case": case, "equal_digest": equal, "rc": 0})
    if not baseline and not additional_only and not config_only:
        for case in ["secret_symlink_target", "secret_symlink_error", "role_swap", "fifo", "missing"]:
            with tempfile.TemporaryDirectory(prefix="fingerprint-boundary-") as temporary:
                root = Path(temporary)
                # Reuse the module entry point for help-free generation through
                # a minimal valid Git root and fixture toolchain.
                run_git(["init", "-q", str(root)], check=True)
                (root / "third_party").mkdir()
                (root / "third_party/sources.lock").write_text("")
                (root / ".gitignore").write_text("/selected/\n")
                run_git(["-C", str(root), "add", "."], check=True)
                run_git(["-C", str(root), "-c", "user.name=Fixture",
                                "-c", "user.email=fixture@example.test", "-c", "commit.gpgsign=false",
                                "commit", "-qm", "fixture"], check=True)
                selected = root / "selected"
                selected.mkdir()
                tools = selected / "bin"
                tools.mkdir()
                for name, text in [("rustc", "rustc 1.90.0-nightly\nhost: x86_64-unknown-linux-gnu"),
                                   ("cargo", "cargo 1.90.0-nightly")]:
                    tool = tools / name
                    tool.write_text("#!/bin/sh\nprintf '%s\\n' '" + text + "'\n")
                    tool.chmod(0o700)
                env = {k: v for k, v in os.environ.items()
                       if not k.startswith(("NAGI_", "CARGO_", "RUST", "OVMF", "GIT_"))}
                env.update(PATH=str(tools) + ":/usr/bin:/bin", RUSTC=str(tools / "rustc"), CARGO=str(tools / "cargo"))
                source = selected / "source"
                source.mkdir()
                (source / "bytes").write_bytes(b"source A")
                env["NAGI_FONT_DIR"] = str(source)
                if case in ["secret_symlink_target", "secret_symlink_error"]:
                    target = selected / "token=fixture-secret-do-not-record"
                    source.rename(target)
                    source.symlink_to(target, target_is_directory=True)
                    if case == "secret_symlink_error":
                        (target / "directory-link").symlink_to(tools, target_is_directory=True)
                elif case == "fifo":
                    os.mkfifo(source / "pipe")
                elif case == "missing":
                    env["NAGI_FONT_DIR"] = str(selected / "absent")
                elif case == "role_swap":
                    other = selected / "other"
                    other.mkdir()
                    (other / "bytes").write_bytes(b"source B")
                    env["NAGI_LLAMA_SOURCE"] = str(other)
                p = subprocess.run([str(binary), str(root)], env=env, capture_output=True, text=True, timeout=10)
                assert "fixture-secret-do-not-record" not in p.stdout + p.stderr
                if case in ["fifo", "missing", "secret_symlink_error"]:
                    assert p.returncode == 4, (case, p.returncode, p.stdout)
                else:
                    assert p.returncode == 0, (case, p.returncode, p.stdout)
                    assert "fixture-secret-do-not-record" not in p.stdout
                    if case == "role_swap":
                        env["NAGI_FONT_DIR"], env["NAGI_LLAMA_SOURCE"] = env["NAGI_LLAMA_SOURCE"], env["NAGI_FONT_DIR"]
                        q = subprocess.run([str(binary), str(root)], env=env, capture_output=True, text=True, timeout=10)
                        assert q.returncode == 0, q.stdout
                        assert json.loads(p.stdout)["compatibility"]["digest"] != json.loads(q.stdout)["compatibility"]["digest"]
                rows.append({"case": case, "rc": p.returncode})
    print(json.dumps({"baseline": baseline, "cases": rows}, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--baseline", action="store_true")
    parser.add_argument("--additional-selectors-only", action="store_true")
    parser.add_argument("--cargo-for-selectors", type=Path)
    parser.add_argument("--config-selector-only", action="store_true")
    args = parser.parse_args()
    run(args.binary.resolve(), args.baseline, args.additional_selectors_only, args.cargo_for_selectors, args.config_selector_only)
