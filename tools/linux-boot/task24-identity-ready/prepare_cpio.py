#!/usr/bin/env python3
"""Assemble signed Task24 boot inputs into the existing normal newc image."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_acceptance import _parse_newc_records
from build_dev_service_cpio import _entry


IDENTITY = "system/task24-identity-ready.elf"
PACKAGE_STORE = "package-store"
INDEX = ".package-store.lock", "package-digest-index.v1"
BLOB = re.compile(r"package-([0-9a-f]{64})\.xapp")


def assemble(base: bytes, inputs: Path, shim: bytes, init: bytes, journal: Path | None) -> bytes:
    catalog = (inputs / "catalog.bin").read_bytes()
    descriptor = json.loads(json.loads(catalog[:-64])["descriptor_json"])
    identity = (inputs / IDENTITY).read_bytes()
    if descriptor["executable"]["artifact_path"] != "/" + IDENTITY:
        raise ValueError("Identity artifact path differs from the signed catalog")
    if hashlib.sha256(identity).hexdigest() != descriptor["executable"]["expected_sha256"]:
        raise ValueError("Identity ELF digest differs from the signed catalog")
    plan = (inputs / "identity-ready-plan.bin").read_bytes()
    if json.loads(plan[:-64])["catalog_sha256"] != list(hashlib.sha256(catalog).digest()):
        raise ValueError("READY plan does not reference this catalog")

    replacements = {
        "init": (0o755, shim),
        "nagi-init": (0o755, init),
        "catalog.bin": (0o600, catalog),
        "identity-ready-plan.bin": (0o600, plan),
        IDENTITY: (0o755, identity),
    }
    store = inputs / PACKAGE_STORE
    for name in INDEX:
        replacements[f"{PACKAGE_STORE}/{name}"] = (0o600, (store / name).read_bytes())
    blobs = sorted(path for path in store.iterdir() if BLOB.fullmatch(path.name))
    if len(blobs) != 2:
        raise ValueError("expected the Identity and login package blobs")
    for blob in blobs:
        payload = blob.read_bytes()
        if BLOB.fullmatch(blob.name)[1] != hashlib.sha256(payload).hexdigest():
            raise ValueError("package blob differs from its content-addressed name")
        replacements[f"{PACKAGE_STORE}/{blob.name}"] = (0o600, payload)
    if journal is not None:
        payload = journal.read_bytes()
        if not payload:
            raise ValueError("empty C3 slot journal")
        replacements[f"{PACKAGE_STORE}/system-service-slot.v1"] = (0o600, payload)
        # C3 opens an adjacent private flock file before scanning the journal.
        # The initramfs starts a fresh owner, so its lock file must be present
        # but contain no prior lock state.
        replacements[f"{PACKAGE_STORE}/system-service-slot.v1.lock"] = (0o600, b"")

    _, records = _parse_newc_records(base)
    names = {name for name, _, _ in records}
    if not {"init", "nagi-init", "catalog.bin", PACKAGE_STORE, "system/fixture.elf"} <= names:
        raise ValueError("unsupported Task24 base image")
    output = bytearray()
    offset = 0
    inode = 1 + max(fields[0] for _, fields, _ in records)
    for name, fields, _ in records:
        end_name = (offset + 110 + fields[11] + 3) & ~3
        end = (end_name + fields[6] + 3) & ~3
        if name == PACKAGE_STORE:
            _entry(output, name, inode=fields[0], mode=stat.S_IFDIR | 0o700, nlink=2)
        elif (
            name not in replacements
            and not name.startswith(PACKAGE_STORE + "/")
            and name not in {"system/fixture.elf", "TRAILER!!!"}
        ):
            output.extend(base[offset:end])
        offset = end

    for name, (mode, payload) in replacements.items():
        _entry(output, name, inode=inode, mode=stat.S_IFREG | mode, nlink=1, payload=payload)
        inode += 1
    _entry(output, "TRAILER!!!", inode=inode, mode=0, nlink=1)
    archive = bytes(output)
    _, assembled = _parse_newc_records(archive)
    staged = {name: (fields, payload) for name, fields, payload in assembled}
    for name, (mode, payload) in replacements.items():
        fields, actual = staged[name]
        if fields[1:4] != [stat.S_IFREG | mode, 0, 0] or actual != payload:
            raise ValueError(f"incorrect archive metadata or content for {name}")
    if staged[PACKAGE_STORE][0][1:4] != [stat.S_IFDIR | 0o700, 0, 0]:
        raise ValueError("archive package store is not root-owned/private")
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("base", "inputs", "shim", "init", "output"):
        parser.add_argument(name, type=Path)
    parser.add_argument("--c3-journal", type=Path, help="C3-provisioned slot journal for normal boot")
    args = parser.parse_args()
    archive = assemble(
        args.base.read_bytes(), args.inputs, args.shim.read_bytes(), args.init.read_bytes(),
        args.c3_journal,
    )
    with args.output.open("xb") as output:
        output.write(archive)


if __name__ == "__main__":
    main()
