#include "summary.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>
int main(void) {
    struct task_summary out = {7, 8, 9};
    const unsigned char mixed[] = {0, 2, 1, 0, 1};
    const unsigned char done[] = {1, 1, 1};
    unsigned char invalid[] = {0, 1, 255};
    const unsigned char invalid_copy[] = {0, 1, 255};
    assert(summarize_tasks(NULL, 0, &out) == 0);
    assert(out.open == 0 && out.done == 0 && out.blocked == 0);
    assert(summarize_tasks(mixed, sizeof mixed, &out) == 0);
    assert(out.open == 2 && out.done == 2 && out.blocked == 1);
    assert(summarize_tasks(done, sizeof done, &out) == 0);
    assert(out.open == 0 && out.done == 3 && out.blocked == 0);
    out = (struct task_summary){7, 8, 9};
    assert(summarize_tasks(invalid, sizeof invalid, &out) == -1);
    assert(out.open == 7 && out.done == 8 && out.blocked == 9);
    assert(memcmp(invalid, invalid_copy, sizeof invalid) == 0);
    assert(summarize_tasks(NULL, 1, &out) == -1);
    assert(out.open == 7 && out.done == 8 && out.blocked == 9);
    assert(summarize_tasks(mixed, sizeof mixed, NULL) == -1);
    assert(summarize_tasks(NULL, 0, NULL) == -1);
    unsigned char mutable_states[] = {0, 2, 1, 0, 1};
    assert(summarize_tasks(mutable_states, sizeof mutable_states, &out) == 0);
    assert(memcmp(mutable_states, mixed, sizeof mixed) == 0);
    assert(summarize_tasks(mixed, sizeof mixed, &out) == 0);
    assert(out.open == 2 && out.done == 2 && out.blocked == 1);
    puts("PASS portable summary: empty, mixed, done, invalid, null, unchanged, repeat");
    return 0;
}
