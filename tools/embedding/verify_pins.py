#!/usr/bin/env python3
"""Check that every pin for the M24 embedding artifact agrees.

Standard library only. Verifies, and prints one line per check:

1. tools/embedding/manifest.toml: original inputs (model.safetensors,
   tokenizer.json, config.json) at the immutable upstream revision, and the
   converted `.nemb` size + SHA-256.
2. The `.nemb` header (written by convert_e5.py) embeds the SHA-256 of the
   exact model.safetensors and tokenizer.json it was converted from and the
   40-hex upstream revision; these must equal the manifest pins.
3. The `.nemb` file itself hashes to manifest [converted].sha256 and has the
   pinned size.
4. crates/nagi-embedding-provider/src/lib.rs constants E5_SMALL_NEMB_SHA256,
   E5_SMALL_NEMB_BYTES and E5_SMALL_REVISION equal the manifest.
5. Optional (--input DIR): the original upstream files in DIR match their pins.
6. Optional (--models-lock-proposal FILE): every value in the proposed
   third_party/models.lock entry equals the manifest.

config.json is not embedded in the header: its values (hidden size, layers,
heads, vocabulary, positions, eps) are copied into the header fields and the
converter refuses inputs whose config.json does not match its pinned SHA-256.

Usage: verify_pins.py --artifact multilingual-e5-small.nemb [--input DIR]
                      [--models-lock-proposal .dev/workstreams/hark-m24-embedding/integration-proposal.md]
"""

import argparse
import hashlib
import os
import re
import struct
import sys
import tomllib

HEADER_U32 = 17  # convert_e5.HEADER_FIELDS


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    repo = os.path.abspath(os.path.join(here, "..", ".."))
    p = argparse.ArgumentParser()
    p.add_argument("--artifact", required=True)
    p.add_argument("--input")
    p.add_argument("--models-lock-proposal")
    p.add_argument("--manifest", default=os.path.join(here, "manifest.toml"))
    a = p.parse_args()
    with open(a.manifest, "rb") as f:
        m = tomllib.load(f)
    files = {e["name"]: e for e in m["model"]["files"]}
    conv = m["converted"]
    rev = m["model"]["revision"]
    failures = 0

    def check(label, ok, detail):
        nonlocal failures
        print(f"verify_pins: {'PASS' if ok else 'FAIL'} {label}: {detail}")
        failures += 0 if ok else 1

    check("revision is an immutable 40-hex commit", re.fullmatch(r"[0-9a-f]{40}", rev) is not None, rev)

    with open(a.artifact, "rb") as f:
        head = f.read(16 + HEADER_U32 * 4 + 4 + 32 + 32 + 40)
    magic, (version, header_len) = head[:8], struct.unpack("<II", head[8:16])
    check("nemb magic/version", magic == b"NAGIEMB\0" and version == 1, f"{magic!r} v{version}")
    off = 16 + HEADER_U32 * 4 + 4
    w_sha = head[off:off + 32].hex()
    t_sha = head[off + 32:off + 64].hex()
    h_rev = head[off + 64:off + 104].decode("ascii")
    check("header_len covers provenance", header_len == HEADER_U32 * 4 + 4 + 104, str(header_len))
    check("nemb header weights sha256 == manifest model.safetensors",
          w_sha == files["model.safetensors"]["sha256"], w_sha)
    check("nemb header tokenizer sha256 == manifest tokenizer.json",
          t_sha == files["tokenizer.json"]["sha256"], t_sha)
    check("nemb header revision == manifest revision", h_rev == rev, h_rev)

    size = os.path.getsize(a.artifact)
    digest = sha256_file(a.artifact)
    check("nemb size == manifest converted.size_bytes", size == conv["size_bytes"], str(size))
    check("nemb sha256 == manifest converted.sha256", digest == conv["sha256"], digest)

    lib = open(os.path.join(repo, "crates/nagi-embedding-provider/src/lib.rs"), encoding="utf-8").read()
    c_sha = re.search(r'E5_SMALL_NEMB_SHA256: \[u8; 32\] =\s*hex32\("([0-9a-f]{64})"\)', lib)
    c_size = re.search(r"E5_SMALL_NEMB_BYTES: u64 = ([0-9_]+);", lib)
    c_rev = re.search(r'E5_SMALL_REVISION: &str = "([0-9a-f]{40})";', lib)
    check("lib.rs E5_SMALL_NEMB_SHA256 == manifest", c_sha and c_sha.group(1) == conv["sha256"],
          c_sha.group(1) if c_sha else "missing")
    check("lib.rs E5_SMALL_NEMB_BYTES == manifest",
          c_size and int(c_size.group(1).replace("_", "")) == conv["size_bytes"],
          c_size.group(1) if c_size else "missing")
    check("lib.rs E5_SMALL_REVISION == manifest", c_rev and c_rev.group(1) == rev,
          c_rev.group(1) if c_rev else "missing")

    if a.input:
        for name, entry in sorted(files.items()):
            path = os.path.join(a.input, name)
            got = sha256_file(path) if os.path.isfile(path) else "missing"
            ok = got == entry["sha256"] and os.path.getsize(path) == entry["size_bytes"]
            check(f"upstream {name} == manifest", ok, got)

    if a.models_lock_proposal:
        text = open(a.models_lock_proposal, encoding="utf-8").read()
        block = re.search(r"\[models\.multilingual_e5_small\]\n(.*?)```", text, re.S)
        entry = tomllib.loads("[x]\n" + block.group(1))["x"] if block else {}
        expected = {
            "revision": rev,
            "file_name": "model.safetensors",
            "size_bytes": files["model.safetensors"]["size_bytes"],
            "sha256": files["model.safetensors"]["sha256"],
            "tokenizer_file_name": "tokenizer.json",
            "tokenizer_size_bytes": files["tokenizer.json"]["size_bytes"],
            "tokenizer_sha256": files["tokenizer.json"]["sha256"],
            "config_sha256": files["config.json"]["sha256"],
            "converted_file_name": conv["name"],
            "converted_format": conv["format"],
            "converted_size_bytes": conv["size_bytes"],
            "converted_sha256": conv["sha256"],
            "license": m["model"]["license"],
        }
        for key, value in expected.items():
            check(f"models.lock proposal {key}", entry.get(key) == value, str(entry.get(key)))

    print(f"verify_pins: {'PASS' if failures == 0 else 'FAIL'} ({failures} failing checks)")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
