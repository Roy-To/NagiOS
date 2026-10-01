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

The current no-exception coverage is incremental: the GGUF parser/writer
slice and chat-template status lookup compile without exceptions. The full
Nagi-target `llama` build still fails on exception syntax in shared model
loading, context, grammar, KV-cache, tokenizer, sampler, and model-specific
sources. Further patches must propagate those errors explicitly; replacing
them with aborts or omitting required model paths would not preserve runtime
behavior.
