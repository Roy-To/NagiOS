#!/usr/bin/env python3
"""Build a fixed x86-64 Linux newc initramfs for a development guest smoke."""

import argparse
import hashlib
import os
import stat
import sys
import tempfile

MAGIC, TRAILER = b"070701", "TRAILER!!!"
ET_EXEC, ET_DYN, EM_X86_64 = 2, 3, 62
PT_LOAD, PT_DYNAMIC, PT_INTERP = 1, 2, 3
REQUIRED_SMOKE_NAMES = (
    "bin/linux_model_clock_consumer",
    "bin/notes-host-preview",
    "bin/in_process_echo",
    "bin/sdk-development-smoke",
    "bin/installed_service_clock_smoke",
    "bin/notes-event-bridge-smoke",
)
OPTIONAL_SMOKE_NAMES = (
    "bin/desktop-host-preview-smoke",
    "bin/desktop-evdev-stream-smoke",
    "bin/desktop-evdev-integrity-smoke",
)


class BuildError(ValueError):
    pass


def read_regular(path, executable=False):
    path = os.path.abspath(path)
    parts, current = [p for p in path.split(os.sep) if p], os.sep
    info = None
    for i, part in enumerate(parts):
        current = os.path.join(current, part)
        info = os.lstat(current)
        if stat.S_ISLNK(info.st_mode):
            raise BuildError(f"symlink path component: {current}")
        if i < len(parts) - 1 and not stat.S_ISDIR(info.st_mode):
            raise BuildError(f"input parent is not a directory: {current}")
    if info is None or not stat.S_ISREG(info.st_mode):
        raise BuildError(f"input is not a regular file: {path}")
    if executable and not stat.S_IMODE(info.st_mode) & 0o111:
        raise BuildError(f"input has no execute permission bits: {path}")
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    try:
        before = os.fstat(fd)
        if (before.st_dev, before.st_ino) != (info.st_dev, info.st_ino):
            raise BuildError(f"input changed while opening: {path}")
        if executable and not stat.S_IMODE(before.st_mode) & 0o111:
            raise BuildError(f"input lost execute permission before opening: {path}")
        chunks = []
        while True:
            block = os.read(fd, 1024 * 1024)
            if not block:
                break
            chunks.append(block)
        after = os.fstat(fd)
        if (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
            after.st_size, after.st_mtime_ns, after.st_ctime_ns
        ):
            raise BuildError(f"input changed while reading: {path}")
        data = b"".join(chunks)
        if len(data) != before.st_size:
            raise BuildError(f"input size mismatch: {path}")
        return data
    finally:
        os.close(fd)


def validate_elf(data, label, expected_type):
    if len(data) < 64 or data[:4] != b"\x7fELF" or data[4:7] != b"\x02\x01\x01":
        raise BuildError(f"{label}: expected ELF64 little-endian")
    etype, machine = int.from_bytes(data[16:18], "little"), int.from_bytes(data[18:20], "little")
    entry, phoff = int.from_bytes(data[24:32], "little"), int.from_bytes(data[32:40], "little")
    ehsize, entsize, count = (int.from_bytes(data[x:x + 2], "little") for x in (52, 54, 56))
    if (etype, machine, ehsize, entsize) != (expected_type, EM_X86_64, 64, 56) or not count:
        raise BuildError(f"{label}: wrong ELF type/machine or missing program headers")
    if phoff < ehsize or phoff + entsize * count > len(data):
        raise BuildError(f"{label}: invalid program-header table")
    executable_entry = dynamic = needed = False
    for i in range(count):
        p = data[phoff + i * entsize:phoff + (i + 1) * entsize]
        kind, flags = int.from_bytes(p[:4], "little"), int.from_bytes(p[4:8], "little")
        offset, vaddr = int.from_bytes(p[8:16], "little"), int.from_bytes(p[16:24], "little")
        filesz, memsz = int.from_bytes(p[32:40], "little"), int.from_bytes(p[40:48], "little")
        if offset + filesz > len(data):
            raise BuildError(f"{label}: program header exceeds file")
        if kind == PT_INTERP:
            raise BuildError(f"{label}: dynamic interpreter required")
        if kind == PT_LOAD and flags & 1 and vaddr <= entry < vaddr + memsz:
            executable_entry = True
        if kind == PT_DYNAMIC:
            dynamic = True
            if filesz % 16:
                raise BuildError(f"{label}: truncated PT_DYNAMIC")
            tags = [int.from_bytes(data[o:o + 8], "little", signed=True)
                    for o in range(offset, offset + filesz, 16)]
            if 0 not in tags:
                raise BuildError(f"{label}: unterminated PT_DYNAMIC")
            needed |= 1 in tags
    if not executable_entry or needed:
        raise BuildError(f"{label}: invalid entry point or shared-library dependency")
    if expected_type == ET_DYN and not dynamic:
        raise BuildError(f"{label}: expected static-PIE relocation segment")


