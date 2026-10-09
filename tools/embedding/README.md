# tools/embedding — M24 embedding model pipeline

Reproducible, credential-free pipeline for the Nagi M24 embedding provider
(`crates/nagi-embedding-provider`).

| Step | Command | Output |
| --- | --- | --- |
| Fetch pinned upstream files (immutable revision, size + SHA-256 checked) | `tools/embedding/fetch.sh [--with-reference]` | `${NAGI_EMBEDDING_CACHE:-~/.cache/nagi-embedding}/<revision>/` |
| Convert (stdlib-only, deterministic) | `python3 tools/embedding/convert_e5.py --input <dir> --output multilingual-e5-small.nemb` | `.nemb` whose SHA-256 must equal `manifest.toml` `[converted].sha256` |
| Reference vectors (verification only) | `reference_e5.py --input <dir> --corpus tests/m24-embedding/parity_corpus.json --output …` | needs `pip install tokenizers onnxruntime numpy` and `--with-reference` |
| Tokenizer parity corpus (verification only) | `tokenizer_parity.py --input <dir> --repo . --output tests/m24-embedding/tokenizer_parity.json` | needs `pip install tokenizers` |
| Full host acceptance | `tests/m24-embedding/run.sh` | log under `$NAGI_EMBEDDING_CACHE/evidence/` |

Pins (see `manifest.toml`): `intfloat/multilingual-e5-small` at
`614241f622f53c4eeff9890bdc4f31cfecc418b3`, MIT. The ONNX export is used only
as an independent reference and is never converted or shipped.

The `.nemb` v1 layout is documented in `convert_e5.py` and parsed by
`crates/nagi-embedding-provider/src/container.rs`.
