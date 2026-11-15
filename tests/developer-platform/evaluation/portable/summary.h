#ifndef NDP_SUMMARY_H
#define NDP_SUMMARY_H
#include <stddef.h>
struct task_summary { size_t open, done, blocked; };
/* 0=open, 1=done, 2=blocked. Invalid input leaves output unchanged. */
int summarize_tasks(const unsigned char *states, size_t count,
                    struct task_summary *output);
#endif