def safe_name(name):
    if not name or name.startswith("/") or "\\" in name or any(p in ("", ".", "..") for p in name.split("/")):
        raise BuildError(f"unsafe CPIO path: {name!r}")
    try:
        return name.encode("ascii") + b"\0"
    except UnicodeEncodeError:
        raise BuildError(f"non-ASCII CPIO path: {name!r}") from None


def optional_smoke(name, path):
    safe_name(name)
    try:
        data = read_regular(path, executable=True)
    except FileNotFoundError:
        return None
    validate_elf(data, path, ET_EXEC)
    return name, path, data, hashlib.sha256(data).hexdigest()


def optional_smokes(specifications):
    entries = []
    for name, path in specifications:
        entry = optional_smoke(name, path)
        if entry is not None:
            entries.append(entry)
    return entries


def parse_named_sources(items, allowed_names, label):
    sources = {}
    for item in items:
        name, separator, path = item.partition("=")
        if not separator or not name or not path:
            raise BuildError(f"{label} must use GUEST_PATH=/absolute/host/path")
        if name not in allowed_names:
            raise BuildError(f"{label} has an unsupported guest path: {name}")
        if name in sources:
            raise BuildError(f"{label} repeats guest path: {name}")
        if not os.path.isabs(path) or os.path.normpath(path) != path:
            raise BuildError(f"{label} host path must be absolute and normalized: {name}")
        sources[name] = path
    return sources


def require_absolute_normalized_path(path, label):
    if not path or not os.path.isabs(path) or os.path.normpath(path) != path:
        raise BuildError(f"{label} must be an absolute normalized path")
    return path


def emit(name, mode, data, inode, offset, nlink=1, rdev=(0, 0)):
    encoded = safe_name(name)
    fields = (inode, mode, 0, 0, nlink, 0, len(data), 0, 0, *rdev, len(encoded), 0)
    if any(x < 0 or x > 0xffffffff for x in fields) or len(data) > 0xffffffff:
        raise BuildError(f"newc field too large: {name}")
    chunk = bytearray(MAGIC + b"".join(f"{x:08x}".encode() for x in fields) + encoded)
    chunk.extend(b"\0" * ((-(offset + len(chunk))) & 3))
    chunk.extend(data)
    chunk.extend(b"\0" * ((-(offset + len(chunk))) & 3))
    return bytes(chunk)


def make_archive(entries):
    out, names = bytearray(), set()
    for inode, (name, mode, data, nlink, rdev) in enumerate(entries, 1):
        safe_name(name)
        if name == TRAILER or name in names:
            raise BuildError(f"duplicate/reserved path: {name}")
        names.add(name)
        out.extend(emit(name, mode, data, inode, len(out), nlink, rdev))
    out.extend(emit(TRAILER, 0, b"", len(entries) + 1, len(out)))
    return bytes(out)


