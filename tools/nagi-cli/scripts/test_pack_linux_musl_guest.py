#!/usr/bin/env python3
"""Focused fail-closed tests for pack-linux-musl-guest.py."""

from __future__ import annotations

import importlib.util
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).with_name("pack-linux-musl-guest.py")
INSTALLER_SCRIPT = Path(__file__).with_name("install-alpine-test-deps.sh")
SPEC = importlib.util.spec_from_file_location("pack_linux_musl_guest", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot import packer at {SCRIPT}")
PACKER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = PACKER
SPEC.loader.exec_module(PACKER)


def write_elf(
    path: Path,
    *,
    machine: int = 62,
    interpreter: bytes | None = None,
    needed_library: bool = False,
    build_id: bytes | None = None,
    unnoted_build_id: bytes | None = None,
    build_id_note_name: bytes = b"GNU\0",
    build_id_note_type: int = 3,
    duplicate_build_id_note: bool = False,
) -> None:
    segments: list[tuple[int, bytes]] = []
    if interpreter is not None:
        segments.append((3, interpreter + b"\0"))
    if needed_library:
        segments.append((2, struct.pack("<qQqQ", 1, 1, 0, 0)))

    if build_id is not None:
        if len(build_id) != 20:
            raise ValueError("synthetic build IDs must be 20 bytes")
        name_padding = b"\0" * ((-len(build_id_note_name)) % 4)
        desc_padding = b"\0" * ((-len(build_id)) % 4)
        note = (
            struct.pack("<III", len(build_id_note_name), len(build_id), build_id_note_type)
            + build_id_note_name
            + name_padding
            + build_id
            + desc_padding
        )
        segments.append((4, note))
        if duplicate_build_id_note:
            segments.append((4, note))

    phnum = 1 + len(segments)
    cursor = 64 + 56 * phnum
    payload = bytearray()
    program_headers = [None]
    for p_type, content in segments:
        program_headers.append(
            struct.pack(
                "<IIQQQQQQ",
                p_type,
                4,
                cursor + len(payload),
                0,
                0,
                len(content),
                len(content),
                1,
            )
        )
        payload.extend(content)

    if unnoted_build_id is not None:
        if len(unnoted_build_id) != 20:
            raise ValueError("synthetic unnoted build IDs must be 20 bytes")
        payload.extend(unnoted_build_id)

    data_size = 64 + 56 * phnum + len(payload)
    load_header = struct.pack(
        "<IIQQQQQQ", 1, 5, 0, 0x400000, 0x400000, data_size, data_size, 0x1000
    )
    program_headers[0] = load_header

    header = bytearray(64)
    header[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into(
        "<HHIQQQIHHHHHH",
        header,
        16,
        2,
        machine,
        1,
        0x400000,
        64,
        0,
        0,
        64,
        56,
        phnum,
        0,
        0,
        0,
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(bytes(header) + b"".join(program_headers) + payload)
    path.chmod(0o755)


def write_dep_info(path: Path, source: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f"{path.name}: {source}\n", encoding="utf-8")


def make_cargo_layout(
    cargo_target_dir: Path,
    *,
    omit: str | None = None,
    wrong_machine: str | None = None,
    interpreter: str | None = None,
    needed_library: str | None = None,
    duplicate: str | None = None,
    build_id: bytes | None = None,
) -> None:
    target = cargo_target_dir / PACKER.TARGET
    write_elf(target / "debug/nagi", build_id=build_id)
    write_elf(target / "release/nagi", build_id=build_id)
    write_dep_info(
        target / "debug/nagi.d",
        str(PACKER.REPO_ROOT / "tools/nagi-cli/src/main.rs"),
    )
    deps = target / "debug/deps"
    fingerprint_root = target / "debug/.fingerprint"
    for label, prefix in PACKER.HARNESS_PREFIXES.items():
        if label == omit:
            continue
        machine = 183 if label == wrong_machine else 62
        interp = b"/lib64/ld-linux-x86-64.so.2" if label == interpreter else None
        has_needed = label == needed_library
        artifact_hash = "testhash"
        write_elf(
            deps / f"{prefix}{artifact_hash}",
            machine=machine,
            interpreter=interp,
            needed_library=has_needed,
            build_id=build_id,
        )
        fingerprint = fingerprint_root / f"nagi-cli-{artifact_hash}"
        fingerprint.mkdir(parents=True, exist_ok=True)
        (fingerprint / PACKER.HARNESS_FINGERPRINTS[label]).touch()
        write_dep_info(deps / f"{prefix}{artifact_hash}.d", PACKER.HARNESS_SOURCES[label])
        if label == duplicate:
            write_elf(deps / f"{prefix}otherhash", build_id=build_id)
            duplicate_fingerprint = fingerprint_root / "nagi-cli-otherhash"
            duplicate_fingerprint.mkdir(parents=True, exist_ok=True)
            (duplicate_fingerprint / PACKER.HARNESS_FINGERPRINTS[label]).touch()

    # Cargo may leave the ordinary binary beside its test harness in deps.
    # It shares the `nagi-` prefix but carries a non-test fingerprint marker.
    write_elf(deps / "nagi-ordinaryhash", build_id=build_id)
    normal_fingerprint = fingerprint_root / "nagi-cli-ordinaryhash"
    normal_fingerprint.mkdir(parents=True, exist_ok=True)
    (normal_fingerprint / "bin-nagi").touch()
    write_dep_info(deps / "nagi-ordinaryhash.d", "tools/nagi-cli/src/main.rs")


class PackerRegressionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.repo_root = PACKER.REPO_ROOT
        self.scratch = tempfile.TemporaryDirectory(
            prefix="nagi-cli-pack-test-", dir=self.repo_root / "target"
        )
        self.root = Path(self.scratch.name)

    def tearDown(self) -> None:
        self.scratch.cleanup()

    def test_accepts_static_x86_64_elf(self) -> None:
        executable = self.root / "valid"
        write_elf(executable)
        PACKER.validate_static_x86_64_elf(executable)

    def test_rejects_wrong_machine(self) -> None:
        executable = self.root / "aarch64"
        write_elf(executable, machine=183)
        with self.assertRaisesRegex(PACKER.PackError, "x86-64"):
            PACKER.validate_static_x86_64_elf(executable)

    def test_rejects_external_interpreter_and_shared_library(self) -> None:
        interpreter = self.root / "interpreter"
        write_elf(interpreter, interpreter=b"/lib64/ld-linux-x86-64.so.2")
        with self.assertRaisesRegex(PACKER.PackError, "interpreter"):
            PACKER.validate_static_x86_64_elf(interpreter)

        needed = self.root / "needed"
        write_elf(needed, needed_library=True)
        with self.assertRaisesRegex(PACKER.PackError, "shared library"):
            PACKER.validate_static_x86_64_elf(needed)

    def test_missing_harness_fails_before_creating_bundle(self) -> None:
        build_root = self.root / "build"
        cargo_target = build_root / "cargo-target"
        make_cargo_layout(cargo_target, omit="run_receipt_cli")
        bundle = self.root / "bundle"
        with self.assertRaisesRegex(PACKER.PackError, "exactly one executable"):
            PACKER.pack_bundle(self.repo_root, build_root, cargo_target, bundle)
        self.assertFalse(bundle.exists())

    def test_wrong_target_fails_before_creating_bundle(self) -> None:
        build_root = self.root / "wrong-target-build"
        cargo_target = build_root / "cargo-target"
        make_cargo_layout(cargo_target, wrong_machine="cli")
        bundle = self.root / "wrong-target-bundle"
        with self.assertRaisesRegex(PACKER.PackError, "x86-64"):
            PACKER.pack_bundle(self.repo_root, build_root, cargo_target, bundle)
        self.assertFalse(bundle.exists())

    def test_dynamic_link_fails_before_creating_bundle(self) -> None:
        build_root = self.root / "dynamic-build"
        cargo_target = build_root / "cargo-target"
        make_cargo_layout(cargo_target, interpreter="cli")
        bundle = self.root / "dynamic-bundle"
        with self.assertRaisesRegex(PACKER.PackError, "external interpreter"):
            PACKER.pack_bundle(self.repo_root, build_root, cargo_target, bundle)
        self.assertFalse(bundle.exists())

    def _manifest_fixture(self) -> dict[str, object]:
        build_script_hash = "a" * 64
        return {
            "schema_version": 2,
            "target": PACKER.TARGET,
            "cargo_target_dir": "target/fixture/cargo-target",
            "build_script_sha256": build_script_hash,
            "build_id": build_script_hash[:40],
            "build_support_sha256": {
                path: "b" * 64 for path in PACKER.BUILD_SUPPORT_PATHS
            },
            "source_sha256": {"tools/nagi-cli/src/main.rs": "c" * 64},
            "cargo_local_dependency_packages": {
                "nagi-model": "crates/nagi-model/Cargo.toml",
                "nagi-model-manager": "user/nagi-model-manager/Cargo.toml",
            },
            "cargo_local_dependency_sha256": {
                "crates/nagi-model/Cargo.toml": "e" * 64,
                "user/nagi-model-manager/Cargo.toml": "f" * 64,
            },
            "artifacts": {
                label: {
                    "path": f"target/fixture/cargo-target/{label}",
                    "sha256": "d" * 64,
                }
                for label in PACKER.BUILD_ARTIFACT_LABELS
            },
        }

    def test_build_manifest_requires_current_script_source_and_binary_inputs(self) -> None:
        path = self.root / "build-manifest.json"
        expected = self._manifest_fixture()
        path.write_text(json.dumps(expected), encoding="utf-8")
        PACKER._verify_build_manifest(path, expected)

        stale_script = dict(expected)
        stale_script["build_script_sha256"] = "e" * 64
        with self.assertRaisesRegex(PACKER.PackError, "different build script"):
            PACKER._verify_build_manifest(path, stale_script)

        stale_support = dict(expected)
        stale_support["build_support_sha256"] = dict(expected["build_support_sha256"])
        stale_support["build_support_sha256"][
            "tools/nagi-cli/scripts/verify-cargo-lock.py"
        ] = "9" * 64
        with self.assertRaisesRegex(PACKER.PackError, "build support inputs changed"):
            PACKER._verify_build_manifest(path, stale_support)

        stale_source = dict(expected)
        stale_source["source_sha256"] = {"tools/nagi-cli/src/main.rs": "f" * 64}
        with self.assertRaisesRegex(PACKER.PackError, "source inputs changed"):
            PACKER._verify_build_manifest(path, stale_source)

        stale_dependency = dict(expected)
        stale_dependency["cargo_local_dependency_sha256"] = {
            "crates/nagi-model/Cargo.toml": "0" * 64,
            "user/nagi-model-manager/Cargo.toml": "f" * 64,
        }
        with self.assertRaisesRegex(PACKER.PackError, "Cargo dependency closure changed"):
            PACKER._verify_build_manifest(path, stale_dependency)

        changed_binary = dict(expected)
        changed_binary["artifacts"] = {
            **expected["artifacts"],
            "release_cli": {"path": "target/fixture/release/nagi", "sha256": "0" * 64},
        }
        with self.assertRaisesRegex(PACKER.PackError, "artifacts changed"):
            PACKER._verify_build_manifest(path, changed_binary)

    def test_mainline_cli_local_dependency_graph_is_resolved_and_tracked(self) -> None:
        root = self.repo_root.resolve()
        metadata = {
            "packages": [
                {
                    "id": "path+nagi-cli",
                    "name": "nagi-cli",
                    "source": None,
                    "manifest_path": str(root / "tools/nagi-cli/Cargo.toml"),
                },
                {
                    "id": "path+nagi-model-manager",
                    "name": "nagi-model-manager",
                    "source": None,
                    "manifest_path": str(root / "user/nagi-model-manager/Cargo.toml"),
                },
                {
                    "id": "path+nagi-model",
                    "name": "nagi-model",
                    "source": None,
                    "manifest_path": str(root / "crates/nagi-model/Cargo.toml"),
                },
                {
                    "id": "registry+serde",
                    "name": "serde",
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "manifest_path": str(root / "vendor/serde/Cargo.toml"),
                },
                {
                    "id": "path+unused",
                    "name": "nagi-init",
                    "source": None,
                    "manifest_path": str(root / "user/nagi-init/Cargo.toml"),
                },
            ],
            "resolve": {
                "nodes": [
                    {
                        "id": "path+nagi-cli",
                        "deps": [
                            {"pkg": "path+nagi-model-manager"},
                            {"pkg": "registry+serde"},
                        ],
                    },
                    {
                        "id": "path+nagi-model-manager",
                        "deps": [{"pkg": "path+nagi-model"}],
                    },
                    {"id": "path+nagi-model", "deps": []},
                    {"id": "registry+serde", "deps": []},
                    {"id": "path+unused", "deps": []},
                ]
            },
        }
        local = PACKER._reachable_local_cargo_packages(root, metadata)
        self.assertEqual(
            local,
            {
                "nagi-model": "crates/nagi-model/Cargo.toml",
                "nagi-model-manager": "user/nagi-model-manager/Cargo.toml",
            },
        )
        sources = PACKER._tracked_cargo_dependency_sources(root, local)
        for required in (
            "crates/nagi-model/Cargo.toml",
            "crates/nagi-model/src/lib.rs",
            "user/nagi-model-manager/Cargo.toml",
            "user/nagi-model-manager/src/lib.rs",
        ):
            self.assertIn(required, sources)

    def test_legacy_binary_without_build_manifest_is_rejected(self) -> None:
        with self.assertRaisesRegex(PACKER.PackError, "build provenance is missing"):
            PACKER._verify_build_manifest(
                self.root / PACKER.BUILD_MANIFEST_NAME,
                self._manifest_fixture(),
            )

    def test_elf_must_carry_the_current_build_script_id(self) -> None:
        current_id = bytes.fromhex("12" * 20)
        executable = self.root / "with-build-id"
        write_elf(executable, build_id=current_id)
        PACKER._validate_build_id(executable, current_id.hex(), "synthetic CLI")
        with self.assertRaisesRegex(PACKER.PackError, "current build-script ID"):
            PACKER._validate_build_id(executable, bytes.fromhex("34" * 20).hex(), "old CLI")

    def test_elf_rejects_build_id_bytes_outside_a_gnu_note(self) -> None:
        current_id = bytes.fromhex("12" * 20)
        unnoted = self.root / "unnoted-build-id"
        write_elf(unnoted, unnoted_build_id=current_id)
        with self.assertRaisesRegex(PACKER.PackError, "current build-script ID"):
            PACKER._validate_build_id(unnoted, current_id.hex(), "unnoted CLI")

    def test_elf_rejects_matching_build_id_with_wrong_note_name(self) -> None:
        current_id = bytes.fromhex("12" * 20)
        wrong_name = self.root / "wrong-build-id-note-name"
        write_elf(wrong_name, build_id=current_id, build_id_note_name=b"NOTGNU\0")
        with self.assertRaisesRegex(PACKER.PackError, "current build-script ID"):
            PACKER._validate_build_id(wrong_name, current_id.hex(), "wrong-name CLI")

    def test_elf_rejects_matching_build_id_with_wrong_note_type(self) -> None:
        current_id = bytes.fromhex("12" * 20)
        wrong_type = self.root / "wrong-build-id-note-type"
        write_elf(wrong_type, build_id=current_id, build_id_note_type=1)
        with self.assertRaisesRegex(PACKER.PackError, "current build-script ID"):
            PACKER._validate_build_id(wrong_type, current_id.hex(), "wrong-type CLI")

    def test_elf_rejects_truncated_build_id_note(self) -> None:
        current_id = bytes.fromhex("12" * 20)
        truncated = self.root / "truncated-build-id-note"
        write_elf(truncated, build_id=current_id)
        data = bytearray(truncated.read_bytes())
        note_header = 64 + 56
        self.assertEqual(struct.unpack_from("<I", data, note_header)[0], 4)
        note_offset = struct.unpack_from("<Q", data, note_header + 8)[0]
        struct.pack_into("<I", data, note_offset + 4, 0xFFFFFFFF)
        truncated.write_bytes(data)
        with self.assertRaisesRegex(PACKER.PackError, "truncated ELF note entry"):
            PACKER._validate_build_id(truncated, current_id.hex(), "truncated CLI")

    def test_elf_rejects_program_segment_past_file_end(self) -> None:
        current_id = bytes.fromhex("12" * 20)
        invalid_segment = self.root / "invalid-note-segment-boundary"
        write_elf(invalid_segment, build_id=current_id)
        data = bytearray(invalid_segment.read_bytes())
        note_header = 64 + 56
        self.assertEqual(struct.unpack_from("<I", data, note_header)[0], 4)
        note_offset = struct.unpack_from("<Q", data, note_header + 8)[0]
        struct.pack_into("<Q", data, note_header + 32, len(data) - note_offset + 1)
        invalid_segment.write_bytes(data)
        with self.assertRaisesRegex(PACKER.PackError, "ELF segment extends beyond the file"):
            PACKER._validate_build_id(invalid_segment, current_id.hex(), "invalid-segment CLI")

    def test_elf_rejects_truncated_program_header_table(self) -> None:
        current_id = bytes.fromhex("12" * 20)
        truncated = self.root / "truncated-program-header-table"
        write_elf(truncated, build_id=current_id)
        data = bytearray(truncated.read_bytes())
        struct.pack_into("<Q", data, 32, len(data) - 1)
        truncated.write_bytes(data)
        with self.assertRaisesRegex(PACKER.PackError, "invalid ELF program header table"):
            PACKER._validate_build_id(truncated, current_id.hex(), "truncated-phdr CLI")

    def test_elf_rejects_truncated_note_header(self) -> None:
        current_id = bytes.fromhex("12" * 20)
        truncated = self.root / "truncated-note-header"
        write_elf(truncated, build_id=current_id)
        data = bytearray(truncated.read_bytes())
        note_header = 64 + 56
        note_offset = struct.unpack_from("<Q", data, note_header + 8)[0]
        struct.pack_into("<Q", data, note_header + 32, 11)
        truncated.write_bytes(data)
        self.assertTrue(any(data[note_offset : note_offset + 11]))
        with self.assertRaisesRegex(PACKER.PackError, "truncated ELF note table"):
            PACKER._validate_build_id(truncated, current_id.hex(), "truncated-note-header CLI")

    def test_elf_rejects_truncated_note_name(self) -> None:
        current_id = bytes.fromhex("12" * 20)
        truncated = self.root / "truncated-note-name"
        write_elf(truncated, build_id=current_id)
        data = bytearray(truncated.read_bytes())
        note_header = 64 + 56
        note_offset = struct.unpack_from("<Q", data, note_header + 8)[0]
        struct.pack_into("<I", data, note_offset, 0x1000)
        struct.pack_into("<I", data, note_offset + 4, 0)
        truncated.write_bytes(data)
        with self.assertRaisesRegex(PACKER.PackError, "truncated ELF note entry"):
            PACKER._validate_build_id(truncated, current_id.hex(), "truncated-note-name CLI")

    def test_elf_rejects_duplicate_gnu_build_id_notes(self) -> None:
        current_id = bytes.fromhex("12" * 20)
        duplicate = self.root / "duplicate-gnu-build-id-notes"
        write_elf(duplicate, build_id=current_id, duplicate_build_id_note=True)
        with self.assertRaisesRegex(PACKER.PackError, "current build-script ID"):
            PACKER._validate_build_id(duplicate, current_id.hex(), "duplicate-note CLI")

    def _run_apk_installer(
        self, *, install_exit: int = 0, signature_error: bool = False, complete_template: bool = True
    ) -> subprocess.CompletedProcess[str]:
        fake_apk = self.root / "fake-apk"
        fake_apk.write_text(
            "#!/bin/sh\n"
            'printf "%s\\n" "$*" >> "$APK_CALL_LOG"\n'
            'if [ "$1" = info ]; then test -f "$APK_INSTALLED_FILE"; exit $?; fi\n'
            'if [ "$1" = --no-network ]; then\n'
            '  if [ "${FAKE_APK_ADD_EXIT:-0}" -ne 0 ]; then\n'
            '    if [ "${FAKE_APK_SIGNATURE_ERROR:-0}" = 1 ]; then echo "ERROR: UNTRUSTED signature" >&2; fi\n'
            '    exit "$FAKE_APK_ADD_EXIT"\n'
            '  fi\n'
            '  : > "$APK_INSTALLED_FILE"\n'
            '  echo "OK: git packages installed"\n'
            '  exit 0\n'
            'fi\n'
            'exit 90\n',
            encoding="utf-8",
        )
        fake_apk.chmod(0o755)
        repository = self.root / "apk-repository"
        repository.mkdir(exist_ok=True)
        template = self.root / "git-template"
        if complete_template:
            (template / "info").mkdir(parents=True, exist_ok=True)
            (template / "info/exclude").write_text("# ignore local files\n", encoding="utf-8")
            (template / "description").write_text("Unnamed repository\n", encoding="utf-8")
        else:
            (template / "info").mkdir(parents=True, exist_ok=True)
            (template / "description").write_text("Unnamed repository\n", encoding="utf-8")
        env = os.environ.copy()
        env.update(
            {
                "APK": str(fake_apk),
                "APK_CALL_LOG": str(self.root / "apk-calls.log"),
                "APK_INSTALLED_FILE": str(self.root / "apk-installed"),
                "FAKE_APK_ADD_EXIT": str(install_exit),
                "FAKE_APK_SIGNATURE_ERROR": "1" if signature_error else "0",
                "NAGI_APK_REPOSITORY": str(repository),
                "NAGI_GIT_TEMPLATE_ROOT": str(template),
                "NAGI_APK_INSTALL_LOG": str(self.root / "apk-install.log"),
            }
        )
        return subprocess.run(
            ["sh", str(INSTALLER_SCRIPT)],
            cwd=self.root,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_guest_apk_installer_uses_default_trust_and_rejects_bad_signature(self) -> None:
        failed = self._run_apk_installer(install_exit=7, signature_error=True)
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("APK_SIGNATURE=FAIL", failed.stderr)
        self.assertNotIn("APK_SIGNATURE=PASS", failed.stdout)
        calls = (self.root / "apk-calls.log").read_text(encoding="utf-8")
        self.assertIn("--no-network --repository=", calls)
        self.assertIn("add git git-init-template", calls)
        self.assertNotIn("--allow-untrusted", calls)

    def test_guest_apk_installer_rejects_incomplete_git_template(self) -> None:
        result = self._run_apk_installer(complete_template=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("APK_SIGNATURE=PASS", result.stdout)
        self.assertIn("GIT_INIT_TEMPLATE=FAIL", result.stderr)
        self.assertNotIn("GUEST_GIT_READY=PASS", result.stdout)

    def test_guest_apk_installer_accepts_only_complete_default_trust_install(self) -> None:
        result = self._run_apk_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("APK_SIGNATURE=PASS", result.stdout)
        self.assertIn("GIT_INIT_TEMPLATE=PASS", result.stdout)
        self.assertIn("GUEST_GIT_READY=PASS", result.stdout)

    def test_ambiguous_cargo_artifact_fails_closed(self) -> None:
        cargo_target = self.root / "ambiguous" / "cargo-target"
        make_cargo_layout(cargo_target, duplicate="nagi_cli")
        with self.assertRaisesRegex(PACKER.PackError, "found 2"):
            PACKER.discover_artifacts(cargo_target)

    def test_selects_main_test_by_cargo_fingerprint(self) -> None:
        cargo_target = self.root / "fingerprinted" / "cargo-target"
        make_cargo_layout(cargo_target)
        artifacts = PACKER.discover_artifacts(cargo_target)
        self.assertEqual(artifacts["main_test"].name, "nagi-testhash")
        self.assertEqual(
            set(artifacts),
            {"debug_cli", "nagi_cli", "main_test", "cli", "run_receipt_cli"},
        )

    def test_debug_cli_requires_matching_cargo_source_dep_info(self) -> None:
        cargo_target = self.root / "debug-provenance" / "cargo-target"
        make_cargo_layout(cargo_target)
        dep_info = cargo_target / PACKER.TARGET / "debug/nagi.d"
        dep_info.write_text("nagi: tools/nagi-pkg/src/main.rs\n", encoding="utf-8")
        with self.assertRaisesRegex(PACKER.PackError, "debug_cli Cargo dep-info"):
            PACKER.discover_artifacts(cargo_target)

    def test_main_test_requires_matching_cargo_source_dep_info(self) -> None:
        cargo_target = self.root / "main-test-provenance" / "cargo-target"
        make_cargo_layout(cargo_target)
        dep_info = cargo_target / PACKER.TARGET / "debug/deps/nagi-testhash.d"
        dep_info.write_text("nagi-testhash: tools/nagi-pkg/src/main.rs\n", encoding="utf-8")
        with self.assertRaisesRegex(PACKER.PackError, "main_test Cargo dep-info"):
            PACKER.discover_artifacts(cargo_target)

    def test_dep_info_matches_exact_prerequisite_across_escaped_multiple_inputs(self) -> None:
        dep_info = self.root / "dep-info" / "valid.d"
        dep_info.parent.mkdir()
        dep_info.write_bytes(
            b"out: unrelated\\ path/file.rs tools/nagi-cli/src/main.rs \\\n"
            b"  another/dependency.rs\n\n"
            b"# env-dep:tools/nagi-cli/src/main.rs.bak\n"
            b"crlf: tools/nagi-cli/src/main.rs\r\n"
        )
        PACKER._validate_dep_info_source(
            dep_info,
            "tools/nagi-cli/src/main.rs",
            self.root,
            "test harness",
        )
        prerequisites = PACKER._makefile_prerequisites(dep_info.read_bytes())
        self.assertIn(b"unrelated path/file.rs", prerequisites)
        self.assertIn(b"another/dependency.rs", prerequisites)
        self.assertNotIn(b"tools/nagi-cli/src/main.rs.bak", prerequisites)

        spaced_source = self.root / "checkout with spaces" / "src/main.rs"
        escaped_source = os.fsencode(spaced_source).replace(b" ", b"\\ ")
        dep_info.write_bytes(b"out: " + escaped_source + b" another/dependency.rs\n")
        PACKER._validate_dep_info_source(
            dep_info,
            str(spaced_source),
            self.root,
            "spaced source",
        )

    def test_dep_info_rejects_embedded_carriage_return_but_accepts_crlf(self) -> None:
        dep_info = self.root / "embedded-cr.d"
        dep_info.write_bytes(
            b"out.d: tools/nagi-cli/src/main.rs\r.bak\n\n"
            b"tools/nagi-cli/src/main.rs\r.bak:\n"
        )
        with self.assertRaisesRegex(PACKER.PackError, "exact prerequisite"):
            PACKER._validate_dep_info_source(
                dep_info,
                "tools/nagi-cli/src/main.rs",
                self.root,
                "embedded CR source",
            )
        prerequisites = PACKER._makefile_prerequisites(dep_info.read_bytes())
        self.assertNotIn(b"tools/nagi-cli/src/main.rs", prerequisites)
        self.assertIn(b"tools/nagi-cli/src/main.rs\r.bak", prerequisites)

        crlf_info = self.root / "crlf.d"
        crlf_info.write_bytes(b"out.d: tools/nagi-cli/src/main.rs\r\n")
        PACKER._validate_dep_info_source(
            crlf_info,
            "tools/nagi-cli/src/main.rs",
            self.root,
            "CRLF source",
        )

    def test_dep_info_rejects_same_suffix_and_source_backup_substrings(self) -> None:
        cases = {
            "another-checkout": b"out: /tmp/other-checkout/tools/nagi-cli/src/main.rs\n",
            "backup-suffix": b"out: tools/nagi-cli/src/main.rs.bak\n",
        }
        for name, contents in cases.items():
            with self.subTest(name=name):
                dep_info = self.root / f"{name}.d"
                dep_info.write_bytes(contents)
                with self.assertRaisesRegex(PACKER.PackError, "exact prerequisite"):
                    PACKER._validate_dep_info_source(
                        dep_info,
                        "tools/nagi-cli/src/main.rs",
                        self.root,
                        name,
                    )

    def test_artifact_parent_symlink_is_rejected(self) -> None:
        cargo_target = self.root / "artifact-symlink" / "cargo-target"
        make_cargo_layout(cargo_target)
        target = cargo_target / PACKER.TARGET
        redirected_debug = self.root / "redirected-debug"
        shutil.move(target / "debug", redirected_debug)
        (target / "debug").symlink_to(redirected_debug, target_is_directory=True)
        with self.assertRaisesRegex(PACKER.PackError, "symlink path"):
            PACKER.discover_artifacts(cargo_target)

    def test_cache_parent_symlink_is_rejected(self) -> None:
        build_root = self.root / "cache-build"
        build_root.mkdir()
        outside_home = self.root / "redirected-cargo-home"
        outside_home.mkdir()
        (build_root / "cargo-home").symlink_to(outside_home, target_is_directory=True)
        cache = build_root / "cargo-home" / PACKER.LIBC_SOURCE
        with self.assertRaisesRegex(PACKER.PackError, "symlink path"):
            PACKER._assert_no_symlink_components(cache, build_root, "Cargo source cache")

    def test_synthetic_git_strips_inherited_repository_redirection(self) -> None:
        rootfs = self.root / "synthetic-root"
        rootfs.mkdir()
        (rootfs / ".dev").mkdir()
        (rootfs / ".dev/workstreams.json").write_text("{}\n", encoding="utf-8")
        redirected_git = self.root / "redirected.git"
        redirected_index = self.root / "redirected.index"
        redirected_objects = self.root / "redirected.objects"
        env = {
            "GIT_DIR": str(redirected_git),
            "GIT_WORK_TREE": str(rootfs),
            "GIT_INDEX_FILE": str(redirected_index),
            "GIT_OBJECT_DIRECTORY": str(redirected_objects),
            "GIT_CONFIG_COUNT": "1",
            "GIT_CONFIG_KEY_0": "core.worktree",
            "GIT_CONFIG_VALUE_0": str(rootfs),
        }
        with mock.patch.dict(os.environ, env):
            PACKER._initialize_synthetic_git(
                rootfs,
                self.root / "empty-template",
                [".dev/workstreams.json"],
            )
        self.assertTrue((rootfs / ".git").is_dir())
        self.assertFalse((redirected_git / "HEAD").exists())
        self.assertFalse(redirected_index.exists())
        self.assertFalse(redirected_objects.exists())

    def test_synthetic_git_rejects_ignored_required_input(self) -> None:
        rootfs = self.root / "ignored-root"
        rootfs.mkdir()
        (rootfs / ".gitignore").write_text("required.txt\n", encoding="utf-8")
        (rootfs / "required.txt").write_text("must be tracked\n", encoding="utf-8")
        with self.assertRaisesRegex(PACKER.PackError, "ignored required guest inputs"):
            PACKER._initialize_synthetic_git(
                rootfs,
                self.root / "ignored-template",
                ["required.txt"],
            )

    def test_synthetic_workstream_fixture_is_sanitized_and_complete(self) -> None:
        rootfs = self.root / "synthetic-workstream"
        rootfs.mkdir()
        written = PACKER._write_synthetic_workstream_files(rootfs)
        self.assertEqual(
            set(written),
            {
                ".dev/workstreams.json",
                PACKER.SYNTHETIC_WORKSTREAM_STATE_PATH,
                PACKER.SYNTHETIC_INTEGRATION_STATE_PATH,
            },
        )
        registry = json.loads((rootfs / ".dev/workstreams.json").read_text(encoding="utf-8"))
        foundation = next(
            entry
            for entry in registry["workstreams"]
            if entry["id"] == "development-foundation"
        )
        self.assertEqual(
            foundation["state_file"], PACKER.SYNTHETIC_WORKSTREAM_STATE_PATH
        )
        state = json.loads(
            (rootfs / PACKER.SYNTHETIC_WORKSTREAM_STATE_PATH).read_text(encoding="utf-8")
        )
        self.assertEqual(state["workstream_id"], "development-foundation")
        self.assertEqual(state["status"], "NOT_STARTED")
        self.assertIn("No product workstream state", state["last_verified"]["normal_state"])
        self.assertEqual(state["last_verified"]["ci_runs"], [])
        integration = next(
            entry
            for entry in registry["workstreams"]
            if entry["id"] == "integration-next-phase"
        )
        self.assertEqual(
            integration["state_file"], PACKER.SYNTHETIC_INTEGRATION_STATE_PATH
        )
        integration_state = json.loads(
            (rootfs / PACKER.SYNTHETIC_INTEGRATION_STATE_PATH).read_text(encoding="utf-8")
        )
        self.assertEqual(integration_state["workstream_id"], "integration-next-phase")
        self.assertEqual(integration_state["status"], "NOT_STARTED")

    def test_pinned_supplemental_source_stages_only_hash_verified_file(self) -> None:
        relative = "third_party/mozjs-sys-nagi/build.rs"
        content = b"synthetic pinned build script\n"
        digest = hashlib.sha256(content).hexdigest()
        source_root = self.root / "supplemental-source"
        source = source_root / relative
        source.parent.mkdir(parents=True)
        source.write_bytes(content)
        stage = self.root / "supplemental-stage"
        with mock.patch.dict(PACKER.PINNED_SUPPLEMENTAL_FILES, {relative: digest}, clear=True):
            copied = PACKER._copy_pinned_supplemental_files(source_root, stage)
            self.assertEqual(copied, [relative])
            self.assertEqual((stage / relative).read_bytes(), content)

            changed_root = self.root / "changed-source"
            changed = changed_root / relative
            changed.parent.mkdir(parents=True)
            changed.write_bytes(b"different source\n")
            with self.assertRaisesRegex(PACKER.PackError, "hash does not match pinned"):
                PACKER._copy_pinned_supplemental_files(
                    changed_root, self.root / "changed-stage"
                )

            linked_root = self.root / "linked-source"
            linked = linked_root / relative
            linked.parent.mkdir(parents=True)
            linked.symlink_to(source)
            with self.assertRaisesRegex(PACKER.PackError, "symlink path"):
                PACKER._copy_pinned_supplemental_files(
                    linked_root, self.root / "linked-stage"
                )

    def test_fixed_inputs_survive_archive_round_trip_and_missing_inputs_fail(self) -> None:
        self.assertIn("nagi", PACKER.SOURCE_PATHS)
        self.assertIn("nagi", PACKER.REQUIRED_SOURCE_FILES)
        installer = "tools/nagi-cli/scripts/install-alpine-test-deps.sh"
        self.assertIn(installer, PACKER.SOURCE_PATHS)
        self.assertIn(installer, PACKER.REQUIRED_SOURCE_FILES)
        self.assertIn(".github/workflows/ci.yml", PACKER.SOURCE_PATHS)
        self.assertIn(".github/workflows/ci.yml", PACKER.REQUIRED_SOURCE_FILES)
        abi_source = "crates/nagi-abi/src/lib.rs"
        self.assertIn(abi_source, PACKER.SOURCE_PATHS)
        self.assertIn(abi_source, PACKER.REQUIRED_SOURCE_FILES)
        relative = "third_party/mozjs-sys-nagi/build.rs"
        content = b"synthetic pinned build script\n"
        digest = hashlib.sha256(content).hexdigest()
        rootfs = self.root / "fixed-input-root"
        rootfs.mkdir()
        (rootfs / "nagi").write_text("#!/usr/bin/env sh\nexit 0\n", encoding="utf-8")
        (rootfs / ".github/workflows").mkdir(parents=True)
        (rootfs / ".github/workflows/ci.yml").write_text("name: test\n", encoding="utf-8")
        abi_file = rootfs / abi_source
        abi_file.parent.mkdir(parents=True)
        abi_file.write_text("pub const SYS_MEMORY_MAP_AT: u64 = 27;\n", encoding="utf-8")
        installer_file = rootfs / installer
        installer_file.parent.mkdir(parents=True, exist_ok=True)
        installer_file.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        PACKER._write_synthetic_workstream_files(rootfs)
        source_root = self.root / "fixed-input-source"
        source = source_root / relative
        source.parent.mkdir(parents=True)
        source.write_bytes(content)
        with mock.patch.dict(
            PACKER.PINNED_SUPPLEMENTAL_FILES, {relative: digest}, clear=True
        ):
            PACKER._copy_pinned_supplemental_files(source_root, rootfs)
            archive = self.root / "fixed-inputs.tar.gz"
            PACKER._write_archive(rootfs, archive)
            extracted = self.root / "fixed-input-extracted"
            PACKER.safe_extract(archive, extracted)
            PACKER._verify_fixed_guest_inputs(extracted)
            self.assertEqual(
                (extracted / ".github/workflows/ci.yml").read_text(encoding="utf-8"),
                "name: test\n",
            )
            self.assertEqual(
                (extracted / "nagi").read_text(encoding="utf-8"),
                "#!/usr/bin/env sh\nexit 0\n",
            )
            self.assertEqual(
                (extracted / abi_source).read_text(encoding="utf-8"),
                "pub const SYS_MEMORY_MAP_AT: u64 = 27;\n",
            )
            self.assertEqual((extracted / relative).read_bytes(), content)

            (extracted / ".github/workflows/ci.yml").unlink()
            with self.assertRaisesRegex(
                PACKER.PackError,
                "missing required fixed input: .github/workflows/ci.yml",
            ):
                PACKER._verify_fixed_guest_inputs(extracted)

            (extracted / ".github/workflows/ci.yml").write_text(
                "name: test\n", encoding="utf-8"
            )
            (extracted / abi_source).unlink()
            with self.assertRaisesRegex(
                PACKER.PackError,
                "missing required fixed input: crates/nagi-abi/src/lib.rs",
            ):
                PACKER._verify_fixed_guest_inputs(extracted)

    def test_synthetic_git_force_adds_only_the_pinned_build_script(self) -> None:
        rootfs = self.root / "forced-source-root"
        rootfs.mkdir()
        relative = "third_party/mozjs-sys-nagi/build.rs"
        (rootfs / ".gitignore").write_text(
            "/third_party/mozjs-sys-nagi/\n", encoding="utf-8"
        )
        build_script = rootfs / relative
        build_script.parent.mkdir(parents=True)
        build_script.write_text("pinned build script\n", encoding="utf-8")
        workstream_paths = PACKER._write_synthetic_workstream_files(rootfs)
        required = [".gitignore", *workstream_paths, relative]
        PACKER._initialize_synthetic_git(
            rootfs,
            self.root / "force-add-template",
            required,
            force_add=[relative],
        )
        tracked = set(PACKER._run_git(rootfs, ["ls-files", "-z"]).decode().split("\0"))
        self.assertIn(relative, tracked)
        self.assertNotIn("third_party/mozjs-sys-nagi/Cargo.toml", tracked)

    def test_source_symlink_cannot_escape_checkout(self) -> None:
        source = self.root / "source"
        destination = self.root / "stage"
        source.mkdir()
        outside = self.root / "outside.txt"
        outside.write_text("not guest input", encoding="utf-8")
        (source / "link.txt").symlink_to(outside)
        with self.assertRaisesRegex(PACKER.PackError, "symlink source"):
            PACKER._copy_regular_file(source, destination, "link.txt")
        self.assertFalse((destination / "link.txt").exists())

    def test_safe_extract_rejects_parent_traversal(self) -> None:
        archive = self.root / "traversal.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            member = tarfile.TarInfo("../outside.txt")
            member.size = 3
            import io

            tar.addfile(member, io.BytesIO(b"bad"))
        destination = self.root / "extracted"
        with self.assertRaisesRegex(PACKER.PackError, "unsafe archive path"):
            PACKER.safe_extract(archive, destination)
        self.assertFalse((self.root / "outside.txt").exists())

    def test_safe_extract_rejects_links(self) -> None:
        archive = self.root / "link.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            member = tarfile.TarInfo("escape")
            member.type = tarfile.SYMTYPE
            member.linkname = "../outside.txt"
            tar.addfile(member)
        with self.assertRaisesRegex(PACKER.PackError, "link or special file"):
            PACKER.safe_extract(archive, self.root / "link-extracted")


if __name__ == "__main__":
    unittest.main()
