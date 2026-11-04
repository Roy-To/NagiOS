# M23 summary input boundary regressions

These are host orchestration tests, not model or guest acceptance evidence.
The Rust tests live beside the production Browser Summary adapter:

```sh
cargo test --locked --offline -p nagi-ai -p nagi-model-manager
cargo clippy --locked --offline -p nagi-ai --all-targets -- -D warnings
cargo test --locked --offline --manifest-path tests/model-session-service/Cargo.toml
cargo fmt -p nagi-ai -- --check
cargo check --locked --offline -p nagi-ai --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,alloc
cargo clippy --locked --offline --manifest-path tests/model-session-service/Cargo.toml --features granite-acceptance --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,alloc -- -D warnings
```

After the repository's pinned, patched llama.cpp source has been materialized,
run the native regression on an ELF host with a C++17 compiler:

```sh
bash tests/m23-summary/run.sh
# A separate validated source cache can also be selected:
NAGI_LLAMA_SOURCE=/path/to/out/cache/llama-cpp-nagi bash tests/m23-summary/run.sh
```

The native test includes the production adapter and pinned llama headers but
replaces llama calls with fixtures. Section garbage collection removes the
unused loader and backend-initialization entry points. It needs no model,
VM, model artifact I/O, or llama library build.

Coverage:

- All four browser fields retain the maximal UTF-8 prefix at their byte caps
  for one-, two-, three-, four-byte and mixed/control/escaped text.
- JSON serialization preserves the bounded text. Escaped control bytes,
  quotes and backslashes count at their serialized size. The maximum 24 KiB
  request budget includes system instructions. Smaller trusted caller budgets
  are preserved; limit-1, limit and limit+1 are exercised at four budgets,
  and oversized requests never call the provider.
- Output requests are capped at 256 tokens, or a smaller trusted caller limit;
  zero budgets cannot be constructed and large budgets cannot expand the caps.
- Model token counts remain unknown (`None`) at the summary boundary; the
  real tokenizer owns model-specific counts including the chat template and
  system instructions. `ContextLimit` reaches the caller as `InputTooLarge`;
  invalid responses and unavailable/cancelled/timeout providers stay errors.
- 53 native cases cover input tokens + reserved output at context-1, context,
  context+1 for multiple contexts/output budgets; negative tokenizer capacity
  results including `INT32_MIN`; zero tokens; and template-buffer boundaries.
  Capacity failures clear reported usage and never clear KV memory or prefill.

On public main `87b7dea4061be2572526181ab559d63037f823d0`, the Rust tests
reproduced accepted requests exceeding the combined budget and loss of
`ContextLimit` classification. The native test reproduced the negative
capacity result becoming status 1 instead of status 2. The fixes preserve
all authorization checks and reduce the summary output ceiling from 2048 to
256 tokens without increasing any provider/service/native capacity.

This combined candidate incorporates the reviewed output-budget source from
`ed67d36954e520122ee889715846ad513bd0ccdf` (current-main source-only equivalent
`aef433487cbe2ff5ecdd40070efadafb28f65046`) and input-boundary source transfer
identified as `3bfe3a0e8dfcc05bafb1c2057b80a2c678bec044`. The input transfer's
25013-byte patch SHA-256 was
`25e9dd3760012c7871a27675eaf3026ab4b8fb7bf5b6a960258b0bba9491c537`;
applying it to the exact base reproduced tree
`26abc34def5a3b3c66377798358301e359e8e984` before combination.

The overlapping admission predicate keeps `self.budget.max_input_bytes`.
The input test helper explicitly initializes the combined model fixture's
`error` and `max_output_tokens` fields. Its exact-byte test now includes
smaller caller budgets and fails if admission is mutated back to a fixed
24 KiB limit. Authenticated session routing and ordinary guest acceptance
remain separate work; no production worker or PR is changed by these tests.

Combined verification: 33 AI + 77 Model Manager + 3 session-access host tests
pass (113 total), as do host AI Clippy/formatting, the AI no-std target check
and actual production/Granite-acceptance Rust service leaf target Clippy.
Eight existing nagi-posix dependency warnings remain visible in the target leaf
check. Native 53-case orchestration passes. These checks do not establish native
Nagi C++ linking or actual guest/model inference.
