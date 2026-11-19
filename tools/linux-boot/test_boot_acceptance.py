"""Synthetic bytes only; no tempdirs, guest execution or source modification."""
import hashlib
import io
import stat
import unittest
import json
import os
from contextlib import ExitStack, contextmanager
from types import SimpleNamespace
from unittest.mock import patch

import boot_acceptance as acceptance
import prepare
import replace_initramfs_init as initramfs_repack
import test_prepare as existing_prepare
from test_prepare import synthetic_static_elf


def archive(entries):
    result = bytearray()
    for ino, entry in enumerate(entries + [("TRAILER!!!", b"", 0, 1)], 1):
        name, payload, mode, nlink, *device = entry
        rdevmajor, rdevminor = device if device else (0, 0)
        name = name.encode("ascii") + b"\0"
        fields = [ino, mode, 0, 0, nlink, 0, len(payload), 0, 0,
                  rdevmajor, rdevminor, len(name), 0]
        result.extend(b"070701" + b"".join(f"{x:08x}".encode() for x in fields))
        result.extend(name)
        result.extend(bytes(-len(result) % 4))
        result.extend(payload)
        result.extend(bytes(-len(result) % 4))
    return bytes(result)


def bundle(elf=None, name="init", mode=stat.S_IFREG | 0o755, nlink=1):
    return archive([(name, synthetic_static_elf() if elf is None else elf, mode, nlink)])


def prepared_entries(elf=None):
    elf = synthetic_static_elf() if elf is None else elf
    return [
        ("dev", b"", stat.S_IFDIR | 0o755, 2),
        ("dev/console", b"", stat.S_IFCHR | 0o600, 1, 5, 1),
        ("dev/null", b"", stat.S_IFCHR | 0o666, 1, 1, 3),
        ("dev/ttyS0", b"", stat.S_IFCHR | 0o600, 1, 4, 64),
        ("proc", b"", stat.S_IFDIR | 0o555, 2),
        ("sys", b"", stat.S_IFDIR | 0o555, 2),
        ("run", b"", stat.S_IFDIR | 0o755, 2),
        ("tmp", b"", stat.S_IFDIR | 0o1777, 2),
        ("init", elf, stat.S_IFREG | 0o755, 1),
    ]


def prepared_bundle(elf=None, entries=None):
    return archive(prepared_entries(elf) if entries is None else entries)


def candidate_serial(*extra_lines):
    lines = ["Linux version 6.18.63-nagi (x86_64)",
             "Run /init as init process", acceptance._UNAVAILABLE]
    lines.extend(extra_lines)
    return ("\n".join(lines) + "\n").encode()


class MemoryFiles:
    """Test-only OS I/O model. All unknown paths/fds fail, never fall through.

    Exercises existing prepare assertions and real parser/generator bodies, not
    actual filesystem durability, race resistance or platform syscall semantics.
    """
    def __init__(self):
        self.nodes = {}
        self.fds = {}
        self.next_fd = 100000
        self.add_directory("/")
        self.add_directory("/memory")

    def path(self, path, dir_fd=None):
        path = os.fspath(path)
        if not path.startswith("/"):
            path = os.path.join(self.fds[dir_fd][0], path)
        return os.path.normpath(path)

    def add_directory(self, path):
        self.nodes[path] = SimpleNamespace(mode=stat.S_IFDIR | 0o755, data=bytearray(),
                                           ino=len(self.nodes) + 1, target=None)

    def node(self, path, follow=True):
        path = self.path(path)
        if path not in self.nodes:
            raise FileNotFoundError(path)
        node = self.nodes[path]
        if follow and node.target is not None:
            return self.nodes[node.target]
        return node

    def metadata(self, path, *, follow_symlinks=True):
        n = self.node(path, follow_symlinks)
        return SimpleNamespace(st_mode=n.mode, st_dev=1, st_ino=n.ino, st_size=len(n.data))

    def mkdir(self, path, mode=0o777, *, dir_fd=None):
        path = self.path(path, dir_fd)
        if path in self.nodes:
            raise FileExistsError(path)
        assert stat.S_ISDIR(self.node(os.path.dirname(path)).mode)
        self.add_directory(path)

    def write_bytes(self, path, data):
        path = self.path(path)
        assert stat.S_ISDIR(self.node(os.path.dirname(path)).mode)
        if path not in self.nodes:
            self.nodes[path] = SimpleNamespace(mode=stat.S_IFREG | 0o644, data=bytearray(),
                                               ino=len(self.nodes) + 1, target=None)
        self.node(path).data[:] = data
        return len(data)

    def read_bytes(self, path):
        return bytes(self.node(path).data)

    def open(self, path, flags, mode=0o777, *, dir_fd=None):
        path = self.path(path, dir_fd)
        if path in self.nodes:
            if flags & os.O_CREAT and flags & os.O_EXCL:
                raise FileExistsError(path)
            if flags & getattr(os, "O_NOFOLLOW", 0) and self.node(path, False).target is not None:
                raise OSError("symlink")
        elif flags & os.O_CREAT:
            self.write_bytes(path, b"")
        else:
            raise FileNotFoundError(path)
        self.next_fd += 1
        self.fds[self.next_fd] = [path, 0]
        return self.next_fd

    def read(self, fd, count):
        path, offset = self.fds[fd]
        data = self.read_bytes(path)[offset:offset + count]
        self.fds[fd][1] += len(data)
        return data

    def write(self, fd, data):
        path, offset = self.fds[fd]
        self.node(path).data[offset:offset + len(data)] = data
        self.fds[fd][1] += len(data)
        return len(data)

    def seek(self, fd, offset, whence):
        path, position = self.fds[fd]
        self.fds[fd][1] = offset + (position if whence == os.SEEK_CUR else
                                  len(self.node(path).data) if whence == os.SEEK_END else 0)
        return self.fds[fd][1]

    def close(self, fd):
        del self.fds[fd]

    def symlink(self, target, path):
        self.write_bytes(path, b"")
        self.nodes[path].mode = stat.S_IFLNK | 0o777
        self.nodes[path].target = target

    def unlink(self, path, *, dir_fd=None):
        path = self.path(path, dir_fd)
        if path not in self.nodes:
            raise FileNotFoundError(path)
        del self.nodes[path]

    @contextmanager
    def fdopen(self, fd, mode):
        assert mode == "rb"
        try:
            yield io.BytesIO(self.read_bytes(self.fds[fd][0]))
        finally:
            self.close(fd)


