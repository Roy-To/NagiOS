#!/usr/bin/env python3
"""Independent reference for the Nagi encoder (verification only, never shipped).

Tokenizes with Hugging Face `tokenizers` and the pinned `tokenizer.json`, runs
the upstream `onnx/model.onnx` from the same pinned revision with
`onnxruntime`, applies mean pooling + L2 normalization exactly as the model
card describes, and writes token ids and vectors as JSON.

Requires: pip install tokenizers onnxruntime numpy  (no network at run time)

Usage: reference_e5.py --input <fetch dir> --corpus <json list of strings>
                       --output <json>
"""

import argparse
import hashlib
import json
import os
import sys
import tomllib

import numpy as np
import onnxruntime as ort
from tokenizers import Tokenizer


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    p = argparse.ArgumentParser()
    p.add_argument("--input", required=True)
    p.add_argument("--corpus", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--manifest", default=os.path.join(here, "manifest.toml"))
    a = p.parse_args()
    with open(a.manifest, "rb") as f:
        manifest = tomllib.load(f)
    pins = {e["name"]: e["sha256"] for e in manifest["model"]["files"]}
    pins[manifest["reference"]["name"]] = manifest["reference"]["sha256"]
    for name in ("tokenizer.json", "onnx/model.onnx"):
        if sha256_file(os.path.join(a.input, name)) != pins[name]:
            sys.exit(f"reference_e5: {name} does not match its pin")

    tok = Tokenizer.from_file(os.path.join(a.input, "tokenizer.json"))
    sess = ort.InferenceSession(os.path.join(a.input, "onnx/model.onnx"),
                                providers=["CPUExecutionProvider"])
    feed_names = {i.name for i in sess.get_inputs()}
    with open(a.corpus, encoding="utf-8") as f:
        corpus = json.load(f)
    out = []
    for text in corpus:
        enc = tok.encode(text)
        ids = np.array([enc.ids], dtype=np.int64)
        mask = np.ones_like(ids)
        feed = {"input_ids": ids, "attention_mask": mask}
        if "token_type_ids" in feed_names:
            feed["token_type_ids"] = np.zeros_like(ids)
        hidden = sess.run(None, feed)[0][0]
        pooled = hidden.astype(np.float64).mean(axis=0)
        vec = pooled / np.linalg.norm(pooled)
        out.append({"text": text, "ids": enc.ids,
                    "embedding": [round(float(v), 7) for v in vec]})
    with open(a.output, "w", encoding="utf-8") as f:
        json.dump({
            "reference": "onnxruntime %s + tokenizers, onnx/model.onnx sha256 %s" %
                         (ort.__version__, pins["onnx/model.onnx"]),
            "revision": manifest["model"]["revision"],
            "items": out,
        }, f, ensure_ascii=False, indent=1)
    print(f"reference_e5: wrote {len(out)} items to {a.output}")


if __name__ == "__main__":
    main()
