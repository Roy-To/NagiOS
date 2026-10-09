#!/usr/bin/env python3
"""Convert pinned multilingual-e5-small inputs into a Nagi `.nemb` v1 container.

Standard library only. The conversion is deterministic: the same verified
inputs always produce byte-identical output, whose SHA-256 is pinned in
tools/embedding/manifest.toml ([converted].sha256).

Container layout (all integers little-endian):

    magic        8 bytes  b"NAGIEMB\\0"
    version      u32      1
    header_len   u32      byte length of the header block
    header       fixed fields, see HEADER_FIELDS below, then
                 32-byte SHA-256 of model.safetensors,
                 32-byte SHA-256 of tokenizer.json,
                 40-byte ASCII upstream revision
    pieces       n_pieces * (f32 score, u8 kind, u16 len, len bytes UTF-8)
    charsmap     charsmap_len bytes (SentencePiece precompiled normalizer)
    tensors      n_tensors * (u16 name_len, name, u32 ndim, u32 dims[ndim],
                 u32 dtype=0 (f32), padding to 4-byte file offset, f32 data)

Piece kinds: 0 normal, 1 control (<s>, <pad>, </s>, <mask>), 2 unknown.
"""

import argparse
import base64
import hashlib
import json
import os
import struct
import sys
import tomllib

