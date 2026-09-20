#ifndef NAGI_RELIBC_STDIO_H
#define NAGI_RELIBC_STDIO_H

/*
 * The pinned relibc stdio header exposes the common FILE API, but the
 * target-only relibc backend also provides the real Nagi memory-stream ABI
 * used by Mesa's util/memstream.c.  Keep this declaration in the tracked
 * Nagi include adapter instead of editing generated headers.
 */
#include_next <stdio.h>

#ifdef __cplusplus
extern "C" {
#endif

FILE *open_memstream(char **bufp, size_t *sizep);

#ifdef __cplusplus
}
#endif

#endif
