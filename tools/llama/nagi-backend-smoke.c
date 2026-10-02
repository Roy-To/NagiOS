#include <ctype.h>
#include <math.h>
#include "llama.h"
#include "ggml-backend.h"

int nagi_m20_libcpp_int_to_string_smoke(void);

int nagi_m20_llama_backend_init_smoke(void) {
    if (nagi_m20_libcpp_int_to_string_smoke() != 0 || tolower('N') != 'n') {
        return 2;
    }
    const float expm1_sample = expm1f(0.25f);
    const float erf_sample = erff(0.5f);
    if (expm1_sample < 0.28402f || expm1_sample > 0.28404f ||
        erf_sample < 0.52049f || erf_sample > 0.52051f || erff(-0.0f) != 0.0f) {
        return 3;
    }
    if (ggml_backend_reg_count() == 0) {
        return 4;
    }
    llama_backend_init();
    const int cpu_registered = ggml_backend_reg_by_name("CPU") != 0;
    llama_backend_free();
    return cpu_registered ? 0 : 1;
}