def existing_prepare_in_memory():
    """Run original PrepareTests methods; no original scratch setup or cleanup.

    Two original fixture names have no CONFIGS entry. Add explicit arm64 aliases
    only in this adapter, preserving each original negative test's assertions.
    The real minimum config is read before the memory-only operation boundary.
    """
    from pathlib import Path
    config_path = os.path.join(os.path.dirname(existing_prepare.MODULE_PATH), "kernel-minimum.config")
    config = Path(config_path).read_bytes()
    memory = MemoryFiles()
    class AdaptedPrepareTests(existing_prepare.PrepareTests):
        def setUp(self):
            memory.__init__()
            # Exact source config is seeded as fixed test data in the model.
            parent = "/"
            for part in os.path.dirname(config_path).split("/"):
                if part:
                    parent = os.path.join(parent, part)
                    if parent not in memory.nodes:
                        memory.add_directory(parent)
            memory.write_bytes(config_path, config)
            self.work = "/memory/work"
            memory.mkdir(self.work)
            self.inputs = {"x86_64": self._write_inputs("x86_64")}

        def tearDown(self):
            self.assertFalse(memory.fds, "prepare leaked model descriptors")

    with ExitStack() as stack:
        stack.enter_context(patch("builtins.open", side_effect=AssertionError("unexpected real file I/O")))
        stack.enter_context(patch.object(existing_prepare.tempfile, "mkdtemp", side_effect=AssertionError("scratch forbidden")))
        stack.enter_context(patch.object(existing_prepare.shutil, "rmtree", side_effect=AssertionError("cleanup forbidden")))
        operations = {"open": memory.open, "close": memory.close, "read": memory.read,
                      "write": memory.write, "lseek": memory.seek, "mkdir": memory.mkdir,
                      "symlink": memory.symlink, "unlink": memory.unlink, "rmdir": memory.unlink,
                      "stat": memory.metadata, "lstat": lambda p: memory.metadata(p, follow_symlinks=False),
                      "fstat": lambda fd: memory.metadata(memory.fds[fd][0]),
                      "pread": lambda fd, size, offset: memory.read_bytes(memory.fds[fd][0])[offset:offset + size],
                      "fsync": lambda fd: memory.fds[fd], "fdopen": memory.fdopen}
        for name, function in operations.items():
            stack.enter_context(patch.object(os, name, side_effect=function))
        stack.enter_context(patch.object(Path, "read_bytes", lambda path: memory.read_bytes(path)))
        stack.enter_context(patch.object(Path, "write_bytes", lambda path, data: memory.write_bytes(path, data)))
        stack.enter_context(patch.dict(existing_prepare.CONFIGS, {
            "aarch64-big-endian": existing_prepare.CONFIGS["aarch64"],
            "aarch64-wrong-config": existing_prepare.CONFIGS["aarch64"]}))
        result = unittest.TestResult()
        unittest.defaultTestLoader.loadTestsFromTestCase(AdaptedPrepareTests).run(result)
    return result


