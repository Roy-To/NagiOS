#!/usr/bin/env python3
"""Run the ARM/HVF Linux cross-build and classify an x86 Linux boot."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import platform
import re
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
KERNEL_DIR = ROOT / "tools/linux-boot/kernel-dev-20261115"
DEFAULT_VM = KERNEL_DIR / "arm64-linux-cross-build-vm-20261115-r1"
KERNEL_VERSION = "6.18.63"
KERNEL_ARCHIVE_SHA256 = "81764a545072d160748beff4d9de2e7d95aa9f16e8261110288f067a2cc61d71"
KERNEL_SIGNATURE_SHA256 = "7104a9bf3c739b0fda00e70bb94ff262582c5d4304a96987d0bb2b78c0a33c66"
KERNEL_SIGNING_KEY_SHA256 = "61b758f0d4a386fc83565b92cfd6c05879573df0e96d44b71a5a3e19e1a8581a"
MINIMUM_CONFIG_SHA256 = "4f95b7f563bede591e4e0604218cc95db6045195e67a273c8d4e1d949c24de7d"
UBUNTU_IMAGE_SHA256 = "50e24e1fb79e89116e939d90aa7738f7af48c442fa1bc2510131e0b39c50348e"
FIRMWARE_DEFAULT = Path("/opt/homebrew/share/qemu/edk2-aarch64-code.fd")
SERIAL_LIMIT = 4 * 1024 * 1024


class ReproError(Exception):
    """A closed preflight, build, or boot-check failure."""


class ChildDeadline(Exception):
    """An owned child did not finish before its deadline."""


def _relative(path: Path) -> str:
    try:
        return path.relative_to(ROOT).as_posix()
    except ValueError:
        return path.name


def _path_in_worktree(value: str | Path, *, allow_missing_leaf: bool) -> Path:
    candidate = Path(value)
    if not candidate.is_absolute():
        candidate = ROOT / candidate
    candidate = Path(os.path.abspath(candidate))
    try:
        relative = candidate.relative_to(ROOT)
    except ValueError:
        raise ReproError("write/input path is outside the assigned worktree") from None
    current = ROOT
    parts = relative.parts
    if not parts:
        raise ReproError("worktree root is not a file path")
    for index, part in enumerate(parts):
        current = current / part
        try:
            info = os.lstat(current)
        except FileNotFoundError:
            if allow_missing_leaf and index == len(parts) - 1:
                return current
            raise ReproError(f"required path is missing: {_relative(current)}") from None
        if stat.S_ISLNK(info.st_mode):
            raise ReproError(f"symlink path is not accepted: {_relative(current)}")
        if index < len(parts) - 1 and not stat.S_ISDIR(info.st_mode):
            raise ReproError(f"path parent is not a directory: {_relative(current)}")
    return current


def _regular(path: Path, label: str, *, private: bool = False) -> Path:
    info = os.lstat(path)
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise ReproError(f"{label} must be a regular file: {_relative(path)}")
    if private and stat.S_IMODE(info.st_mode) & 0o077:
        raise ReproError(f"{label} permissions must exclude group/other access")
    return path


def _directory(path: Path, label: str) -> Path:
    info = os.lstat(path)
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
        raise ReproError(f"{label} must be a real directory: {_relative(path)}")
    return path


def _digest(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _pin(path: Path, label: str, expected: str) -> None:
    if _digest(path) != expected:
        raise ReproError(f"{label} checksum does not match the pinned input")


def _tool(name: str) -> str:
    result = shutil.which(name)
    if result is None:
        raise ReproError(f"required host tool is unavailable: {name}")
    return result


def _capture(argv: list[str], *, timeout: int = 10) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, text=True, timeout=timeout, check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ReproError(f"host command failed: {Path(argv[0]).name}") from error


def _new_log(path: Path) -> int:
    _directory(path.parent, "serial log directory")
    try:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        raise ReproError(f"serial log already exists: {_relative(path)}") from None
    return descriptor


def _fresh_name(directory: Path, stem: str) -> Path:
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S")
    return directory / f"{stem}-{stamp}-{time.time_ns() % 1_000_000_000:09d}.serial.log"


def _spawn(argv: list[str], *, stdout: int | None = None,
           stderr: int | None = None) -> subprocess.Popen[bytes]:
    try:
        return subprocess.Popen(
            argv, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr,
            start_new_session=True, close_fds=True,
        )
    except OSError as error:
        raise ReproError(f"could not start owned child: {Path(argv[0]).name}") from error


def _terminate_owned(child: subprocess.Popen[bytes], *, grace: float = 3.0) -> int:
    if child.poll() is not None:
        return int(child.returncode)
    try:
        os.killpg(child.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        return int(child.wait(timeout=grace))
    except subprocess.TimeoutExpired:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        return int(child.wait())


def _wait_owned(child: subprocess.Popen[bytes], deadline: float) -> int:
    while True:
        result = child.poll()
        if result is not None:
            return int(result)
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            _terminate_owned(child)
            raise ChildDeadline
        try:
            return int(child.wait(timeout=min(remaining, 0.2)))
        except subprocess.TimeoutExpired:
            continue


def _timeout(value: str | int) -> int:
    try:
        seconds = int(value)
    except (TypeError, ValueError):
        raise argparse.ArgumentTypeError("timeout must be an integer") from None
    if seconds < 1 or seconds > 600:
        raise argparse.ArgumentTypeError("timeout must be between 1 and 600 seconds")
    return seconds


def _vcpus(value: str | int) -> int:
    try:
        count = int(value)
    except (TypeError, ValueError):
        raise argparse.ArgumentTypeError("vCPU count must be an integer") from None
    if count < 1 or count > 4:
        raise argparse.ArgumentTypeError("vCPU count must be between 1 and 4")
    return count


def _build_inputs(vm_value: str, output_value: str, firmware_value: str) -> dict:
    if platform.system() != "Darwin" or platform.machine().lower() not in ("arm64", "aarch64"):
        raise ReproError("ARM/HVF build entry requires an Apple Silicon macOS host")
    vm = _directory(_path_in_worktree(vm_value, allow_missing_leaf=False), "VM directory")
    output = _path_in_worktree(output_value, allow_missing_leaf=True)
    config_output = output.with_name(output.name + ".config")
    if output.exists() or os.path.lexists(output):
        raise ReproError(f"refusing to overwrite artifact: {_relative(output)}")
    if config_output.exists() or os.path.lexists(config_output):
        raise ReproError(f"refusing to overwrite artifact: {_relative(config_output)}")
    _directory(output.parent, "artifact output directory")

    inputs = _directory(vm / "inputs", "VM inputs directory")
    disk = _directory(vm / "disk", "VM disk directory")
    seed = _directory(vm / "seed", "VM seed directory")
    logs = _directory(vm / "logs", "VM log directory")

    image = _regular(inputs / "ubuntu-24.04-server-cloudimg-arm64.img", "Ubuntu ARM image")
    _pin(image, "Ubuntu ARM image", UBUNTU_IMAGE_SHA256)
    overlay = _regular(disk / "ubuntu-arm64-build-v2.qcow2", "reusable VM overlay")
    cloud_seed = _regular(seed / "nocloud-seed.iso", "NoCloud seed")
    ssh_key = _regular(seed / "id_ed25519", "VM SSH key", private=True)
    config = _regular(ROOT / "tools/linux-boot/kernel-minimum.config", "kernel minimum config")
    _pin(config, "kernel minimum config", MINIMUM_CONFIG_SHA256)
    archive = _regular(KERNEL_DIR / "inputs/linux-6.18.63.tar.xz", "signed kernel archive")
    _pin(archive, "signed kernel archive", KERNEL_ARCHIVE_SHA256)
    signature = _regular(KERNEL_DIR / "inputs/linux-6.18.63.tar.sign", "kernel signature")
    _pin(signature, "kernel signature", KERNEL_SIGNATURE_SHA256)
    signing_key = _regular(KERNEL_DIR / "inputs/gregkh-wkd-key", "kernel signing key")
    _pin(signing_key, "kernel signing key", KERNEL_SIGNING_KEY_SHA256)

    firmware = Path(firmware_value)
    if not firmware.is_absolute():
        firmware = ROOT / firmware
    firmware = firmware.resolve(strict=True)
    _regular(firmware, "AArch64 UEFI firmware")
    qemu_arm = _tool("qemu-system-aarch64")
    qemu_x86 = _tool("qemu-system-x86_64")
    qemu_img = _tool("qemu-img")
    ssh = _tool("ssh")
    info = _capture([qemu_img, "info", "--output=json", str(overlay)])
    if info.returncode != 0:
        raise ReproError("qemu-img could not read the reusable VM overlay")
    try:
        disk_info = json.loads(info.stdout)
    except json.JSONDecodeError:
        raise ReproError("qemu-img returned invalid overlay metadata") from None
    if (disk_info.get("format") != "qcow2"
            or Path(str(disk_info.get("backing-filename", ""))).name != image.name):
        raise ReproError("VM overlay does not reference the pinned Ubuntu ARM image")
    accel = _capture([qemu_arm, "-accel", "help"])
    if accel.returncode != 0 or not re.search(r"\bhvf\b", accel.stdout):
        raise ReproError("qemu-system-aarch64 does not advertise HVF")
    log = _fresh_name(logs, "arm64-linux-cross-build")
    expected_log = _path_in_worktree(log, allow_missing_leaf=True)
    if os.path.lexists(expected_log):
        raise ReproError(f"serial log already exists: {_relative(expected_log)}")
    return {
        "vm": vm, "image": image, "overlay": overlay, "cloud_seed": cloud_seed,
        "ssh_key": ssh_key, "config": config, "logs": logs, "firmware": firmware,
        "qemu_arm": qemu_arm, "qemu_x86": qemu_x86, "ssh": ssh,
        "log": expected_log, "output": output, "config_output": config_output,
    }


def _free_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def _ssh_argv(inputs: dict, port: int) -> list[str]:
    known_hosts = inputs["logs"] / "arm64-cross-build-known-hosts"
    return [
        inputs["ssh"], "-i", str(inputs["ssh_key"]), "-p", str(port),
        "-o", "IdentitiesOnly=yes", "-o", "BatchMode=yes",
        "-o", "ConnectTimeout=5", "-o", "StrictHostKeyChecking=accept-new",
        "-o", f"UserKnownHostsFile={known_hosts}",
        "builder@127.0.0.1",
    ]


def _wait_guest(inputs: dict, child: subprocess.Popen[bytes], port: int,
                deadline: float) -> None:
    probe = b"set -eu\n[ \"$(uname -m)\" = aarch64 ]\n[ \"$(gcc -dumpmachine)\" = aarch64-linux-gnu ]\n[ \"$(x86_64-linux-gnu-gcc -dumpmachine)\" = x86_64-linux-gnu ]\nprintf 'guest=arm64\\ncross=x86_64-linux-gnu\\n'\n"
    while time.monotonic() < deadline:
        if child.poll() is not None:
            raise ReproError(f"ARM/HVF QEMU exited before guest SSH (rc={child.returncode})")
        remaining = max(1, min(8, int(deadline - time.monotonic())))
        try:
            result = subprocess.run(
                [*_ssh_argv(inputs, port), "bash", "-s"], input=probe,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=remaining,
                check=False,
            )
        except subprocess.TimeoutExpired:
            continue
        if result.returncode == 0:
            print("guest=arm64 cross=x86_64-linux-gnu")
            return
        time.sleep(min(2.0, max(0.0, deadline - time.monotonic())))
    raise ReproError("ARM guest did not become reachable before the startup deadline")


def _guest_build_script(config_entries: dict[str, str]) -> bytes:
    encoded = base64.b64encode(json.dumps(config_entries, sort_keys=True).encode()).decode()
    script = r"""set -euo pipefail
