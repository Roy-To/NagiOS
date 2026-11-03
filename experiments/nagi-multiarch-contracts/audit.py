#!/usr/bin/env python3
"""Reproducible static inventory; matches are evidence, not runtime proof."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
RULES = {
    "cpu-entry-traps": r'(?i)\b(x86_64|win64|syscall|sysretq?|iretq?|lgdt|lidt|ltr|rdmsr|wrmsr|cr[034]|invlpg|cli|sti|hlt|rflags|fxsave|fxrstor|cpuid)\b',
    "platform-discovery": r'(?i)\b(ACPI|APIC|IOAPIC|MADT|RSDP|UEFI|OVMF|q35|COM1|0x0?cf8|0x0?cfc|0x3[fF]8)\b',
    "transport-dma": r'(?i)\b(PCI|MMIO|DMA|VIRTIO|legacy|fence|read_volatile|write_volatile)\b|queue_address.*u32::MAX|asm!.*(in |out )',
    "address-layout": r'(?i)PML4|PDPT|0x0000[_]?4000|0x04000000|0xFFFF_FFFF|u32::MAX|PAGE_SIZE|USER_ADDRESS_LIMIT|USER_IMAGE_BASE|disable-redzone|code-model|EM_X86_64|read_u16\(bytes, 18\)',
    "userspace-toolchain": r'(?i)\b(x86(?:_64)?|aarch64|FS_BASE|TLS|SSE\d*|AVX\d*|simd|cpufeatures|relibc|soft-float)\b|target_arch|llvm-target',
    "reference-acceptance": r'(?i)MAX_CPUS|topology.count|qemu-system-x86|q35|\b(320|200)\b|\b(smp|8G|8192)\b',
}
FIELDS = ["magic", "version", "size", "memory_map", "framebuffer", "acpi_rsdp", "init_image", "realtime_epoch_ns", "boot_control"]
SOURCE_PREFIXES = ("kernel/", "loader/", "user/", "targets/", "tools/", "tests/", ".cargo/", ".github/workflows/", "crates/", "sdk/", "patches/")
SOURCE_SUFFIXES = (".rs", ".sh", ".py", ".toml", ".json", ".ld", ".yml", ".patch")
AUTHORITY_PATHS = (
    "AGENTS.md",
    "docs/implementation_status.md",
    "docs/Nagi_OS_0.1_Codex_Implementation_Spec.md",
    "docs/Nagi_OS_0.2_Codex_Implementation_Spec.md",
    "docs/Nagi_OS_0.3_Architecture_Roadmap.md",
    "docs/0.2/DEVELOPMENT_ARCHITECTURE.md",
    "docs/0.2/WORKSTREAMS.md",
    "docs/0.2/Nagi_OS_0.2_Parallel_Execution_Rules.md",
    ".dev/workstreams.json",
    ".dev/workstreams/integration-next-phase/state.json",
    "docs/decisions/ADR-0001-hybrid-kernel.md",
    "docs/decisions/0014-unified-device-application-model.md",
    "docs/architecture/unified-device-application-model.md",
)
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
LOCAL_GUARD_ROOTS = (".codex", ".entire")


class AuditError(ValueError):
    pass


def git(root, *args, check=True):
    result = subprocess.run(
        ["git", *args], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, check=False,
    )
    if check and result.returncode != 0:
        raise AuditError(result.stderr.strip() or f"git {' '.join(args)} failed")
    return result


def full_commit(root, value):
    if not isinstance(value, str) or not FULL_SHA.fullmatch(value):
        raise AuditError("base must be an explicit full 40-character lowercase commit SHA")
    result = git(root, "rev-parse", "--verify", f"{value}^{{commit}}", check=False)
    resolved = result.stdout.strip()
    if result.returncode != 0 or resolved != value:
        raise AuditError("base must resolve exactly to the supplied full commit SHA")
    return resolved


def guard_untracked_paths(root):
    raw = git(root, "ls-files", "--others", "--exclude-standard").stdout
    paths = sorted(p for p in raw.splitlines() if p)
    guard_paths = [p for p in paths if any(p == name or p.startswith(name + "/") for name in LOCAL_GUARD_ROOTS)]
    unexpected = sorted(set(paths) - set(guard_paths))
    return paths, guard_paths, unexpected


def require_tracked_clean(root):
    unstaged = git(root, "diff", "--name-only", "HEAD").stdout.splitlines()
    staged = git(root, "diff", "--cached", "--name-only", "HEAD").stdout.splitlines()
    _, guard_paths, unexpected = guard_untracked_paths(root)
    if unstaged or staged:
        raise AuditError("tracked worktree is dirty")
    if unexpected:
        raise AuditError("unexpected untracked source paths: " + ", ".join(unexpected))
    return {
        "tracked_clean": True,
        "local_guard_untracked_roots": sorted({p.split("/", 1)[0] for p in guard_paths}),
        "local_guard_untracked_count": len(guard_paths),
    }


def inventory(root=ROOT, base=None):
    if base is None:
        raise AuditError("an explicit --base commit is required")
    base = full_commit(root, base)
    head = git(root, "rev-parse", "HEAD").stdout.strip()
    tree = git(root, "rev-parse", "HEAD^{tree}").stdout.strip()
    ancestor = git(root, "merge-base", "--is-ancestor", base, head, check=False)
    if ancestor.returncode != 0:
        raise AuditError("base must be an ancestor of the inspected source commit")
    clean = require_tracked_clean(root)
    paths = git(root, "ls-files").stdout.splitlines()
    files, matches, consumers = [], [], []
    for name in paths:
        if not name.startswith(SOURCE_PREFIXES) or not name.endswith(SOURCE_SUFFIXES):
            continue
        data = (Path(root) / name).read_bytes()
        try:
            lines = data.decode("utf-8").splitlines()
        except UnicodeDecodeError:
            continue
        files.append({"path": name, "sha256": hashlib.sha256(data).hexdigest(), "lines": len(lines)})
        for line, text in enumerate(lines, 1):
            categories = [key for key, pattern in RULES.items() if re.search(pattern, text)]
            if categories:
                matches.append({"path": name, "line": line, "categories": categories, "text": text.strip()})
            if name.startswith(("kernel/", "loader/", "crates/nagi-bootinfo/")):
                for field in FIELDS:
                    if re.search(r"\." + field + r"\b", text) or re.search(r"\b" + field + r"\s*:", text):
                        consumers.append({"path": name, "line": line, "field": field, "text": text.strip()})
    return {
        "baseline_sha": base,
        "source_commit_sha": head,
        "source_tree_sha": tree,
        "source_status": clean,
        "scope": "tracked production/tool/test source; tests and comments included; indirect assumptions need semantic review",
        "rules": RULES,
        "files": files,
        "matches": matches,
        "bootinfo_field_references": consumers,
    }


def authority_sources(root=ROOT):
    result = {}
    for name in AUTHORITY_PATHS:
        data = (Path(root) / name).read_bytes()
        result[name] = hashlib.sha256(data).hexdigest()
    return result


def encode_record(value):
    entries = []
    for key, item in value.items():
        rendered = json.dumps(item, ensure_ascii=False)
        if isinstance(item, list):
            rendered = "[\n" + ",\n".join("    " + json.dumps(entry, ensure_ascii=False) for entry in item) + "\n  ]"
        entries.append("  " + json.dumps(key) + ": " + rendered)
    return "{\n" + ",\n".join(entries) + "\n}\n"


def write_or_check(path, content, check):
    path = Path(path)
    if check:
        if not path.is_file() or path.read_text() != content:
            raise AuditError(f"FAIL record changed or missing: {path}")
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", required=True, help="explicit full 40-character baseline commit SHA")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--authority-output", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)
    try:
        result = inventory(ROOT, args.base)
        write_or_check(args.output, encode_record(result), args.check)
        if args.authority_output:
            authority = authority_sources(ROOT)
            write_or_check(args.authority_output, json.dumps(authority, indent=2) + "\n", args.check)
    except (AuditError, OSError) as exc:
        parser.error(str(exc))
    print(
        f"PASS static inventory base={result['baseline_sha']} source={result['source_commit_sha']}: "
        f"{len(result['files'])} files, {len(result['matches'])} anchored matches, "
        f"{len(result['bootinfo_field_references'])} BootInfo field references"
    )
    if args.authority_output:
        print(f"PASS authority hashes: {len(AUTHORITY_PATHS)} sources")


if __name__ == "__main__":
    main()
