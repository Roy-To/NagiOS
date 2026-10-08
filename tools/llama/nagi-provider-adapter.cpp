#include <cerrno>
#include <cstdio>
#include <cstring>
#include <new>

#include "llama.h"
#include "ggml-backend.h"

namespace {

constexpr uint32_t GRANITE_CONTEXT_TOKENS = 4096;
constexpr int32_t PREFILL_BATCH_TOKENS = 128;
constexpr int32_t MAX_PROMPT_TOKENS = 8192;
constexpr size_t MAX_FORMATTED_PROMPT_BYTES = 65536;
constexpr int32_t MAX_TOKEN_BYTES = 4096;

struct NagiLlamaSession {
    llama_model * model = nullptr;
    llama_context * context = nullptr;
    bool (*cancel)(void *) = nullptr;
    void * cancel_context = nullptr;
};

bool llama_abort(void * data) {
    auto * session = static_cast<NagiLlamaSession *>(data);
    return session != nullptr && session->cancel != nullptr && session->cancel(session->cancel_context);
}

void free_session(NagiLlamaSession * session) {
    if (session == nullptr) {
        return;
    }
    if (session->context != nullptr) {
        llama_free(session->context);
    }
    if (session->model != nullptr) {
        llama_model_free(session->model);
    }
    delete session;
}

bool append_piece(char * output, size_t output_capacity, size_t * output_size,
                  const char * piece, size_t piece_size) {
    if (output == nullptr || output_size == nullptr || piece == nullptr ||
        piece_size > output_capacity - *output_size) {
        return false;
    }
    std::memcpy(output + *output_size, piece, piece_size);
    *output_size += piece_size;
    return true;
}

} // namespace