export SOURCE_DATE_EPOCH=1794334026
export KBUILD_BUILD_TIMESTAMP='Tue Nov 10 18:07:06 UTC 2026'
export KBUILD_BUILD_USER=nagi
export KBUILD_BUILD_HOST=linux-boot
export KBUILD_BUILD_VERSION=1
if [ "$(uname -m)" != aarch64 ] || [ "$(x86_64-linux-gnu-gcc -dumpmachine)" != x86_64-linux-gnu ]; then
    echo 'guest architecture or cross compiler mismatch' >&2
    exit 70
fi
mapfile -t configured_builds < <(find /home/builder -maxdepth 7 -type f -path '*/build/.config' -print)
if [ "${#configured_builds[@]}" -ne 1 ]; then
    echo 'expected exactly one pinned kernel build input' >&2
    exit 71
fi
baseline_builddir=${configured_builds[0]%/.config}
kernel_root=${baseline_builddir%/build}
srcdir="$kernel_root/source/linux-6.18.63"
if [ ! -f "$srcdir/Makefile" ] || [ ! -d "$srcdir/arch/x86" ]; then
    echo 'pinned kernel source is missing beside its original build tree' >&2
    exit 72
fi
version=$(make -s -C "$srcdir" ARCH=x86_64 CROSS_COMPILE=x86_64-linux-gnu- KERNELRELEASE=6.18.63 kernelversion)
if [ "$version" != 6.18.63 ]; then
    echo 'kernel source version mismatch' >&2
    exit 73
