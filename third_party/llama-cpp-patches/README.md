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

Patch `0008-nagi-tensor-weight-status.patch` replaces the throwing tensor
weight constructor with checked initialization. It rejects absent tensor
metadata and truncated or overflowed file extents before inserting a weight,
stops loader construction on the first failure, and returns the existing model
load failure status before metadata output or model creation. Duplicate tensor
names remain rejected through the same status path. Its focused host test
covers valid extents, missing metadata index, EOF truncation, and addition
overflow; a separate target compile checks the helper with exceptions disabled.

Patch `0031-nagi-llama-dsv4-checked-status.patch` propagates invalid DSV4 batch,
stream, compressor-plan, and rollback metadata through `FAILED_PREPARE` rather
than throwing on Nagi. It constructs and validates compressor plans before
reserving raw cache slots, and stops state serialization after the first
checked I/O failure. Host validation still throws as before. Host and
Nagi-macro loader-bounds builds cover integration; DSV4-specific malformed
batch and state-writer fault injection are not yet available.

Patch `0032-nagi-llama-sampler-ring-status.patch` replaces sampler ring-buffer
throws with checked reads and a sticky failure bit on Nagi. Sampling returns
`LLAMA_TOKEN_NULL` after a ring failure. Backend graph probes now report
allocation/setup failure; a failed chain probe clears its partial backend
prefix and leaves CPU sampling available. Host ring and probe errors retain
their exceptions. The no-exception syntax check and existing host/Nagi
sampling tests cover compilation and normal sampler behavior; a direct
fault-injection test for internal ring corruption is still unavailable.

Patch `0033-nagi-llama-quantize-checked-status.patch` propagates checked status
through tensor type selection, dequantization, row quantization, and the model
quantization driver. Invalid layer metadata, unsupported conversions, invalid
imatrix data, and failed quantized-row validation stop the operation. The
`llama_quant_compute_types` helper now returns `bool`, initializes result slots
to `GGML_TYPE_COUNT`, and stages assignments so a failure cannot expose a
partial type list. Host builds retain their exception behavior. The host and
Nagi-macro quantization type-selection CTests pass; the Nagi test includes a
checked-status regression for an invalid quantization state.

The current incremental no-exception syntax sweep passes all 32 top-level
`src/*.cpp` translation units, including the DSV4 cache, sampler, and
quantizer. This is not a complete Nagi-target `llama` build: the last full
target attempt, before patches 0031–0033, stopped on exception syntax in
model-specific translation units, and that full target has not yet been
re-run. Direct fault injection for DSV4 malformed batches/state I/O and
sampler ring corruption is also unavailable. These compile and checked-status
results do not establish Granite inference; M20 remains `PARTIAL`.