extern "C" {

void llama_backend_init(void);
struct llama_model * llama_model_load_from_file_ptr(FILE *, struct llama_model_params);
FILE * fdopen(int, const char *);
int close(int);
int fclose(FILE *);

int nagi_m20_llama_backend_initialize(void) {
    llama_backend_init();
    return ggml_backend_reg_count() != 0 && ggml_backend_reg_by_name("CPU") != nullptr;
}

void * nagi_m20_llama_load_from_fd(int fd, uint32_t context_tokens, int32_t thread_count) {
    if (fd < 0 || context_tokens == 0 || context_tokens > GRANITE_CONTEXT_TOKENS || thread_count < 1) {
        if (fd >= 0) {
            close(fd);
        }
        return nullptr;
    }

    FILE * stream = fdopen(fd, "rb");
    if (stream == nullptr) {
        close(fd);
        return nullptr;
    }

    llama_model_params model_params = llama_model_default_params();
    model_params.n_gpu_layers = 0;
    model_params.load_mode = LLAMA_LOAD_MODE_NONE;
    model_params.check_tensors = true;

    auto * model = llama_model_load_from_file_ptr(stream, model_params);
    const int close_status = fclose(stream);
    if (model == nullptr || close_status != 0) {
        if (model != nullptr) {
            llama_model_free(model);
        }
        return nullptr;
    }

    auto * session = new (std::nothrow) NagiLlamaSession{};
    if (session == nullptr) {
        llama_model_free(model);
        return nullptr;
    }
    session->model = model;

    llama_context_params context_params = llama_context_default_params();
    context_params.n_ctx = context_tokens;
    context_params.n_batch = PREFILL_BATCH_TOKENS;
    context_params.n_ubatch = 32;
    context_params.n_seq_max = 1;
    context_params.n_threads = thread_count;
    context_params.n_threads_batch = thread_count;
    context_params.abort_callback = llama_abort;
    context_params.abort_callback_data = session;
    session->context = llama_init_from_model(model, context_params);
    if (session->context == nullptr) {
        free_session(session);
        return nullptr;
    }
    return session;
}

void nagi_m20_llama_set_cancel_callback(
    void * handle,
    bool (*callback)(void *),
    void * callback_context
) {
    auto * session = static_cast<NagiLlamaSession *>(handle);
    if (session == nullptr) {
        return;
    }
    session->cancel = callback;
    session->cancel_context = callback_context;
}

int nagi_m20_llama_generate(
    void * handle,
    const char * system_prompt,
    const char * input,
    const char * grammar,
    int32_t max_output_tokens,
    float temperature,
    float top_p,
    uint32_t seed,
    char * output,
    size_t output_capacity,
    size_t * output_size,
    uint32_t * input_tokens,
    uint32_t * output_tokens
) {
    auto * session = static_cast<NagiLlamaSession *>(handle);
    if (session == nullptr || session->model == nullptr || session->context == nullptr ||
        input == nullptr || output == nullptr || output_size == nullptr ||
        input_tokens == nullptr || output_tokens == nullptr || max_output_tokens <= 0 ||
        max_output_tokens >= static_cast<int32_t>(GRANITE_CONTEXT_TOKENS) ||
        temperature < 0.0f || temperature > 2.0f || top_p <= 0.0f || top_p > 1.0f) {
        return 1;
    }
    *output_size = 0;
    *input_tokens = 0;
    *output_tokens = 0;

    const llama_vocab * vocab = llama_model_get_vocab(session->model);
    const char * model_template = llama_model_chat_template(session->model, nullptr);
    if (vocab == nullptr || model_template == nullptr) {
        return 1;
    }

    const llama_chat_message messages[] = {
        {"system", system_prompt == nullptr ? "You are a helpful assistant." : system_prompt},
        {"user", input},
    };
    const size_t message_count = system_prompt == nullptr ? 1 : 2;
    const llama_chat_message * chat = system_prompt == nullptr ? &messages[1] : messages;
    char prompt[MAX_FORMATTED_PROMPT_BYTES];
    const int32_t prompt_size = llama_chat_apply_template(
        model_template, chat, message_count, true, prompt, sizeof(prompt));
    if (prompt_size <= 0 || static_cast<size_t>(prompt_size) >= sizeof(prompt)) {
        return 1;
    }
    prompt[prompt_size] = '\0';

    llama_token tokens[MAX_PROMPT_TOKENS];
    const int32_t token_count = llama_tokenize(
        vocab, prompt, prompt_size, tokens, MAX_PROMPT_TOKENS, true, true);
    if (token_count <= 0) {
        return 1;
    }
    if (static_cast<uint64_t>(token_count) + static_cast<uint64_t>(max_output_tokens) >
        llama_n_ctx(session->context)) {
        return 2;
    }
    *input_tokens = static_cast<uint32_t>(token_count);

    llama_memory_clear(llama_get_memory(session->context), true);
    for (int32_t first = 0; first < token_count;) {
        const int32_t count = (token_count - first) < PREFILL_BATCH_TOKENS
            ? token_count - first
            : PREFILL_BATCH_TOKENS;
        llama_batch batch = llama_batch_get_one(tokens + first, count);
        const int32_t status = llama_decode(session->context, batch);
        if (status != 0) {
            return status == 2 ? 3 : 1;
        }
        first += count;
    }

    llama_sampler_chain_params sampler_params = llama_sampler_chain_default_params();
    llama_sampler * sampler = llama_sampler_chain_init(sampler_params);
    if (sampler == nullptr) {
        return 1;
    }
    if (grammar != nullptr && grammar[0] != '\0') {
        llama_sampler * grammar_sampler = llama_sampler_init_grammar(vocab, grammar, "root");
        if (grammar_sampler == nullptr) {
            llama_sampler_free(sampler);
            return 4;
        }
        llama_sampler_chain_add(sampler, grammar_sampler);
    }
    if (temperature > 0.0f) {
        llama_sampler * temperature_sampler = llama_sampler_init_temp(temperature);
        llama_sampler * top_p_sampler = llama_sampler_init_top_p(top_p, 1);
        llama_sampler * distribution_sampler = llama_sampler_init_dist(seed);
        if (temperature_sampler == nullptr || top_p_sampler == nullptr || distribution_sampler == nullptr) {
            if (temperature_sampler != nullptr) llama_sampler_free(temperature_sampler);
            if (top_p_sampler != nullptr) llama_sampler_free(top_p_sampler);
            if (distribution_sampler != nullptr) llama_sampler_free(distribution_sampler);
            llama_sampler_free(sampler);
            return 1;
        }
        llama_sampler_chain_add(sampler, temperature_sampler);
        llama_sampler_chain_add(sampler, top_p_sampler);
        llama_sampler_chain_add(sampler, distribution_sampler);
    } else {
        llama_sampler * greedy_sampler = llama_sampler_init_greedy();
        if (greedy_sampler == nullptr) {
            llama_sampler_free(sampler);
            return 1;
        }
        llama_sampler_chain_add(sampler, greedy_sampler);
    }

    char piece[MAX_TOKEN_BYTES];
    size_t written = 0;
    int status = 0;
    for (int32_t generated = 0; generated < max_output_tokens; ++generated) {
        const llama_token token = llama_sampler_sample(sampler, session->context, -1);
        if (token < 0) {
            status = 1;
            break;
        }
        if (llama_vocab_is_eog(vocab, token)) {
            break;
        }
        const int32_t piece_size = llama_token_to_piece(vocab, token, piece, sizeof(piece), 0, false);
        if (piece_size < 0 || piece_size > MAX_TOKEN_BYTES ||
            !append_piece(output, output_capacity, &written, piece, static_cast<size_t>(piece_size))) {
            status = 5;
            break;
        }
        // llama_sampler_sample() accepts its selected token internally. A
        // second accept advances constrained grammar state twice.
        *output_tokens = static_cast<uint32_t>(generated + 1);
        llama_token token_for_decode = token;
        llama_batch batch = llama_batch_get_one(&token_for_decode, 1);
        const int32_t decode_status = llama_decode(session->context, batch);
        if (decode_status != 0) {
            status = decode_status == 2 ? 3 : 1;
            break;
        }
        if (session->cancel != nullptr && session->cancel(session->cancel_context)) {
            status = 3;
            break;
        }
    }
    llama_sampler_free(sampler);
    *output_size = written;
    return status;
}

void nagi_m20_llama_free(void * handle) {
    free_session(static_cast<NagiLlamaSession *>(handle));
}

} // extern "C"