fi
builddir=/home/builder/nagi-kernel-fbdev-6.18.63-r1
make_flags=(O="$builddir" ARCH=x86_64 CROSS_COMPILE=x86_64-linux-gnu- KERNELRELEASE=6.18.63)
if [ ! -s "$builddir/.config" ]; then
    make -C "$srcdir" "${make_flags[@]}" x86_64_defconfig >&2
fi
python3 - "$builddir/.config" __CONFIG_B64__ <<'PY'
import base64
import json
import os
import re
import sys
import tempfile
from pathlib import Path

expected = json.loads(base64.b64decode(sys.argv[2]))
path = Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
for name, value in expected.items():
    pattern = re.compile(rf"^(?:{re.escape(name)}=.*|# {re.escape(name)} is not set)$")
    lines = [line for line in lines if not pattern.fullmatch(line)]
    lines.append(f"# {name} is not set" if value == "n" else f"{name}={value}")
fd, temporary = tempfile.mkstemp(prefix=".nagi-kernel-config-", dir=path.parent)
with os.fdopen(fd, "w", encoding="utf-8") as stream:
    stream.write("\n".join(lines) + "\n")
    stream.flush()
    os.fsync(stream.fileno())
os.replace(temporary, path)
PY
make -s -C "$srcdir" "${make_flags[@]}" olddefconfig >&2
python3 - "$builddir/.config" __CONFIG_B64__ <<'PY'
import base64
import json
import re
import sys