def parse_newc(data):
    entries, offset = [], 0
    while True:
        if offset % 4 or offset + 110 > len(data) or data[offset:offset + 6] != MAGIC:
            raise BuildError(f"invalid newc header at {offset}")
        f = [int(data[offset + 6 + i * 8:offset + 14 + i * 8], 16) for i in range(13)]
        name_size, file_size = f[11], f[6]
        start, end = offset + 110, offset + 110 + name_size
        if name_size < 1 or end > len(data) or data[end - 1:end] != b"\0":
            raise BuildError("invalid newc filename")
        name = data[start:end - 1].decode("ascii")
        dstart, dend = (end + 3) & ~3, ((end + 3) & ~3) + file_size
        following = (dend + 3) & ~3
        if following > len(data) or any(data[end:dstart]) or any(data[dend:following]):
            raise BuildError(f"truncated or nonzero-padded entry: {name}")
        entries.append({"name": name, "mode": f[1], "data": data[dstart:dend], "offset": offset,
                        "data_offset": dstart, "next": following, "device": (f[9], f[10])})
        if name == TRAILER:
            if f[1] or file_size or following != len(data):
                raise BuildError("TRAILER!!! must be the final empty record")
            return entries
        offset = following


def verify(data, expected):
    parsed = parse_newc(data)
    if [e["name"] for e in parsed] != [e[0] for e in expected] + [TRAILER]:
        raise BuildError("archive path list mismatch")
    for actual, wanted in zip(parsed[:-1], expected, strict=True):
        name, mode, contents, _, rdev = wanted
        if actual["mode"] != mode or actual["data"] != contents or actual["device"] != rdev:
            raise BuildError(f"archive content/mode mismatch: {name}")
        if actual["offset"] % 4 or actual["data_offset"] % 4 or actual["next"] % 4:
            raise BuildError(f"archive alignment mismatch: {name}")
    if len(data) % 4:
        raise BuildError("archive EOF is not four-byte aligned")
    return parsed


def validate_output_path(workspace, output):
    workspace, output = os.path.abspath(workspace), os.path.abspath(output)
    current = os.sep
    parts = [part for part in workspace.split(os.sep) if part]
    for part in parts:
        current = os.path.join(current, part)
        info = os.lstat(current)
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            raise BuildError(f"workspace ancestor is not a real directory: {current}")
    if output == workspace or os.path.commonpath((workspace, output)) != workspace:
        raise BuildError("output must be a new child inside the fixed task workspace")
    relative = os.path.relpath(output, workspace)
    current = workspace
    children = relative.split(os.sep)
    for index, part in enumerate(children):
        current = os.path.join(current, part)
        try:
            info = os.lstat(current)
        except FileNotFoundError:
            if index == len(children) - 1:
                return output
            raise BuildError(f"output parent does not exist: {current}") from None
        if stat.S_ISLNK(info.st_mode):
            raise BuildError(f"output path contains a symlink ancestor: {current}")
        if index < len(children) - 1 and not stat.S_ISDIR(info.st_mode):
            raise BuildError(f"output parent is not a directory: {current}")
        if index == len(children) - 1:
            raise BuildError(f"output already exists: {current}")
    raise BuildError("invalid output path")


