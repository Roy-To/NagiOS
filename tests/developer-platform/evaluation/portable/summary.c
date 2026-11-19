#include "summary.h"
int summarize_tasks(const unsigned char *states, size_t count,
                    struct task_summary *output) {
    struct task_summary result = {0, 0, 0};
    if (output == NULL || (states == NULL && count != 0)) return -1;
    for (size_t index = 0; index < count; ++index) {
        switch (states[index]) {
        case 0: ++result.open; break;
        case 1: ++result.done; break;
        case 2: ++result.blocked; break;
        default: return -1;
        }
    }
    *output = result;
    return 0;
}
