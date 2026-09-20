#ifndef NAGI_RELIBC_STDINT_H
#define NAGI_RELIBC_STDINT_H

/*
 * relibc's generated stdint.h uses GCC's __INT*_C builtins for the integer
 * constant macros.  The freestanding x86_64-unknown-elf clang used for the
 * Mesa target build does not provide those aliases, although it does provide
 * the underlying integer types used by that header.  Keep the C ABI adapter
 * in the tracked Nagi include boundary rather than changing generated relibc
 * output or teaching Mesa about the toolchain.
 */
#include_next <stdint.h>

#ifdef INT8_C
#undef INT8_C
#endif
#define INT8_C(c) c

#ifdef UINT8_C
#undef UINT8_C
#endif
#define UINT8_C(c) c

#ifdef INT16_C
#undef INT16_C
#endif
#define INT16_C(c) c

#ifdef UINT16_C
#undef UINT16_C
#endif
#define UINT16_C(c) c

#ifdef INT32_C
#undef INT32_C
#endif
#define INT32_C(c) c

#ifdef UINT32_C
#undef UINT32_C
#endif
#define UINT32_C(c) c##U

#ifdef INT64_C
#undef INT64_C
#endif
#define INT64_C(c) c##L

#ifdef UINT64_C
#undef UINT64_C
#endif
#define UINT64_C(c) c##UL

#ifdef INTMAX_C
#undef INTMAX_C
#endif
#define INTMAX_C(c) c##L

#ifdef UINTMAX_C
#undef UINTMAX_C
#endif
#define UINTMAX_C(c) c##UL

#endif
