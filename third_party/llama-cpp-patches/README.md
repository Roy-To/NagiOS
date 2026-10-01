# Nagi llama.cpp patches

This directory is the tracked patch boundary for the pinned llama.cpp source in
`third_party/sources.lock`. Number `.patch` files to define application order.
The fetch tool keeps the upstream checkout pristine and builds an ignored,
fingerprinted checkout under `out/cache/llama-cpp-nagi` when patches exist.

Patch `0001-nagi-gguf-noexceptions.patch` bounds Nagi GGUF metadata, removes
exception paths from the patched parser/writer code, streams tensor writes in
8 KiB chunks, and reports write/flush failures. Its target `ggml-base` build
and host GGUF regression suite pass.

Patch `0004-nagi-chat-template-status.patch` uses the existing
`LLM_CHAT_TEMPLATE_UNKNOWN` value for unrecognized chat-template names in the
Nagi build and removes exception-based lookup from Nagi template detection.
Other builds keep the upstream `map::at` and detection behavior. This keeps
unknown-template handling explicit at the caller without translating an
invalid template into a successful chat format.

Patch `0005-nagi-grammar-status.patch` makes grammar parser errors explicit,
checks repetition and token-ID integer overflow, rejects malformed escapes and
undefined rules with parser status, and latches a failed runtime grammar so it
cannot admit more tokens. It also propagates sampler failure as
`LLAMA_TOKEN_NULL`. Grammar regex trigger matching remains available through
the upstream implementation, but invalid `std::regex` patterns have no
status-capable compile path in the current Nagi slice; malformed regex trigger
behavior in the no-exceptions target remains unverified.

Patch `0006-nagi-sampler-null-consumers.patch` documents the
`llama_sampler_sample()` failure sentinel and updates the pinned C++ and Swift
examples to stop before treating `LLAMA_TOKEN_NULL` as an end token, decoding
it to text, or submitting it in a later batch.

Patch `0007-nagi-hybrid-state-restore-rollback.patch` extends the existing
state-restore failure test for generated hybrid models. It truncates the
serialized recurrent suffix after attention state has been read and checks
that a failed restore leaves the target sequence empty, returns its serialized
state size to the empty baseline, and preserves another sequence's logits.
This is host regression coverage for the rollback contract; it does not remove
the current exception-based error propagation.

The current no-exception coverage is incremental: the GGUF parser/writer,
chat-template status lookup, and grammar parser translation unit compile
without exceptions. The full Nagi-target `llama` build still fails on exception
syntax in shared model loading, context, KV-cache, tokenizer, sampler, and
model-specific sources. Further patches must propagate those errors
explicitly; replacing them with aborts or omitting required model paths would
not preserve runtime behavior.
