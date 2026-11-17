#!/usr/bin/env python3
"""Build the tokenizer parity corpus and its reference token ids.

Sources (all deterministic): the repository's own en-US / ja-JP localization
strings, and seeded random strings drawn from many Unicode blocks (CJK,
kana incl. half-width, Hangul, Arabic, Devanagari, Thai, Cyrillic, Latin with
combining marks, full-width forms, emoji with modifiers/ZWJ, whitespace and
control characters). Reference ids come from Hugging Face `tokenizers` with the
pinned `tokenizer.json`. Inputs that contain the literal special-token strings
are excluded (the Nagi tokenizer deliberately never promotes them).

Usage: tokenizer_parity.py --input <fetch dir> --repo <repo root> --output <json>
"""

import argparse
import hashlib
import json
import os
import random
import re
import sys
import tomllib

from tokenizers import Tokenizer

SPECIALS = ("<s>", "</s>", "<pad>", "<unk>", "<mask>")
BLOCKS = [
    (0x0020, 0x007E), (0x00A0, 0x00FF), (0x0300, 0x036F), (0x0400, 0x04FF),
    (0x0600, 0x06FF), (0x0900, 0x097F), (0x0E00, 0x0E7F), (0x1100, 0x11FF),
    (0x2000, 0x206F), (0x2100, 0x218F), (0x2460, 0x24FF), (0x3000, 0x303F),
    (0x3040, 0x309F), (0x30A0, 0x30FF), (0x3200, 0x33FF), (0x4E00, 0x9FFF),
    (0xAC00, 0xD7A3), (0xF900, 0xFAFF), (0xFF00, 0xFFEF), (0x1F300, 0x1F6FF),
    (0x1F900, 0x1F9FF), (0x1D400, 0x1D7FF), (0x20000, 0x2A6DF),
]
EXTRAS = ["\u200d", "\ufe0f", "\U0001F3FD", "\t", "\n", "\r\n", "  ", "\u3000",
          "\u00ad", "\u200b", "\ufeff", "\x00", "\x7f"]


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def localization_strings(repo):
    out = []
    for rel in ("crates/nagi-i18n/locales/en-US.json", "crates/nagi-i18n/locales/ja-JP.json"):
        path = os.path.join(repo, rel)
        if os.path.exists(path):
            with open(path, encoding="utf-8") as f:
                data = json.load(f)
            stack = [data]
            while stack:
                item = stack.pop()
                if isinstance(item, dict):
                    stack.extend(item.values())
                elif isinstance(item, list):
                    stack.extend(item)
                elif isinstance(item, str):
                    out.append(item)
    for rel in ("user/nagi-localization/locales/ja-JP.lang",
                "user/nagi-localization/locales/en-US.lang",
                "apps/nagi-files/locales/ja-JP.properties",
                "user/nagi-notes/locales/ja-JP.properties"):
        path = os.path.join(repo, rel)
        if os.path.exists(path):
            with open(path, encoding="utf-8") as f:
                for line in f:
                    m = re.match(r"^\s*[^#;=\s][^=]*=\s*(.+?)\s*$", line)
                    if m:
                        out.append(m.group(1))
    return out


def random_strings(count, seed=24):
    rng = random.Random(seed)
    out = []
    for _ in range(count):
        parts = []
        for _ in range(rng.randint(1, 40)):
            if rng.random() < 0.12:
                parts.append(rng.choice(EXTRAS))
                continue
            lo, hi = rng.choice(BLOCKS)
            c = rng.randint(lo, hi)
            if 0xD800 <= c <= 0xDFFF:
                continue
            parts.append(chr(c))
            if rng.random() < 0.2:
                parts.append(" ")
        out.append("".join(parts))
    return out


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    p = argparse.ArgumentParser()
    p.add_argument("--input", required=True)
    p.add_argument("--repo", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--random", type=int, default=600)
    a = p.parse_args()
    with open(os.path.join(here, "manifest.toml"), "rb") as f:
        pins = {e["name"]: e["sha256"] for e in tomllib.load(f)["model"]["files"]}
    tj = os.path.join(a.input, "tokenizer.json")
    if sha256_file(tj) != pins["tokenizer.json"]:
        sys.exit("tokenizer_parity: tokenizer.json does not match its pin")
    tok = Tokenizer.from_file(tj)
    texts = localization_strings(a.repo) + random_strings(a.random)
    seen, items = set(), []
    for text in texts:
        if text in seen or any(s in text for s in SPECIALS):
            continue
        seen.add(text)
        items.append({"text": text, "ids": tok.encode(text).ids})
    with open(a.output, "w", encoding="utf-8") as f:
        json.dump({"tokenizer_sha256": pins["tokenizer.json"], "items": items},
                  f, ensure_ascii=True, separators=(",", ":"))
    print(f"tokenizer_parity: {len(items)} items -> {a.output}")


if __name__ == "__main__":
    main()