expected = json.loads(base64.b64decode(sys.argv[2]))
actual = {}
with open(sys.argv[1], encoding="utf-8") as stream:
    for line in stream:
        line = line.rstrip("\n")
        match = re.fullmatch(r"(CONFIG_[A-Z0-9_]+)=(.*)", line)
        if match:
            actual[match.group(1)] = match.group(2)
            continue
        match = re.fullmatch(r"# (CONFIG_[A-Z0-9_]+) is not set", line)
        if match:
            actual[match.group(1)] = "n"
if any(actual.get(name, "n") != value for name, value in expected.items()):
    print("kernel minimum config did not survive olddefconfig", file=sys.stderr)
    raise SystemExit(74)
PY
make -C "$srcdir" "${make_flags[@]}" -j4 bzImage >&2
image="$builddir/arch/x86/boot/bzImage"
if [ ! -s "$image" ]; then
    echo 'kernel build produced no bzImage' >&2
    exit 77
fi
digest=$(sha256sum "$image" | awk '{print $1}')
config_digest=$(sha256sum "$builddir/.config" | awk '{print $1}')
echo "kernel_version=$version bzImage_sha256=$digest" >&2
echo "kernel_config_sha256=$config_digest" >&2
grep -E '^(CONFIG_(DRM_BOCHS|DRM_FBDEV_EMULATION|DRM_VIRTIO_GPU|DRM_VIRTIO_GPU_KMS|FB|FB_DEVICE)=)' "$builddir/.config" >&2
cat "$image"
"""
    return script.replace("__CONFIG_B64__", encoded).encode()


def _parse_fragment(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        match = re.fullmatch(r"# (CONFIG_[A-Z0-9_]+) is not set", line)
        if match:
            name, value = match.group(1), "n"
        elif not line or line.startswith("#"):
            continue
        else:
            match = re.fullmatch(r"(CONFIG_[A-Z0-9_]+)=(.*)", line)
            if match is None:
                raise ReproError("kernel minimum config contains an unsupported entry")
            name, value = match.groups()
        if name in values and values[name] != value:
            raise ReproError(f"kernel minimum config conflicts on {name}")
        values[name] = value
    if not values:
        raise ReproError("kernel minimum config has no entries")
    return values


def _read_config_values(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        match = re.fullmatch(r"# (CONFIG_[A-Z0-9_]+) is not set", line)
        if match:
            values[match.group(1)] = "n"
            continue
        match = re.fullmatch(r"(CONFIG_[A-Z0-9_]+)=(.*)", line)
        if match:
            values[match.group(1)] = match.group(2)
    return values


def _is_bzimage(path: Path) -> bool:
    with path.open("rb") as stream:
        stream.seek(0x202)
        return stream.read(4) == b"HdrS"


def _build(args: argparse.Namespace) -> int:
    inputs = _build_inputs(args.vm_dir, args.output, args.firmware)
    config_entries = _parse_fragment(inputs["config"])
    log_fd = _new_log(inputs["log"])
    port = _free_loopback_port()
    qemu_argv = [
        inputs["qemu_arm"], "-machine", "virt", "-accel", "hvf", "-cpu", "host",
        "-smp", "4", "-m", "8G", "-bios", str(inputs["firmware"]),
        "-drive", f"if=none,file={inputs['overlay']},format=qcow2,id=systemdisk",
        "-device", "virtio-blk-device,drive=systemdisk",
        "-device", "virtio-scsi-device,id=scsi0",
        "-drive", f"if=none,file={inputs['cloud_seed']},format=raw,media=cdrom,readonly=on,id=cidata",
        "-device", "scsi-cd,drive=cidata,bus=scsi0.0",
        "-netdev", f"user,id=net0,hostfwd=tcp:127.0.0.1:{port}-:22",
        "-device", "virtio-net-pci,netdev=net0", "-display", "none",
        "-serial", "stdio", "-monitor", "none", "-no-reboot",
    ]
    deadline = time.monotonic() + args.timeout
    child: subprocess.Popen[bytes] | None = None
    guest_reachable = False
    temp_dir: Path | None = None
    try:
        print(f"qemu=ARM64/HVF timeout={args.timeout}s serial_log={_relative(inputs['log'])}")
        child = _spawn(qemu_argv, stdout=log_fd, stderr=log_fd)
        _wait_guest(inputs, child, port, min(deadline, time.monotonic() + args.startup_timeout))
        guest_reachable = True
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise ChildDeadline
        temp_dir = Path(tempfile.mkdtemp(prefix=".arm-bzimage-", dir=inputs["output"].parent))
        os.chmod(temp_dir, 0o700)
        received = temp_dir / "bzImage"
        received_config = temp_dir / "kernel.config"
        ssh_argv = [*_ssh_argv(inputs, port), "bash", "-s"]
        try:
            with received.open("xb") as output:
                result = subprocess.run(
                    ssh_argv, input=_guest_build_script(config_entries),
                    stdout=output, stderr=subprocess.PIPE, timeout=remaining, check=False,
                )
                output.flush()
                os.fsync(output.fileno())
        except subprocess.TimeoutExpired as error:
            raise ChildDeadline from error
        if result.returncode != 0:
            detail = result.stderr.decode("utf-8", "replace").strip()[-2000:]
            raise ReproError(
                f"guest kernel build/extraction failed (ssh rc={result.returncode})"
                + (f": {detail}" if detail else "")
            )
        detail = result.stderr.decode("utf-8", "replace")
        remote_hash = re.search(r"bzImage_sha256=([0-9a-f]{64})", detail)
        remote_config_hash = re.search(r"kernel_config_sha256=([0-9a-f]{64})", detail)
        if remote_hash is None:
            raise ReproError("guest did not return the bzImage checksum")
        if remote_config_hash is None:
            raise ReproError("guest did not return the kernel config checksum")
        if not received.is_file() or received.stat().st_size == 0 or not _is_bzimage(received):
            raise ReproError("received kernel is not a non-empty x86 bzImage")
        local_hash = _digest(received)
        if local_hash != remote_hash.group(1):
            raise ReproError("downloaded bzImage checksum differs from the guest")
        config_script = (
            "set -euo pipefail\n"
            "cat /home/builder/nagi-kernel-fbdev-6.18.63-r1/.config\n"
        ).encode()
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise ChildDeadline
        try:
            with received_config.open("xb") as output:
                config_result = subprocess.run(
                    ssh_argv, input=config_script, stdout=output, stderr=subprocess.PIPE,
                    timeout=remaining, check=False,
                )
                output.flush()
                os.fsync(output.fileno())
        except subprocess.TimeoutExpired as error:
            raise ChildDeadline from error
        if config_result.returncode != 0:
            detail = config_result.stderr.decode("utf-8", "replace").strip()[-1000:]
            raise ReproError(
                f"guest kernel config extraction failed (ssh rc={config_result.returncode})"
                + (f": {detail}" if detail else "")
            )
        actual_config = _read_config_values(received_config)
        if any(actual_config.get(name, "n") != value for name, value in config_entries.items()):
            raise ReproError("downloaded kernel config does not match the pinned fragment")
        config_hash = _digest(received_config)
        if config_hash != remote_config_hash.group(1):
            raise ReproError("downloaded kernel config checksum differs from the guest")
        print(f"kernel_build=verified bzImage_sha256={local_hash}")
        print(f"kernel_config=verified sha256={config_hash}")
        for name in (
            "CONFIG_DRM_BOCHS", "CONFIG_DRM_VIRTIO_GPU", "CONFIG_DRM_FBDEV_EMULATION",
            "CONFIG_FB", "CONFIG_FB_CORE", "CONFIG_FB_DEVICE",
        ):
            print(f"{name}={actual_config.get(name, 'n')}")

        subprocess.run(
            [*_ssh_argv(inputs, port), "sudo", "-n", "systemctl", "poweroff", "--no-wall"],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            timeout=max(1, min(15, int(deadline - time.monotonic()))), check=False,
        )
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise ChildDeadline
        try:
            qemu_rc = child.wait(timeout=min(remaining, 25))
        except subprocess.TimeoutExpired as error:
            raise ReproError("ARM/HVF guest did not stop after requested poweroff") from error
        if qemu_rc != 0:
            raise ReproError(f"ARM/HVF QEMU child exited unexpectedly (rc={qemu_rc})")
        try:
            os.link(received_config, inputs["config_output"], follow_symlinks=False)
            os.link(received, inputs["output"], follow_symlinks=False)
        except FileExistsError:
            raise ReproError("refusing to overwrite a kernel artifact") from None
        print(f"artifact={_relative(inputs['output'])} bytes={received.stat().st_size}")
        print(f"config_artifact={_relative(inputs['config_output'])} bytes={received_config.stat().st_size}")
        print(f"qemu_child_exit={qemu_rc}")
        return 0
    except ChildDeadline:
        print("build_result=TIMEOUT", file=sys.stderr)
        return 124
    finally:
        if child is not None and child.poll() is None:
            if guest_reachable:
                try:
                    subprocess.run(
                        [*_ssh_argv(inputs, port), "sudo", "-n", "systemctl", "poweroff", "--no-wall"],
                        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL, timeout=5, check=False,
                    )
                except (OSError, subprocess.TimeoutExpired):
                    pass
            _terminate_owned(child)
        os.close(log_fd)
        if temp_dir is not None:
            shutil.rmtree(temp_dir)


def _read_serial(path: Path) -> bytes:
    info = os.stat(path)
    if info.st_size > SERIAL_LIMIT:
        raise ReproError("serial capture exceeded the 4 MiB classifier limit")
    return path.read_bytes()


def _boot_check(args: argparse.Namespace) -> int:
    if args.observe_seconds > args.timeout:
        raise ReproError("wait observation must fit within the QEMU timeout")
    if args.probe_marker is not None:
        if not args.probe_marker or not args.probe_marker.isascii() or "\n" in args.probe_marker:
            raise ReproError("probe marker must be a nonempty ASCII line prefix")
        if not args.probe_expect or not args.probe_expect.isascii() or "\n" in args.probe_expect:
            raise ReproError("probe expectation must be a nonempty ASCII line")
    elif args.probe_expect is not None:
        raise ReproError("probe expectation requires --probe-marker")
    kernel = _regular(
        _path_in_worktree(args.kernel, allow_missing_leaf=False), "x86 kernel image",
    )
    if not _is_bzimage(kernel):
        raise ReproError("kernel input is not an x86 bzImage")
    initramfs = Path(args.initramfs).resolve(strict=True)
    _regular(initramfs, "read-only initramfs")
    qemu = _tool("qemu-system-x86_64")
    log = _path_in_worktree(args.log, allow_missing_leaf=True)
    _directory(log.parent, "serial log directory")
    log_fd = _new_log(log)
    argv = [
        qemu, "-machine", args.machine, "-accel", "tcg,thread=multi", "-cpu", "max",
        "-smp", str(args.vcpus), "-m", "2G", "-kernel", str(kernel), "-initrd", str(initramfs),
        "-append", "console=ttyS0,115200 rdinit=/init",
    ]
    if args.stdvga:
        argv.extend(["-device", "VGA,xres=320,yres=200,edid=on"])
    argv.extend([
        "-display", "none",
        "-serial", "stdio", "-monitor", "none", "-no-reboot",
    ])
    deadline = time.monotonic() + args.timeout
    child: subprocess.Popen[bytes] | None = None
    timed_out = False
    wait_started: float | None = None
    wait_confirmed = False
    probe_marker_confirmed = False
    probe_marker_since: float | None = None
    panic_seen = False
    ready_text_seen = False
    unavailable_text = b"nagi-init: startup failed (Unavailable); no service is ready"
    wait_text = b"nagi-init: PID 1 remains alive in fail-closed wait; no service is ready"
    ready_text = b"nagi-init: state=ready"
    try:
        display = "stdvga-320x200-edid" if args.stdvga else "default"
        print(f"qemu=x86_64/TCG machine={args.machine} display={display} vcpus={args.vcpus} timeout={args.timeout}s serial_log={_relative(log)}")
        child = _spawn(argv, stdout=log_fd, stderr=log_fd)
        while time.monotonic() < deadline:
            if child.poll() is not None:
                break
            try:
                current = _read_serial(log)
            except FileNotFoundError:
                current = b""
            if b"Kernel panic - not syncing:" in current:
                panic_seen = True
                _terminate_owned(child)
                break
            if ready_text in current:
                ready_text_seen = True
                _terminate_owned(child)
                break
            if args.probe_marker is not None and args.probe_marker.encode("ascii") in current:
                if probe_marker_since is None:
                    probe_marker_since = time.monotonic()
                elif time.monotonic() - probe_marker_since >= args.observe_seconds:
                    probe_marker_confirmed = child.poll() is None
                    _terminate_owned(child)
                    break
            if wait_text in current:
                if wait_started is None:
                    wait_started = time.monotonic()
                elif time.monotonic() - wait_started >= args.observe_seconds:
                    wait_confirmed = child.poll() is None
                    _terminate_owned(child)
                    break
            if child.poll() is not None:
                break
            time.sleep(min(0.1, max(0.0, deadline - time.monotonic())))
        else:
            timed_out = True
            _terminate_owned(child)
        qemu_rc = child.returncode
        serial = _read_serial(log)
        raw_lines = serial.decode("utf-8", "replace").splitlines()
        lines = [re.sub(r"^\[\s*\d+\.\d+\]\s*", "", line) for line in raw_lines]
        # Linux may reset early printk timestamps when clocksource calibration
        # switches clocks. Runtime markers use serial order, not the stricter
        # offline transcript timestamp classifier.
        kernel_positions = [i for i, line in enumerate(lines) if line.startswith("Linux version ")]
        init_positions = [i for i, line in enumerate(lines) if line == "Run /init as init process"]
        unavailable_positions = [
            i for i, line in enumerate(lines)
            if line == unavailable_text.decode()
        ]
        other_init_seen = any(
            line.startswith("Run ") and line.endswith(" as init process")
            and line != "Run /init as init process"
            for line in lines
        )
        init_failure_seen = any(
            line.startswith("Failed to execute ") or line.startswith("Kernel panic - not syncing:")
            for line in lines
        )
        kernel_seen = len(kernel_positions) == 1
        pid1_seen = len(init_positions) == 1
        unavailable = len(unavailable_positions) == 1
        panic_seen = panic_seen or any(line.startswith("Kernel panic - not syncing:") for line in lines)
        ready_text_seen = ready_text_seen or ready_text.decode() in lines
        fb0_registered = any(re.search(r"\bfb0:\s", line) for line in lines)
        if args.probe_marker is not None:
            probe_lines = [line for line in lines if line.startswith(args.probe_marker)]
            probe_marker_count = len(probe_lines)
            expected_probe = args.probe_expect or "NAGI_FBDEV_PROBE=PASS"
            probe_pass = probe_marker_count == 1 and probe_lines[0] == expected_probe
            print(f"qemu_probe_marker={'OBSERVED' if probe_marker_count == 1 else 'NOT_UNIQUE'}")
            print(f"qemu_probe_result={probe_lines[0] if probe_marker_count == 1 else 'UNAVAILABLE'}")
            print(f"qemu_probe_child_alive_after_observe={'YES' if probe_marker_confirmed else 'NO'}")
            print(f"kernel_text={'OBSERVED' if kernel_seen else 'NOT_OBSERVED'} pid1_invocation={'OBSERVED' if pid1_seen else 'NOT_OBSERVED'}")
            print(f"fb0_kernel_registered={'OBSERVED' if fb0_registered else 'NOT_OBSERVED'}")
            print(f"kernel_panic={'OBSERVED' if panic_seen else 'NOT_OBSERVED'} ready_marker={'OBSERVED' if ready_text_seen else 'NOT_OBSERVED'}")
            print(f"qemu_child_exit={child.returncode}")
            if timed_out:
                return 124
            if panic_seen or ready_text_seen or not kernel_seen or not pid1_seen:
                return 2
            if probe_pass and probe_marker_confirmed and fb0_registered and child.returncode == 0:
                return 0
            return 3
        ordered = (
            kernel_seen and pid1_seen and unavailable
            and kernel_positions[0] < init_positions[0] < unavailable_positions[0]
        )
        ambiguous = (
            len(kernel_positions) > 1 or len(init_positions) > 1
            or len(unavailable_positions) > 1 or other_init_seen
            or init_failure_seen or panic_seen
            or bool(kernel_positions and init_positions and kernel_positions[0] >= init_positions[0])
            or bool(init_positions and unavailable_positions and init_positions[0] >= unavailable_positions[0])
        )
        if ambiguous:
            serial_stage = "AMBIGUOUS_TRANSCRIPT"
        elif ordered:
            serial_stage = "POST_GUARD_UNAVAILABLE_TEXT_OBSERVED"
        elif kernel_positions:
            serial_stage = "KERNEL_TEXT_OBSERVED"
        elif init_positions:
            serial_stage = "INIT_EXEC_ATTEMPT_OBSERVED"
        else:
            serial_stage = "UNKNOWN"
        service_state = (
            "NOT_READY_DIAGNOSTIC" if unavailable else
            "UNVERIFIED_READY_TEXT_ONLY" if ready_text_seen else
            "UNKNOWN"
        )
        wait_state = "CONFIRMED_ALIVE" if wait_confirmed else (
            "MESSAGE_ONLY" if wait_text.decode() in lines else "NOT_OBSERVED"
        )
        print(f"kernel_text={'OBSERVED' if kernel_seen else 'NOT_OBSERVED'}")
        print(f"pid1_invocation={'OBSERVED' if pid1_seen else 'NOT_OBSERVED'}")
        print(f"service_ready={service_state} actual_pid1_verified=False")
        print(f"pid1_wait={wait_state} kernel_panic={'OBSERVED' if panic_seen else 'NOT_OBSERVED'}")
        print(f"ready_marker={'OBSERVED' if ready_text_seen else 'NOT_OBSERVED'}")
        print(f"fb0_kernel_registered={'OBSERVED' if fb0_registered else 'NOT_OBSERVED'}")
        print(f"serial_stage={serial_stage} qemu_child_exit={qemu_rc}")
        if timed_out:
            return 124
        if not kernel_seen or not pid1_seen:
            return 1
        if panic_seen or ready_text_seen:
            return 2
        if serial_stage == "AMBIGUOUS_TRANSCRIPT":
            return 4
        if ordered and wait_confirmed:
            return 0
        return 3
    finally:
        if child is not None and child.poll() is None:
            _terminate_owned(child)
        os.close(log_fd)


def _self_test() -> int:
    short = _spawn([sys.executable, "-c", "raise SystemExit(23)"])
    if _wait_owned(short, time.monotonic() + 5) != 23:
        raise ReproError("owned child exit-code check failed")
    long = _spawn([sys.executable, "-c", "import time; time.sleep(30)"])
    try:
        _wait_owned(long, time.monotonic() + 0.2)
    except ChildDeadline:
        pass
    else:
        raise ReproError("owned child timeout check did not time out")
    if long.poll() is None:
        _terminate_owned(long)
        raise ReproError("timed-out owned child was not reaped")
    print("managed_child=exit-code-and-timeout-cleanup-verified")
    return 0


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    build = commands.add_parser("build", help="cross-build with ARM Linux under HVF")
    build.add_argument("--vm-dir", default=str(DEFAULT_VM.relative_to(ROOT)))
    build.add_argument("--output", required=True, help="new artifact path inside the assigned worktree")
    build.add_argument("--firmware", default=str(FIRMWARE_DEFAULT))
    build.add_argument("--timeout", type=_timeout, default=600)
    build.add_argument("--startup-timeout", type=_timeout, default=180)
    build.set_defaults(handler=_build)
    boot = commands.add_parser("boot-check", help="boot an x86 kernel and report observed stages")
    boot.add_argument("--kernel", required=True, help="bzImage path inside the assigned worktree")
    boot.add_argument("--initramfs", required=True, help="read-only CPIO input")
    boot.add_argument("--log", required=True, help="new serial log path inside the assigned worktree")
    boot.add_argument("--timeout", type=_timeout, default=120)
    boot.add_argument("--vcpus", type=_vcpus, default=1)
    boot.add_argument("--machine", choices=("q35", "pc"), default="q35")
    boot.add_argument("--stdvga", action="store_true", help="attach QEMU stdvga at 320x200 with EDID")
    boot.add_argument("--observe-seconds", type=_timeout, default=3)
    boot.add_argument("--probe-marker", help="stop after this development-probe line while QEMU remains alive")
    boot.add_argument("--probe-expect", help="exact development-probe line; defaults to NAGI_FBDEV_PROBE=PASS")
    boot.set_defaults(handler=_boot_check)
    test = commands.add_parser("self-test", help="check owned-child timeout and reaping behavior")
    test.set_defaults(handler=lambda _args: _self_test())
    return parser


def main() -> int:
    args = _parser().parse_args()
    try:
        return args.handler(args)
    except (ReproError, ChildDeadline) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    except FileNotFoundError as error:
        missing = Path(error.filename).name if error.filename else "unknown"
        print(f"error: required file unavailable ({missing})", file=sys.stderr)
        return 2
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {type(error).__name__}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