class AcceptanceTests(unittest.TestCase):
    def assert_code(self, code, function, *args, **kwargs):
        with self.assertRaises(acceptance.AcceptanceError) as caught:
            function(*args, **kwargs)
        self.assertEqual(caught.exception.code, code)

    def test_valid_static_identity_is_not_execution(self):
        result = acceptance.validate_artifacts(synthetic_static_elf(), prepared_bundle())
        self.assertEqual(result["cpio_init_byte_identity"], "PASS")
        self.assertEqual(result["x86_prepared_cpio_contract"], "PASS")
        self.assertFalse(result["guest_execution_verified"])

    def test_appending_package_preserves_records_and_trailer(self):
        original = bundle()
        package = b"signed developer-preview package"
        packed, entries = initramfs_repack._append_regular_file(
            original, "nagi-dev-preview-linux.xapp", package
        )
        _init, original_entries = acceptance._parse_newc(original)
        _packed_init, packed_entries = acceptance._parse_newc(packed)

        self.assertEqual(packed_entries[:-2], original_entries[:-1])
        self.assertEqual(packed_entries[-2], entries[-2])
        self.assertEqual(packed_entries[-2][0], "nagi-dev-preview-linux.xapp")
        self.assertEqual(packed_entries[-2][1][1], stat.S_IFREG | 0o644)
        self.assertEqual(packed_entries[-1], original_entries[-1])
        self.assertEqual(initramfs_repack._payload_for(packed, "nagi-dev-preview-linux.xapp"), package)

    def test_appending_package_rejects_duplicate_and_unsafe_names(self):
        original = bundle()
        packed, _entries = initramfs_repack._append_regular_file(
            original, "nagi-dev-preview-linux.xapp", b"package"
        )
        for archive_bytes, name in [
            (packed, "nagi-dev-preview-linux.xapp"),
            (original, "../nagi-dev-preview-linux.xapp"),
        ]:
            with self.subTest(name=name):
                with self.assertRaises(initramfs_repack.RepackError):
                    initramfs_repack._append_regular_file(archive_bytes, name, b"package")

    def test_development_cpio_inputs_match_exact_paths_modes_and_bytes(self):
        package = b"signed development package input"
        server = synthetic_static_elf()
        entries = [
            ("init", synthetic_static_elf(), stat.S_IFREG | 0o755, 1),
            ("bin", b"", stat.S_IFDIR | 0o755, 2),
            ("run", b"", stat.S_IFDIR | 0o755, 2),
            ("run/nagi", b"", stat.S_IFDIR | 0o700, 2),
            ("bin/dev-service", server, stat.S_IFREG | 0o755, 1),
            ("nagi-dev-preview-linux.xapp", package, stat.S_IFREG | 0o644, 1),
        ]
        cpio = archive(entries)
        expected = {
            "nagi-dev-preview-linux.xapp": (0o644, package),
            "bin/dev-service": (0o755, server),
        }
        result = acceptance.validate_development_cpio_inputs(
            cpio,
            expected,
            expected_directories={"run/nagi": 0o700},
        )
        self.assertEqual(result["cpio_structure"], "PASS")
        self.assertEqual(result["development_input_bytes_match"], "PASS")
        self.assertEqual(result["development_directories"], [{"path": "run/nagi", "mode": 0o700}])
        self.assertFalse(result["service_execution_verified"])
        self.assertFalse(result["handshake_verified"])
        self.assertFalse(result["ready_verified"])

        directory_only = acceptance.validate_development_cpio_inputs(
            cpio,
            {},
            expected_directories={"run/nagi": 0o700},
        )
        self.assertEqual(directory_only["development_input_bytes_match"], "NOT_CHECKED")

        self.assert_code(
            "DEV_CPIO_INPUT_MISSING",
            acceptance.validate_development_cpio_inputs,
            cpio,
            {"nagi-missing.xapp": (0o644, package)},
        )
        self.assert_code(
            "DEV_CPIO_INPUT_MISMATCH",
            acceptance.validate_development_cpio_inputs,
            cpio,
            {"nagi-dev-preview-linux.xapp": (0o644, package + b"-changed")},
        )
        self.assert_code(
            "DEV_CPIO_INPUT_MODE",
            acceptance.validate_development_cpio_inputs,
            cpio,
            {"nagi-dev-preview-linux.xapp": (0o600, package)},
        )
        self.assert_code(
            "DEV_CPIO_PARENT_MISSING",
            acceptance.validate_development_cpio_inputs,
            archive([
                ("init", synthetic_static_elf(), stat.S_IFREG | 0o755, 1),
                ("bin/dev-service", server, stat.S_IFREG | 0o755, 1),
            ]),
            {"bin/dev-service": (0o755, server)},
        )
        self.assert_code(
            "DEV_CPIO_INPUT_MISSING",
            acceptance.validate_development_cpio_inputs,
            archive([
                ("init", synthetic_static_elf(), stat.S_IFREG | 0o755, 1),
                ("run", b"", stat.S_IFDIR | 0o755, 2),
            ]),
            {},
            expected_directories={"run/nagi": 0o700},
        )
        self.assert_code(
            "DEV_CPIO_DIRECTORY_MODE",
            acceptance.validate_development_cpio_inputs,
            archive([
                ("init", synthetic_static_elf(), stat.S_IFREG | 0o755, 1),
                ("run", b"", stat.S_IFDIR | 0o755, 2),
                ("run/nagi", b"", stat.S_IFDIR | 0o755, 2),
            ]),
            {},
            expected_directories={"run/nagi": 0o700},
        )
        self.assert_code(
            "CPIO_DUPLICATE",
            acceptance.validate_development_cpio_inputs,
            archive(entries + [("nagi-dev-preview-linux.xapp", package, stat.S_IFREG | 0o644, 1)]),
            {"nagi-dev-preview-linux.xapp": (0o644, package)},
        )
        self.assert_code(
            "CPIO_ENTRY_TYPE",
            acceptance.validate_development_cpio_inputs,
            archive(entries + [("bin/link", b"", stat.S_IFLNK | 0o777, 1)]),
            {"nagi-dev-preview-linux.xapp": (0o644, package)},
        )
        self.assert_code(
            "DEV_CPIO_EXPECTED_PATH",
            acceptance.validate_development_cpio_inputs,
            cpio,
            {"../nagi-dev-preview-linux.xapp": (0o644, package)},
        )

    def test_reuses_prepare_elf_contract(self):
        for elf in [synthetic_static_elf(interp=True), synthetic_static_elf(dynamic=True),
                    synthetic_static_elf("aarch64"), b"not ELF"]:
            with self.subTest(size=len(elf)):
                self.assert_code("ELF_CONTRACT", acceptance.validate_artifacts, elf, prepared_bundle(elf))

    def test_fd_and_bytes_contract_parity(self):
        for elf in [synthetic_static_elf(), synthetic_static_elf(dynamic=True), b"short"]:
            def pread(_fd, size, offset):
                return elf[offset:offset + size]
            with patch.object(prepare.os, "pread", side_effect=pread):
                if elf == synthetic_static_elf():
                    prepare._validate_init_elf(97, len(elf), "x86_64")
                else:
                    with self.assertRaises(prepare.InputError):
                        prepare._validate_init_elf(97, len(elf), "x86_64")

    def test_actual_generator_with_memory_only_io(self):
        elf = synthetic_static_elf()
        source = io.BytesIO(elf)
        output = io.BytesIO()
        def read(fd, count):
            self.assertEqual(fd, 98)
            return source.read(count)
        def write(fd, block):
            self.assertEqual(fd, 99)
            return output.write(block)
        with patch.object(prepare.os, "read", side_effect=read), \
             patch.object(prepare.os, "write", side_effect=write), \
             patch.object(prepare.os, "lseek", side_effect=lambda fd, off, how: source.seek(off, how)):
            digest, size = prepare._write_initramfs(99, 98, len(elf), hashlib.sha256(elf).hexdigest(), "x86_64")
        self.assertEqual(size, len(output.getvalue()))
        self.assertEqual(digest, hashlib.sha256(output.getvalue()).hexdigest())
        acceptance.validate_artifacts(elf, output.getvalue())
        for end in range(len(output.getvalue())):
            with self.subTest(generator_prefix=end):
                with self.assertRaises(acceptance.AcceptanceError):
                    acceptance._init_from_newc(output.getvalue()[:end])

    def test_wrong_bytes(self):
        self.assert_code("CPIO_INIT_MISMATCH", acceptance.validate_artifacts,
                         synthetic_static_elf(), prepared_bundle(b"changed"))

    def test_closed_x86_prepared_cpio_contract(self):
        elf = synthetic_static_elf()
        entries = prepared_entries(elf)
        acceptance.validate_artifacts(elf, prepared_bundle(elf, entries))

        with_extra_sbin_init = entries + [
            ("sbin", b"", stat.S_IFDIR | 0o755, 2),
            ("sbin/init", b"fallback", stat.S_IFREG | 0o755, 1),
        ]
        self.assert_code("CPIO_PREPARE_ENTRY_SET", acceptance.validate_artifacts,
                         elf, prepared_bundle(elf, with_extra_sbin_init))

        wrong_console_device = list(entries)
        wrong_console_device[1] = ("dev/console", b"", stat.S_IFCHR | 0o600, 1, 1, 3)
        self.assert_code("CPIO_PREPARE_CONSOLE_DEVICE", acceptance.validate_artifacts,
                         elf, prepared_bundle(elf, wrong_console_device))

        missing_parent = entries + [("a/x", b"x", stat.S_IFREG | 0o644, 1)]
        self.assert_code("CPIO_PREPARE_ENTRY_SET", acceptance.validate_artifacts,
                         elf, prepared_bundle(elf, missing_parent))

        non_init_setuid = list(entries)
        non_init_setuid[6] = ("run", b"", stat.S_IFDIR | stat.S_ISUID | 0o755, 2)
        self.assert_code("CPIO_PRIVILEGED_MODE", acceptance.validate_artifacts,
                         elf, prepared_bundle(elf, non_init_setuid))

    def test_x86_config_uses_external_bootloader_initramfs(self):
        base = existing_prepare.CONFIGS["x86_64"]
        accepted = prepare._parse_kernel_config(
            base + b'CONFIG_INITRAMFS_SOURCE=""\n', "x86_64")
        self.assertEqual(accepted["CONFIG_INITRAMFS_SOURCE"], '""')
        for addition, message in [
            (b'CONFIG_INITRAMFS_SOURCE="embedded.cpio"\n', "CONFIG_INITRAMFS_SOURCE"),
            (b"CONFIG_INITRAMFS_FORCE=y\n", "CONFIG_INITRAMFS_FORCE"),
        ]:
            with self.subTest(addition=addition):
                with self.assertRaisesRegex(prepare.InputError, message):
                    prepare._parse_kernel_config(base + addition, "x86_64")

    def test_duplicate_and_alias_paths(self):
        item = ("init", synthetic_static_elf(), stat.S_IFREG | 0o755, 1)
        self.assert_code("CPIO_DUPLICATE", acceptance._init_from_newc, archive([item, item]))
        for name in ["/init", "./init", "a/../init", "a//init", "a\\init"]:
            with self.subTest(name=name):
                self.assert_code("CPIO_NAME", acceptance._init_from_newc, bundle(name=name))

    def test_missing_or_nonexecutable_init(self):
        self.assert_code("CPIO_INIT_MISSING", acceptance._init_from_newc, archive([]))
        for mode in [stat.S_IFREG | 0o644, stat.S_IFREG | 0o4755, stat.S_IFDIR | 0o755]:
            with self.subTest(mode=mode):
                # Nonempty directory is rejected before its /init mode.
                code = "CPIO_ENTRY_DATA" if stat.S_ISDIR(mode) else "CPIO_INIT_MODE"
                self.assert_code(code, acceptance._init_from_newc, bundle(mode=mode, nlink=2 if stat.S_ISDIR(mode) else 1))

    def test_symlink_hardlink_and_init_descendant(self):
        self.assert_code("CPIO_ENTRY_TYPE", acceptance._init_from_newc, bundle(mode=stat.S_IFLNK | 0o755))
        self.assert_code("CPIO_LINK", acceptance._init_from_newc, bundle(nlink=2))
        self.assert_code("CPIO_INIT_PATH_CONFLICT", acceptance._init_from_newc, bundle(name="init/child"))

    def test_bad_header_padding_truncation_and_trailer(self):
        data = bundle()
        self.assert_code("CPIO_HEADER", acceptance._init_from_newc, b"070702" + data[6:])
        self.assert_code("CPIO_HEADER", acceptance._init_from_newc, data[:6] + b"z" + data[7:])
        changed = bytearray(data)
        changed[115] = 1  # name padding following init\0
        self.assert_code("CPIO_PADDING", acceptance._init_from_newc, bytes(changed))
        self.assert_code("CPIO_TRUNCATED", acceptance._init_from_newc, data[:120])
        self.assert_code("CPIO_TRAILER", acceptance._init_from_newc, data + data)
        self.assert_code("CPIO_TRAILER", acceptance._init_from_newc, data + b"unexpected payload")
        # Zero-only tail padding is permitted, but no later archive or data.
        self.assertEqual(acceptance._init_from_newc(data + bytes(512)), synthetic_static_elf())
        self.assert_code("CPIO_NAME", acceptance._init_from_newc, data[:-4])

    def test_repeated_inode_cannot_alias_init(self):
        elf = synthetic_static_elf()
        data = bytearray(archive([("other", b"x", stat.S_IFREG | 0o755, 1),
                                  ("init", elf, stat.S_IFREG | 0o755, 1)]))
        second = (110 + len(b"other\0") + 3) & ~3
        second = (second + 1 + 3) & ~3
        data[second + 6:second + 14] = b"00000001"
        self.assert_code("CPIO_LINK", acceptance._init_from_newc, bytes(data))

    def test_cpio_checksum_and_oversized_name(self):
        for offset, value in [(102, b"00000001"), (94, b"ffffffff")]:
            data = bytearray(bundle())
            data[offset:offset + 8] = value
            self.assert_code("CPIO_HEADER", acceptance._init_from_newc, bytes(data))
        # c_filesize is exactly 8 hex digits. The maximal declared payload
        # exceeds the input: Python's cursor arithmetic must not wrap to it.
        data = bytearray(bundle())
        data[54:62] = b"ffffffff"
        self.assert_code("CPIO_TRUNCATED", acceptance._init_from_newc, bytes(data))

    def test_missing_trailer(self):
        data = bundle()
        trailer = data.index(b"070701", 6)
        self.assert_code("CPIO_TRAILER_MISSING", acceptance._init_from_newc, data[:trailer])

    def test_immutable_bytes_only(self):
        self.assert_code("ELF_INPUT_LIMIT", acceptance.validate_artifacts,
                         bytearray(synthetic_static_elf()), prepared_bundle())
        self.assert_code("CPIO_INPUT_LIMIT", acceptance._init_from_newc, bytearray(bundle()))
        self.assert_code("SERIAL_INPUT_LIMIT", acceptance.observe_serial, "READY\n")

    def test_bounds_without_real_large_allocations(self):
        self.assert_code("SERIAL_LINE", acceptance.observe_serial, b"x" * 65537 + b"\n")
        self.assert_code("SERIAL_LINE", acceptance.observe_serial, b"\n" * 65537)
        self.assert_code("SERIAL_LINE", acceptance.observe_serial, b"x" * 65537)
        self.assert_code("SERIAL_LINE", acceptance.observe_serial, b"partial\r")

    def test_all_byte_truncations(self):
        elf = synthetic_static_elf()
        cpio = bundle()
        for end in range(len(cpio)):
            with self.subTest(kind="cpio", end=end):
                with self.assertRaises(acceptance.AcceptanceError):
                    acceptance._init_from_newc(cpio[:end])
        for end in range(len(elf) + 1):
            truncated = elf[:end]
            with self.subTest(kind="elf", end=end):
                try:
                    prepare.validate_init_elf_bytes(truncated)
                except prepare.InputError:
                    self.assert_code("ELF_CONTRACT", acceptance.validate_artifacts,
                                     truncated, prepared_bundle(truncated))
                else:
                    # Tail padding is optional ELF data; removing it is valid.
                    self.assertGreaterEqual(end, 0x104)
                    acceptance.validate_artifacts(truncated, prepared_bundle(truncated))
        serial = candidate_serial()
        for end in range(len(serial) + 1):
            with self.subTest(kind="serial", end=end):
                result = acceptance.observe_serial(serial[:end])
                self.assertFalse(result["actual_pid1_verified"])
                self.assertFalse(result["minimal_service_ready_verified"])
                self.assertEqual(result["ordered_post_guard_candidate"], end == len(serial))

    def test_boundary_fields_paths_and_misleading_markers(self):
        data = bundle()
        for field in range(13):
            for value in [b"00000000", b"ffffffff", b"gggggggg"]:
                mutated = data[:6 + field * 8] + value + data[14 + field * 8:]
                with self.subTest(field=field, value=value):
                    try:
                        payload = acceptance._init_from_newc(mutated)
                    except acceptance.AcceptanceError:
                        pass
                    else:
                        # Some metadata fields do not affect byte identity.
                        self.assertEqual(payload, synthetic_static_elf())
        for name in ["init/", "../init", "init//x", "init/.", "init/..", "\x00init", "init\x00x"]:
            with self.subTest(path=repr(name)):
                with self.assertRaises(acceptance.AcceptanceError):
                    acceptance._init_from_newc(bundle(name=name))
        for marker in ["Run /init as init process", acceptance._UNAVAILABLE, acceptance._REFUSAL]:
            for changed in ["quoted: " + marker, marker + " extra", marker.lower(), marker.replace(" ", "\t")]:
                with self.subTest(marker=changed):
                    self.assertEqual(acceptance.observe_serial((changed + "\n").encode())["stage"], "UNKNOWN")

    def test_invalid_semantic_newc_fields_are_rejected(self):
        data = bundle()
        for field, value, code in [
            (1, b"00000000", "CPIO_ENTRY_TYPE"),
            (1, b"ffffffff", "CPIO_ENTRY_TYPE"),
            (4, b"00000000", "CPIO_LINK"),
            (11, b"00000000", "CPIO_HEADER"),
            (12, b"00000001", "CPIO_HEADER"),
        ]:
            with self.subTest(field=field, value=value):
                offset = 6 + field * 8
                mutated = data[:offset] + value + data[offset + 8:]
                self.assert_code(code, acceptance._init_from_newc, mutated)

    def test_kernel_attempt_is_not_pid1(self):
        result = acceptance.observe_serial(b"[    1.234] Run /init as init process\n")
        self.assertEqual(result["stage"], "INIT_EXEC_ATTEMPT_OBSERVED")
        self.assertFalse(result["actual_pid1_verified"])
        self.assertEqual(result["service_ready"], "UNKNOWN")

    def test_ordered_unavailable_remains_conditional(self):
        result = acceptance.observe_serial(candidate_serial().replace(b"\n", b"\r\n"))
        self.assertTrue(result["ordered_post_guard_candidate"])
        self.assertEqual(result["service_ready"], "NOT_READY_DIAGNOSTIC")
        self.assertFalse(result["actual_pid1_verified"])
        self.assertFalse(result["minimal_service_ready_verified"])

    def test_panic_does_not_erase_prior_stage(self):
        result = acceptance.observe_serial(candidate_serial(
            "Kernel panic - not syncing: Attempted to kill init!"))
        self.assertEqual(result["stage"], "POST_GUARD_UNAVAILABLE_TEXT_OBSERVED")
        self.assertTrue(result["ordered_post_guard_candidate"])
        self.assertEqual(result["failure"], "KERNEL_PANIC")
        self.assertFalse(result["actual_pid1_verified"])

    def test_unbound_or_reversed_diagnostic(self):
        for text in [acceptance._UNAVAILABLE + "\n", acceptance._UNAVAILABLE + "\nRun /init as init process\n"]:
            result = acceptance.observe_serial(text.encode())
            self.assertFalse(result["ordered_post_guard_candidate"])
            self.assertEqual(result["service_ready"], "UNKNOWN")

    def test_refusal_and_conflicting_runs(self):
        result = acceptance.observe_serial((acceptance._REFUSAL + "\n").encode())
        self.assertEqual(result["failure"], "NON_PID1_REFUSAL")
        for text in [acceptance._REFUSAL + "\n" + acceptance._UNAVAILABLE + "\n",
                     "Linux version first\nLinux version second\n",
                     "Failed to execute /init (error -8)\n" + acceptance._UNAVAILABLE + "\n"]:
            self.assertEqual(acceptance.observe_serial(text.encode())["stage"], "AMBIGUOUS_TRANSCRIPT")

    def test_serial_candidate_requires_one_ordered_kernel_init_path(self):
        good = acceptance.observe_serial(candidate_serial())
        self.assertTrue(good["ordered_post_guard_candidate"])
        self.assertEqual([item["event"] for item in good["observations"]],
                         ["KERNEL_BANNER", "INIT_EXEC_ATTEMPT",
                          "POST_GUARD_UNAVAILABLE_DIAGNOSTIC"])
        timestamped = ("[  0.000000] Linux version 6.18.63-nagi (x86_64)\n"
                       "[  0.100000] Run /init as init process\n"
                       "[  0.200000] " + acceptance._UNAVAILABLE + "\n")
        self.assertTrue(acceptance.observe_serial(timestamped.encode())[
            "ordered_post_guard_candidate"])

        for text in [
            "Run /init as init process\n" + acceptance._UNAVAILABLE +
            "\nLinux version 6.18.63-nagi (x86_64)\n",
            "Run /init as init process\n" + acceptance._UNAVAILABLE + "\n",
            "Linux version 6.18.63-nagi (x86_64)\nRun /init as init process\n" +
            acceptance._UNAVAILABLE + "\nRun /init as init process\n",
            "Linux version 6.18.63-nagi (x86_64)\nRun /init as init process\n" +
            "Run /init as init process\n" + acceptance._UNAVAILABLE + "\n",
            "Linux version 6.18.63-nagi (x86_64)\nRun /init as init process\n" +
            acceptance._UNAVAILABLE + "\n" + acceptance._UNAVAILABLE + "\n",
            "Kernel panic - not syncing: No working init found. Try passing init=\n" +
            "Linux version 6.18.63-nagi (x86_64)\nRun /init as init process\n" +
            acceptance._UNAVAILABLE + "\n",
            "Kernel panic - not syncing: Requested init /init failed (error -2).\n" +
            "Linux version 6.18.63-nagi (x86_64)\nRun /init as init process\n" +
            acceptance._UNAVAILABLE + "\n",
            "Linux version 6.18.63-nagi (x86_64)\nRun /init as init process\n" +
            "Failed to execute /init (error -8)\n" + acceptance._UNAVAILABLE + "\n",
            "Linux version 6.18.63-nagi (x86_64)\nRun /init as init process\n" +
            acceptance._UNAVAILABLE + "\nRun /sbin/init as init process\n",
            "Linux version 6.18.63-nagi (x86_64)\nRun /init as init process\n" +
            acceptance._UNAVAILABLE + "\nFailed to execute /sbin/init (error -2)\n",
            "[  2.000000] Linux version 6.18.63-nagi (x86_64)\n" +
            "[  3.000000] Run /init as init process\n" +
            "[  2.500000] " + acceptance._UNAVAILABLE + "\n",
        ]:
            with self.subTest(text=text):
                result = acceptance.observe_serial(text.encode())
                self.assertFalse(result["ordered_post_guard_candidate"])
                self.assertFalse(result["minimal_service_ready_verified"])

        for blocker in [
            "Kernel panic - not syncing: No working init found. Try passing init=",
            "Failed to execute /init (error -8)",
            "Kernel panic - not syncing: Requested init /init failed (error -2).",
            acceptance._REFUSAL,
        ]:
            with self.subTest(blocker=blocker, position="after-unavailable"):
                result = acceptance.observe_serial(candidate_serial(blocker))
                self.assertFalse(result["ordered_post_guard_candidate"])
                self.assertEqual(result["stage"], "AMBIGUOUS_TRANSCRIPT")
                self.assertEqual(result["failure"], "AMBIGUOUS_TRANSCRIPT")

    def test_conflicting_init_events_before_candidate_block_it(self):
        valid = candidate_serial().decode()
        valid_tail = valid.split("\n", 1)[1]
        kernel_banner = "Linux version 6.18.63-nagi (x86_64)\n"
        cases = {
            "other-init-before-candidate": (
                kernel_banner + "Run /sbin/init as init process\n" + valid_tail),
            "failed-rdinit-before-init": (
                kernel_banner + "Run /sbin/init as init process\n"
                "Failed to execute /sbin/init (error -2)\n" + valid_tail),
            "duplicate-banner-before-candidate": "Linux version earlier\n" + valid,
        }
        for name, text in cases.items():
            with self.subTest(name=name):
                result = acceptance.observe_serial(text.encode())
                self.assertEqual(result["stage"], "AMBIGUOUS_TRANSCRIPT")
                self.assertEqual(result["failure"], "AMBIGUOUS_TRANSCRIPT")
                self.assertFalse(result["ordered_post_guard_candidate"])
                self.assertFalse(result["minimal_service_ready_verified"])

    def test_other_init_fallback_is_not_nagi_init_attempt(self):
        result = acceptance.observe_serial(
            b"Linux version 6.18.63-nagi (x86_64)\nRun /sbin/init as init process\n")
        self.assertFalse(result["ordered_post_guard_candidate"])
        self.assertIn("OTHER_INIT_EXEC_ATTEMPT",
                      [item["event"] for item in result["observations"]])
        failed_fallback = acceptance.observe_serial(candidate_serial(
            "Failed to execute /sbin/init (error -2)"))
        self.assertFalse(failed_fallback["ordered_post_guard_candidate"])
        self.assertIn("OTHER_INIT_EXEC_FAILED",
                      [item["event"] for item in failed_fallback["observations"]])

    def test_panic_and_exec_failures(self):
        for text, code in [("Failed to execute /init (error -8)", "INIT_EXEC_FAILED"),
                           ("Kernel panic - not syncing: Attempted to kill init!", "KERNEL_PANIC"),
                           ("Kernel panic - not syncing: No working init found. Try passing init=", "NO_WORKING_INIT"),
                           ("Kernel panic - not syncing: Requested init /init failed (error -2).", "REQUESTED_INIT_FAILED")]:
            self.assertEqual(acceptance.observe_serial((text + "\n").encode())["failure"], code)

    def test_arbitrary_ready_partial_and_quoted_text_are_unknown(self):
        for text in [b"READY\nNagi login readiness reported PASS\n", b"Run /init as init process",
                     b"quoted Run /init as init process\n", b""]:
            result = acceptance.observe_serial(text)
            self.assertEqual(result["stage"], "UNKNOWN")
            self.assertFalse(result["minimal_service_ready_verified"])

    def test_serial_encoding_and_control_rejected(self):
        for data, code in [(b"\xff", "SERIAL_ENCODING"), (b"\x1b[32mREADY\n", "SERIAL_CONTROL"),
                           (b"x\ry\n", "SERIAL_LINE")]:
            self.assert_code(code, acceptance.observe_serial, data)

    def test_schema_hashes_and_authority(self):
        elf, cpio, serial = synthetic_static_elf(), prepared_bundle(), b"READY\n"
        inputs = {"schema": "nagi-linux-boot-evidence-input-v1", "evidence_kind": "synthetic_fixture",
                  **{k + "_sha256": hashlib.sha256(v).hexdigest() for k, v in
                     [("init", elf), ("initramfs", cpio), ("serial", serial)]}}
        def run(data):
            return acceptance.evaluate(data, init=elf, initramfs=cpio, serial=serial)
        # Function bodies must not touch filesystem, spawn or network.
        with patch("builtins.open", side_effect=AssertionError("file I/O")), \
             patch.object(prepare.os, "open", side_effect=AssertionError("os I/O")), \
             patch.object(prepare.os, "stat", side_effect=AssertionError("stat")):
            result = run(inputs)
        self.assertEqual(result["guest_acceptance"], "NOT_VERIFIED")
        self.assertFalse(result["run_binding_verified"])
        for changed, code in [({**inputs, "trusted": True}, "INPUT_SCHEMA"),
                              ({**inputs, "evidence_kind": "real_verified"}, "INPUT_SCHEMA"),
                              ({**inputs, "init_sha256": "0" * 64}, "INPUT_HASH_MISMATCH"),
                              ({**inputs, "serial_sha256": "bad"}, "INPUT_HASH_FORMAT")]:
            self.assert_code(code, run, changed)

    def test_existing_prepare_regressions_in_memory(self):
        result = existing_prepare_in_memory()
        self.assertGreater(result.testsRun, 0)
        self.assertEqual(result.errors, [])
        self.assertEqual(result.failures, [])

    def test_receipt_success_failure_and_truncation(self):
        elf, cpio, serial = synthetic_static_elf(), prepared_bundle(), b"READY\n"
        inputs = {"schema": "nagi-linux-boot-evidence-input-v1", "evidence_kind": "unverified_capture",
                  **{k + "_sha256": hashlib.sha256(v).hexdigest() for k, v in
                     [("init", elf), ("initramfs", cpio), ("serial", serial)]}}
        good = acceptance.verification_receipt(inputs, init=elf, initramfs=cpio, serial=serial)
        self.assertEqual(good["returncode"], 0)
        self.assertEqual(good["status"], "PASS")
        self.assertEqual(good["format_version"], 3)
        self.assertEqual(good["receipt_role"], "OFFLINE_VALIDATION_ONLY")
        self.assertEqual(good["kernel_built"], "NOT_VERIFIED")
        self.assertEqual(good["QEMU_started"], "NOT_VERIFIED")
        self.assertEqual(acceptance.verify_offline_receipt(good, init=elf, initramfs=cpio, serial=serial),
                         {"receipt_valid_offline_only": True, "status": "PASS",
                          "guest_acceptance": "NOT_VERIFIED", "run_binding_verified": False})
        self.assert_code("OFFLINE_RECEIPT_NOT_RUNTIME_EVIDENCE",
                         acceptance.require_runtime_acceptance,
                         good, init=elf, initramfs=cpio, serial=serial)
        for name, data in [("init", elf), ("initramfs", cpio), ("serial", serial)]:
            for end in range(len(data)):
                byte_inputs = {"init": elf, "initramfs": cpio, "serial": serial}
                byte_inputs[name] = data[:end]
                failed = acceptance.verification_receipt(inputs, **byte_inputs)
                self.assertEqual(failed["returncode"], 2)
                self.assertEqual(failed["result"]["error_code"], "INPUT_HASH_MISMATCH")
                self.assertEqual(failed["guest_acceptance"], "NOT_VERIFIED")
                self.assertEqual(failed["checks_completed"], [])
                self.assertEqual(failed["caller_hash_references"][name + "_sha256"], inputs[name + "_sha256"])
                self.assertEqual(failed, json.loads(json.dumps(failed)))
        truncated_serial = b"READY\n"
        serial_only = acceptance.verification_receipt(
            {"schema": inputs["schema"], "evidence_kind": "synthetic_fixture",
             "init_sha256": hashlib.sha256(elf).hexdigest(),
             "initramfs_sha256": hashlib.sha256(cpio).hexdigest(),
             "serial_sha256": hashlib.sha256(truncated_serial).hexdigest()},
            init=elf, initramfs=cpio, serial=truncated_serial)
        self.assertEqual(serial_only["status"], "PASS")
        self.assertEqual(serial_only["result"]["serial"]["stage"], "UNKNOWN")
        self.assert_code("OFFLINE_RECEIPT_NOT_RUNTIME_EVIDENCE",
                         acceptance.require_runtime_acceptance,
                         serial_only, init=elf, initramfs=cpio, serial=truncated_serial)
        bad = acceptance.verification_receipt(inputs, init=elf, initramfs=cpio, serial=None)
        self.assertEqual(bad["result"]["error_code"], "INPUT_BYTES")
        self.assertIsNone(bad["inputs"]["serial"]["sha256"])
        # Altered, missing, stale-mixed, or runtime-relabelled records cannot
        # pass the offline recomputation or the future-runtime consumer boundary.
        for changed in [dict(good, status="FAIL"), dict(good, guest_acceptance="READY"),
                        {k: v for k, v in good.items() if k != "checks_completed"},
                        dict(good, result=dict(good["result"], guest_acceptance="READY")),
                        dict(good, format_version=2),
                        dict(good, receipt_role="RUNTIME_READY"),
                        dict(good, format="nagi-linux-runtime-ready-v1")]:
            expected_code = ("RUNTIME_RECEIPT_SCHEMA_UNDEFINED"
                             if changed.get("format") != good["format"] else
                             "OFFLINE_RECEIPT_SCHEMA" if changed.get("format_version") != good["format_version"]
                             else
                             "OFFLINE_RECEIPT_SCHEMA" if changed.get("receipt_role") != good["receipt_role"]
                             else "OFFLINE_RECEIPT_INTEGRITY")
            self.assert_code(expected_code, acceptance.require_runtime_acceptance,
                             changed, init=elf, initramfs=cpio, serial=serial)
        mixed_init = synthetic_static_elf(dynamic=True)
        self.assert_code("OFFLINE_RECEIPT_INTEGRITY", acceptance.verify_offline_receipt,
                         good, init=mixed_init, initramfs=cpio, serial=serial)
        for mixed in [(elf[:-1], cpio, serial), (elf, cpio + bytes(4), serial),
                      (elf, cpio, serial + b"\n")]:
            self.assert_code("OFFLINE_RECEIPT_INTEGRITY", acceptance.verify_offline_receipt,
                             good, init=mixed[0], initramfs=mixed[1], serial=mixed[2])
        self.assert_code("RUNTIME_RECEIPT_SCHEMA_UNDEFINED",
                         acceptance.require_runtime_acceptance, {}, init=elf, initramfs=cpio, serial=serial)
        self.assert_code("OFFLINE_VALIDATION_FAILED", acceptance.require_runtime_acceptance,
                         bad, init=elf, initramfs=cpio, serial=None)
        self.assertEqual(good["guest_acceptance"], "NOT_VERIFIED")
        self.assertFalse(good["minimal_service_ready_verified"])
        for invalid in [None, [], {}, {**inputs, "schema": []}, {**inputs, "evidence_kind": {}},
                        {**inputs, "init_sha256": None}]:
            rejected = acceptance.verification_receipt(invalid, init=elf, initramfs=cpio, serial=serial)
            self.assertEqual(rejected["status"], "FAIL")
            self.assertEqual(rejected["returncode"], 2)
            self.assertEqual(rejected, json.loads(json.dumps(rejected)))


if __name__ == "__main__":
    unittest.main()
