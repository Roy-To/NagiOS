// Orchestration-only test: compile the production adapter against pinned headers,
// replacing llama calls with deterministic fixtures. No model or inference.
#include <cassert>
#include <climits>
#include <string>
#include "../../tools/llama/nagi-provider-adapter.cpp"

namespace {
int32_t formatted_size = 8;
int32_t token_result = 1;
int tokenize_calls = 0;
int decode_calls = 0;
int clear_calls = 0;
bool saw_system = false;
uint32_t context_size = 4096;
llama_sampler fake_sampler{};
}

extern "C" {
const llama_vocab * llama_model_get_vocab(const llama_model *) {
    return reinterpret_cast<const llama_vocab *>(1);
}
const char * llama_model_chat_template(const llama_model *, const char *) { return "fixture"; }
int32_t llama_chat_apply_template(const char *, const llama_chat_message * chat, size_t count,
                                bool, char * buf, int32_t capacity) {
    saw_system = count == 2 && std::string(chat[0].role) == "system" &&
        std::string(chat[0].content) == "system fixture";
    if (formatted_size > 0 && formatted_size < capacity) {
        std::memset(buf, 'x', static_cast<size_t>(formatted_size));
    }
    return formatted_size;
}
int32_t llama_tokenize(const llama_vocab *, const char *, int32_t size, llama_token *,
                       int32_t capacity, bool special, bool parse) {
    ++tokenize_calls;
    assert(size == formatted_size && capacity == MAX_PROMPT_TOKENS && special && parse);
    return token_result;
}
uint32_t llama_n_ctx(const llama_context *) { return context_size; }
llama_memory_t llama_get_memory(const llama_context *) { return nullptr; }
void llama_memory_clear(llama_memory_t, bool) { ++clear_calls; }
llama_batch llama_batch_get_one(llama_token *, int32_t) { return {}; }
int32_t llama_decode(llama_context *, llama_batch) { ++decode_calls; return 0; }
llama_sampler_chain_params llama_sampler_chain_default_params() { return {}; }
llama_sampler * llama_sampler_chain_init(llama_sampler_chain_params) { return &fake_sampler; }
void llama_sampler_chain_add(llama_sampler *, llama_sampler *) {}
llama_sampler * llama_sampler_init_grammar(const llama_vocab *, const char *, const char *) { return &fake_sampler; }
llama_sampler * llama_sampler_init_temp(float) { return &fake_sampler; }
llama_sampler * llama_sampler_init_top_p(float, size_t) { return &fake_sampler; }
llama_sampler * llama_sampler_init_dist(uint32_t) { return &fake_sampler; }
llama_sampler * llama_sampler_init_greedy() { return &fake_sampler; }
void llama_sampler_free(llama_sampler *) {}
llama_token llama_sampler_sample(llama_sampler *, llama_context *, int32_t) { return 0; }
bool llama_vocab_is_eog(const llama_vocab *, llama_token) { return true; }
int32_t llama_token_to_piece(const llama_vocab *, llama_token, char *, int32_t, int32_t, bool) {
    assert(false && "EOG must not produce text"); return 0;
}
}

int main() {
    NagiLlamaSession session{};
    session.model = reinterpret_cast<llama_model *>(1);
    session.context = reinterpret_cast<llama_context *>(1);
    // Every token boundary around input+output == context, plus the tokenizer's
    // negative required-capacity convention and its INT32_MIN overflow sentinel.
    for (int32_t output : {1, 64, 256}) {
        for (uint32_t context : {512u, 4096u}) {
            context_size = context;
            for (int32_t tokens : {0, -1, -8193, INT32_MIN,
                    static_cast<int32_t>(context) - output - 1,
                    static_cast<int32_t>(context) - output,
                    static_cast<int32_t>(context) - output + 1, 8192}) {
                token_result = tokens;
                tokenize_calls = decode_calls = clear_calls = 0;
                char text[16]{};
                size_t written = 99;
                uint32_t input_usage = 99, output_usage = 99;
                const int status = nagi_m20_llama_generate(&session, "system fixture", "page", nullptr,
                    output, 0.0f, 1.0f, 0, text, sizeof(text), &written, &input_usage, &output_usage);
                const bool too_large = tokens < 0 ||
                    static_cast<int64_t>(tokens) + output > context;
                assert(status == (too_large ? 2 : tokens == 0 ? 1 : 0));
                assert(saw_system && tokenize_calls == 1);
                if (status != 0) {
                    assert(decode_calls == 0 && clear_calls == 0);
                    assert(written == 0 && input_usage == 0 && output_usage == 0);
                } else {
                    assert(clear_calls == 1 && decode_calls > 0);
                    assert(input_usage == static_cast<uint32_t>(tokens));
                }
            }
        }
    }
    // Template buffer exhaustion is also an input-capacity failure and must
    // stop before tokenization/prefill. Unsupported templates stay general errors.
    for (int32_t size : {0, -1, 65535, 65536, 65537}) {
        formatted_size = size;
        token_result = 1;
        tokenize_calls = decode_calls = clear_calls = 0;
        char text[16]{};
        size_t written = 99;
        uint32_t input_usage = 99, output_usage = 99;
        int status = nagi_m20_llama_generate(&session, "system fixture", "page", nullptr,
            256, 0.0f, 1.0f, 0, text, sizeof(text), &written, &input_usage, &output_usage);
        assert(status == (size >= 65536 ? 2 : size <= 0 ? 1 : 0));
        assert(tokenize_calls == (size > 0 && size < 65536 ? 1 : 0));
        if (status != 0) assert(decode_calls == 0 && clear_calls == 0 && written == 0);
    }
}