def self_test():
    sample = [("d", stat.S_IFDIR | 0o755, b"", 2, (0, 0)),
              ("d/x", stat.S_IFREG | 0o755, b"a", 1, (0, 0)),
              ("dev/null", stat.S_IFCHR | 0o666, b"", 1, (1, 3))]
    parsed = verify(make_archive(sample), sample)
    assert parsed[0]["mode"] == stat.S_IFDIR | 0o755
    assert parsed[2]["mode"] == stat.S_IFCHR | 0o666 and parsed[2]["device"] == (1, 3)
    assert parsed[-1]["name"] == TRAILER and parsed[-1]["next"] == len(make_archive(sample))
    for path in ("/init", "../init", "bin/../init", "bin//init", ""):
        try:
            safe_name(path)
        except BuildError:
            continue
        raise BuildError(f"self-test accepted unsafe path {path!r}")
    with tempfile.TemporaryDirectory(
        prefix=".path-check-", dir=os.path.realpath(tempfile.gettempdir())
    ) as fixture:
        fixture = os.path.abspath(fixture)
        workspace = os.path.join(fixture, "workspace")
        os.mkdir(workspace)
        real = os.path.join(workspace, "real")
        os.mkdir(real)
        os.mkdir(os.path.join(real, "deep"))
        link = os.path.join(workspace, "alias")
        os.symlink(real, link)

        def legacy_accepts(cwd, output):
            parent = os.lstat(os.path.dirname(output))
            return (os.path.commonpath((cwd, output)) == cwd and not os.path.lexists(output)
                    and stat.S_ISDIR(parent.st_mode) and not stat.S_ISLNK(parent.st_mode))

        symlink_output = os.path.join(link, "deep", "not-created")
        assert legacy_accepts(workspace, symlink_output), "symlink-ancestor fixture did not reproduce old check"
        try:
            validate_output_path(workspace, symlink_output)
        except BuildError as error:
            assert "symlink ancestor" in str(error)
        else:
            raise BuildError("symlink-ancestor regression was accepted")

        good_output = os.path.join(real, "new-child")
        assert validate_output_path(workspace, good_output) == good_output
        original_cwd = os.getcwd()
        try:
            os.chdir(fixture)
            escaped_output = os.path.join(os.getcwd(), "not-created")
            assert legacy_accepts(os.getcwd(), escaped_output), "cwd-expansion fixture did not reproduce old check"
            try:
                validate_output_path(workspace, escaped_output)
            except BuildError as error:
                assert "fixed task workspace" in str(error)
            else:
                raise BuildError("output outside the fixed workspace was accepted")
        finally:
            os.chdir(original_cwd)

        assert optional_smokes((("bin/missing-smoke", os.path.join(fixture, "absent")),)) == []
        invalid = os.path.join(fixture, "invalid-elf")
        with open(invalid, "wb") as stream:
            stream.write(b"not an ELF")
        os.chmod(invalid, 0o755)
        try:
            optional_smoke("bin/invalid-smoke", invalid)
        except BuildError as error:
            assert "ELF64" in str(error)
        else:
            raise BuildError("invalid optional ELF was accepted")
        noexec = os.path.join(fixture, "non-executable")
        with open(noexec, "wb") as stream:
            stream.write(b"not an ELF")
        os.chmod(noexec, 0o644)
        try:
            optional_smoke("bin/nonexec-smoke", noexec)
        except BuildError as error:
            assert "execute permission" in str(error)
        else:
            raise BuildError("optional input without execute permission was accepted")

        mode_race = os.path.join(fixture, "mode-race")
        with open(mode_race, "wb") as stream:
            stream.write(b"fixture bytes")
        os.chmod(mode_race, 0o755)
        real_open = os.open

        def clear_execute_before_open(path, flags, *args, **kwargs):
            if os.fspath(path) == mode_race:
                os.chmod(mode_race, 0o644)
            return real_open(path, flags, *args, **kwargs)

        os.open = clear_execute_before_open
        try:
            try:
                read_regular(mode_race, executable=True)
            except BuildError as error:
                assert "lost execute permission" in str(error)
            else:
                raise BuildError("execute-bit removal between lstat and fstat was accepted")
        finally:
            os.open = real_open


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--init", help="absolute path to the static-PIE init ELF")
    parser.add_argument("--workspace", help="absolute task workspace used to contain output")
    parser.add_argument("--smoke", action="append", default=[], metavar="GUEST_PATH=HOST_PATH",
                        help="required guest ELF source; repeat once for each required path")
    parser.add_argument("--optional-smoke", action="append", default=[], metavar="GUEST_PATH=HOST_PATH",
                        help="optional guest ELF source; repeat for available optional paths")
    parser.add_argument("--output-dir", help="new directory under --workspace")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    try:
        self_test()
        if args.self_test:
            print("PASS self-test: CPIO layout; optional missing/invalid/mode cases; post-open execute-bit change; output boundaries")
            return 0
        if not args.output_dir or not args.workspace or not args.init:
            parser.error("--init, --workspace, and --output-dir are required unless --self-test is used")
        init_path = require_absolute_normalized_path(args.init, "--init")
        workspace_path = require_absolute_normalized_path(args.workspace, "--workspace")
        output_path = require_absolute_normalized_path(args.output_dir, "--output-dir")
        required_sources = parse_named_sources(args.smoke, REQUIRED_SMOKE_NAMES, "--smoke")
        if set(required_sources) != set(REQUIRED_SMOKE_NAMES):
            missing = sorted(set(REQUIRED_SMOKE_NAMES) - set(required_sources))
            raise BuildError("--smoke must name every required guest path exactly once: " + ", ".join(missing))
        optional_sources = parse_named_sources(args.optional_smoke, OPTIONAL_SMOKE_NAMES, "--optional-smoke")
        sources = [("init", init_path, ET_DYN)] + [
            (name, required_sources[name], ET_EXEC) for name in REQUIRED_SMOKE_NAMES
        ]
        payloads = []
        for name, path, elf_type in sources:
            data = read_regular(path)
            validate_elf(data, name, elf_type)
            payloads.append((name, path, data, hashlib.sha256(data).hexdigest()))
        payloads.extend(optional_smokes(
            (name, optional_sources[name]) for name in OPTIONAL_SMOKE_NAMES if name in optional_sources
        ))
        entries = [("bin", stat.S_IFDIR | 0o755, b"", 2, (0, 0)),
                   ("dev", stat.S_IFDIR | 0o755, b"", 2, (0, 0)),
                   ("dev/console", stat.S_IFCHR | 0o600, b"", 1, (5, 1)),
                   ("dev/null", stat.S_IFCHR | 0o666, b"", 1, (1, 3)),
                   ("dev/ttyS0", stat.S_IFCHR | 0o600, b"", 1, (4, 64)),
                   ("proc", stat.S_IFDIR | 0o555, b"", 2, (0, 0)),
                   ("sys", stat.S_IFDIR | 0o555, b"", 2, (0, 0)),
                   ("run", stat.S_IFDIR | 0o755, b"", 2, (0, 0)),
                   ("tmp", stat.S_IFDIR | 0o1777, b"", 2, (0, 0)),
                   ("init", stat.S_IFREG | 0o755, payloads[0][2], 1, (0, 0))]
        entries += [(name, stat.S_IFREG | 0o755, data, 1, (0, 0))
                    for name, _, data, _ in payloads[1:]]
        archive = make_archive(entries)
        parsed = verify(archive, entries)
        elf_types = [("init", ET_DYN)] + [(name, ET_EXEC) for name, _, _, _ in payloads[1:]]
        for name, elf_type in elf_types:
            validate_elf(next(e["data"] for e in parsed if e["name"] == name), name, elf_type)

        output = validate_output_path(workspace_path, output_path)
        os.mkdir(output, 0o755)
        dest = os.path.join(output, "initramfs.cpio")
        try:
            fd = os.open(dest, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o644)
            try:
                with os.fdopen(fd, "wb", closefd=False) as stream:
                    stream.write(archive)
                    stream.flush()
                    os.fsync(fd)
            finally:
                os.close(fd)
        except Exception:
            try:
                os.unlink(dest)
            except FileNotFoundError:
                pass
            os.rmdir(output)
            raise
        on_disk = open(dest, "rb").read()
        verify(on_disk, entries)
        print(f"initramfs=initramfs.cpio bytes={len(on_disk)} sha256={hashlib.sha256(on_disk).hexdigest()}")
        for name, _path, data, digest in payloads:
            print(f"source_sha256={digest} bytes={len(data)} guest_path={name}")
        print(f"PASS archive read-back: /init plus {len(payloads) - 1} /bin static ELFs; modes/alignment/trailer verified; not executed")
        return 0
    except (BuildError, OSError) as error:
        print(f"FAIL initramfs: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
