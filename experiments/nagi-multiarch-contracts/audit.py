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

def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def inventory():
    paths = git("ls-files").splitlines()
    prefixes = ("kernel/", "loader/", "user/", "targets/", "tools/", "tests/", ".cargo/", ".github/workflows/", "crates/", "sdk/", "patches/")
    suffixes = (".rs", ".sh", ".py", ".toml", ".json", ".ld", ".yml", ".patch")
    files, matches, consumers = [], [], []
    for name in paths:
        if not name.startswith(prefixes) or not name.endswith(suffixes):
            continue
        data = (ROOT / name).read_bytes()
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
    return {"baseline_sha": git("rev-parse", "origin/main"), "scope": "tracked production/tool/test source; tests and comments included; indirect assumptions need semantic review",
            "rules": RULES, "files": files, "matches": matches, "bootinfo_field_references": consumers}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    result = inventory()
    # One record per line keeps generated evidence reviewable without discarding matches.
    entries = []
    for key, value in result.items():
        rendered = json.dumps(value)
        if isinstance(value, list):
            rendered = "[\n" + ",\n".join("    " + json.dumps(item) for item in value) + "\n  ]"
        entries.append("  " + json.dumps(key) + ": " + rendered)
    encoded = "{\n" + ",\n".join(entries) + "\n}\n"
    if args.check:
        if args.output.read_text() != encoded:
            raise SystemExit("FAIL inventory changed; review baseline and regenerate")
    else:
        args.output.write_text(encoded)
    print(f"PASS static inventory: {len(result['files'])} files, {len(result['matches'])} anchored matches, {len(result['bootinfo_field_references'])} BootInfo field references")


if __name__ == "__main__":
    main()
