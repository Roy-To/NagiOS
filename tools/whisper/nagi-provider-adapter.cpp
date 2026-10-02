// Nagi-owned, path-free ABI adapter for the pinned whisper.cpp library.

#include <stddef.h>
#include <string.h>

#include "whisper.h"

typedef size_t (*nagi_whisper_read_fn)(void *, void *, size_t);
typedef bool (*nagi_whisper_eof_fn)(void *);
typedef void (*nagi_whisper_close_fn)(void *);

constexpr int samples_per_audio_context = 320; // 20 ms at 16 kHz, after encoder stride.
constexpr int audio_context_tail_samples = 16000; // Keep one second of zero-padding.

constexpr int audio_context_for_samples(int sample_count, int maximum_context) {
  if (sample_count <= 0 || maximum_context <= 0) {
    return 0;
  }
  const int sample_context =
      sample_count / samples_per_audio_context +
      (sample_count % samples_per_audio_context != 0 ? 1 : 0);
  const int tail_context = audio_context_tail_samples / samples_per_audio_context;
  const int requested_context = sample_context + tail_context;
  return requested_context < maximum_context ? requested_context : maximum_context;
}

static_assert(audio_context_for_samples(23605, 1500) == 124);
static_assert(audio_context_for_samples(1, 1500) == 51);
static_assert(audio_context_for_samples(524288, 1500) == 1500);

extern "C" void *nagi_m25_whisper_init(
    void *reader_context,
    nagi_whisper_read_fn read_callback,
    nagi_whisper_eof_fn eof_callback,
    nagi_whisper_close_fn close_callback) {
  if (reader_context == nullptr || read_callback == nullptr ||
      eof_callback == nullptr || close_callback == nullptr) {
    return nullptr;
  }

  whisper_model_loader loader = {};
  loader.context = reader_context;
  loader.read = read_callback;
  loader.eof = eof_callback;
  loader.close = close_callback;

  whisper_context_params params = whisper_context_default_params();
  params.use_gpu = false;
  params.flash_attn = false;
  params.gpu_device = 0;
  return whisper_init_with_params(&loader, params);
}

extern "C" int nagi_m25_whisper_transcribe(
    void *opaque_context, const float *samples, int sample_count,
    int thread_count, const char *language, char *destination,
    size_t destination_size, size_t *written) {
  if (opaque_context == nullptr || samples == nullptr || sample_count <= 0 ||
      thread_count <= 0 || language == nullptr || destination == nullptr ||
      destination_size == 0 || written == nullptr) {
    return 1;
  }

  whisper_context *context = static_cast<whisper_context *>(opaque_context);
  whisper_full_params params = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
  params.n_threads = thread_count;
  params.translate = false;
  params.no_context = true;
  params.no_timestamps = true;
  params.print_special = false;
  params.print_progress = false;
  params.print_realtime = false;
  params.print_timestamps = false;
  params.language = language;
  params.detect_language = strcmp(language, "auto") == 0;
  params.suppress_blank = true;
  params.suppress_nst = true;
  params.temperature = 0.0f;
  params.temperature_inc = 0.0f;
  params.audio_ctx =
      audio_context_for_samples(sample_count, whisper_n_audio_ctx(context));
  if (params.audio_ctx <= 0) {
    return 1;
  }

  if (whisper_full(context, params, samples, sample_count) != 0) {
    return 2;
  }

  size_t total = 0;
  const int segment_count = whisper_full_n_segments(context);
  if (segment_count < 0) {
    return 3;
  }
  for (int index = 0; index < segment_count; ++index) {
    const char *segment = whisper_full_get_segment_text(context, index);
    if (segment == nullptr) {
      return 3;
    }
    const size_t length = strlen(segment);
    if (length > destination_size - total) {
      return 4;
    }
    memcpy(destination + total, segment, length);
    total += length;
  }
  if (total == 0) {
    return 5;
  }
  *written = total;
  return 0;
}

extern "C" void nagi_m25_whisper_free(void *opaque_context) {
  if (opaque_context != nullptr) {
    whisper_free(static_cast<whisper_context *>(opaque_context));
  }
}