MAGIC = b"NAGIEMB\0"
VERSION = 1
HEADER_FIELDS = (
    "hidden", "layers", "heads", "intermediate", "max_positions", "type_vocab",
    "vocab_rows", "unk_id", "bos_id", "eos_id", "pad_id", "pooling",
    "normalize", "n_pieces", "pieces_len", "charsmap_len", "n_tensors",
)
TENSOR_ORDER_GLOBAL = (
    "embeddings.word_embeddings.weight",
    "embeddings.position_embeddings.weight",
    "embeddings.token_type_embeddings.weight",
    "embeddings.LayerNorm.weight",
    "embeddings.LayerNorm.bias",
)
TENSOR_ORDER_LAYER = (
    "attention.self.query.weight", "attention.self.query.bias",
    "attention.self.key.weight", "attention.self.key.bias",
    "attention.self.value.weight", "attention.self.value.bias",
    "attention.output.dense.weight", "attention.output.dense.bias",
    "attention.output.LayerNorm.weight", "attention.output.LayerNorm.bias",
    "intermediate.dense.weight", "intermediate.dense.bias",
    "output.dense.weight", "output.dense.bias",
    "output.LayerNorm.weight", "output.LayerNorm.bias",
)
CONTROL_TOKENS = {"<s>", "<pad>", "</s>", "<mask>"}


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def fail(message):
    sys.exit(f"convert_e5: {message}")


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--input", required=True, help="directory produced by fetch.sh")
    parser.add_argument("--output", required=True, help="output .nemb path")
    parser.add_argument("--manifest", default=os.path.join(here, "manifest.toml"))
    args = parser.parse_args()

    with open(args.manifest, "rb") as f:
        manifest = tomllib.load(f)
    pinned = {e["name"]: e for e in manifest["model"]["files"]}
    revision = manifest["model"]["revision"]
    digests = {}
    for name, entry in pinned.items():
        path = os.path.join(args.input, name)
        if not os.path.isfile(path):
            fail(f"missing pinned input {name}")
        if os.path.getsize(path) != entry["size_bytes"]:
            fail(f"size mismatch for {name}")
        digests[name] = sha256_file(path)
        if digests[name] != entry["sha256"]:
            fail(f"sha256 mismatch for {name}: {digests[name]}")

    with open(os.path.join(args.input, "config.json"), "rb") as f:
        config = json.load(f)
    expected = {
        "model_type": "bert", "hidden_act": "gelu", "position_embedding_type": "absolute",
    }
    for key, value in expected.items():
        if config.get(key) != value:
            fail(f"unsupported config {key}={config.get(key)!r}")
    hidden = config["hidden_size"]
    layers = config["num_hidden_layers"]
    heads = config["num_attention_heads"]
    intermediate = config["intermediate_size"]
    max_positions = config["max_position_embeddings"]
    type_vocab = config["type_vocab_size"]
    vocab_rows = config["vocab_size"]
    ln_eps = config["layer_norm_eps"]

    with open(os.path.join(args.input, "tokenizer.json"), "rb") as f:
        tok = json.load(f)
    model = tok["model"]
    if model["type"] != "Unigram" or model.get("byte_fallback"):
        fail("tokenizer model is not a plain Unigram model")
    norms = tok["normalizer"]["normalizers"]
    if [n["type"] for n in norms] != ["Precompiled", "Replace"] or \
            norms[1]["pattern"] != {"Regex": " {2,}"} or norms[1]["content"] != " ":
        fail("unexpected normalizer pipeline")
    pre = tok["pre_tokenizer"]
    if pre.get("type") != "Metaspace" or pre.get("replacement") != "\u2581" or \
            not pre.get("add_prefix_space"):
        fail("unexpected pre-tokenizer")
    charsmap = base64.b64decode(norms[0]["precompiled_charsmap"])
    vocab = model["vocab"]
    unk_id = model["unk_id"]
    pieces = bytearray()
    for index, (piece, score) in enumerate(vocab):
        raw = piece.encode("utf-8")
        if index == unk_id:
            kind = 2
        elif piece in CONTROL_TOKENS:
            kind = 1
        else:
            kind = 0
        if not raw or len(raw) > 0xFFFF:
            fail(f"invalid piece {index}")
        pieces += struct.pack("<fBH", float(score), kind, len(raw)) + raw
    ids = {piece: i for i, (piece, _) in enumerate(vocab)}
    bos_id, eos_id, pad_id = ids["<s>"], ids["</s>"], ids["<pad>"]
    if len(vocab) > vocab_rows:
        fail("tokenizer vocabulary exceeds embedding rows")

    st_path = os.path.join(args.input, "model.safetensors")
    with open(st_path, "rb") as f:
        header_len = struct.unpack("<Q", f.read(8))[0]
        st_header = json.loads(f.read(header_len))
    data_start = 8 + header_len
    names = list(TENSOR_ORDER_GLOBAL)
    for layer in range(layers):
        names += [f"encoder.layer.{layer}.{suffix}" for suffix in TENSOR_ORDER_LAYER]

    header = struct.pack(
        "<" + "I" * len(HEADER_FIELDS),
        hidden, layers, heads, intermediate, max_positions, type_vocab, vocab_rows,
        unk_id, bos_id, eos_id, pad_id, 1, 1, len(vocab), len(pieces), len(charsmap),
        len(names),
    )
    header += struct.pack("<f", ln_eps)
    header += bytes.fromhex(digests["model.safetensors"])
    header += bytes.fromhex(digests["tokenizer.json"])
    header += revision.encode("ascii")

    tmp = args.output + ".part"
    out_hash = hashlib.sha256()
    with open(tmp, "wb") as out, open(st_path, "rb") as src:
        def emit(blob):
            out.write(blob)
            out_hash.update(blob)

        emit(MAGIC + struct.pack("<II", VERSION, len(header)) + header)
        emit(bytes(pieces))
        emit(charsmap)
        for name in names:
            meta = st_header.get(name)
            if meta is None:
                fail(f"missing tensor {name}")
            if meta["dtype"] != "F32":
                fail(f"tensor {name} is {meta['dtype']}, expected F32")
            shape = meta["shape"]
            begin, end = meta["data_offsets"]
            count = 1
            for dim in shape:
                count *= dim
            if end - begin != count * 4:
                fail(f"tensor {name} size mismatch")
            raw_name = name.encode("ascii")
            record = struct.pack("<H", len(raw_name)) + raw_name
            record += struct.pack("<I", len(shape)) + struct.pack(f"<{len(shape)}I", *shape)
            record += struct.pack("<I", 0)
            emit(record)
            pad = (-out.tell()) % 4
            emit(b"\0" * pad)
            src.seek(data_start + begin)
            remaining = end - begin
            while remaining:
                block = src.read(min(remaining, 1 << 22))
                if not block:
                    fail(f"truncated tensor {name}")
                emit(block)
                remaining -= len(block)
    os.replace(tmp, args.output)
    print(json.dumps({
        "output": args.output,
        "size_bytes": os.path.getsize(args.output),
        "sha256": out_hash.hexdigest(),
        "pieces": len(vocab),
        "tensors": len(names),
    }))


if __name__ == "__main__":
    main()
