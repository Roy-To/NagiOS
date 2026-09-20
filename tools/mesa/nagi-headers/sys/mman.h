#ifndef NAGI_RELIBC_SYS_MMAN_H
#define NAGI_RELIBC_SYS_MMAN_H

/*
 * cbindgen currently does not select relibc's target-specific sys/mman
 * constants for the Nagi target.  Keep Mesa's mmap flags aligned with the
 * Nagi libc ABI instead of modifying the generated header.
 */
#include_next <sys/mman.h>

#ifdef PROT_EXEC
#undef PROT_EXEC
#endif
#define PROT_EXEC 0x0001

#ifdef PROT_WRITE
#undef PROT_WRITE
#endif
#define PROT_WRITE 0x0002

#ifdef PROT_READ
#undef PROT_READ
#endif
#define PROT_READ 0x0004

#ifdef MAP_SHARED
#undef MAP_SHARED
#endif
#define MAP_SHARED 0x0001

#ifdef MAP_PRIVATE
#undef MAP_PRIVATE
#endif
#define MAP_PRIVATE 0x0002

#ifdef MAP_ANON
#undef MAP_ANON
#endif
#define MAP_ANON 0x0020

#ifdef MAP_ANONYMOUS
#undef MAP_ANONYMOUS
#endif
#define MAP_ANONYMOUS MAP_ANON

#endif
