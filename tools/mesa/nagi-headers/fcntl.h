#ifndef NAGI_RELIBC_FCNTL_H
#define NAGI_RELIBC_FCNTL_H

/*
 * relibc's fcntl cbindgen module currently selects only Linux or Redox
 * platform constants.  The Nagi libc ABI uses the descriptor and open flag
 * values from third_party/libc/src/unix/nagi.rs; expose those values at the C
 * include boundary without modifying generated relibc output.
 */
#include_next <fcntl.h>

#ifdef FD_CLOEXEC
#undef FD_CLOEXEC
#endif
#define FD_CLOEXEC 0x01000000

/* Nagi's access and creation flags use the Redox-compatible ABI values. */
#ifdef O_RDONLY
#undef O_RDONLY
#endif
#define O_RDONLY 0x00010000

#ifdef O_WRONLY
#undef O_WRONLY
#endif
#define O_WRONLY 0x00020000

#ifdef O_RDWR
#undef O_RDWR
#endif
#define O_RDWR 0x00030000

#ifdef O_ACCMODE
#undef O_ACCMODE
#endif
#define O_ACCMODE 0x00030000

#ifdef O_NONBLOCK
#undef O_NONBLOCK
#endif
#define O_NONBLOCK 0x00040000

#ifdef O_APPEND
#undef O_APPEND
#endif
#define O_APPEND 0x00080000

#ifdef O_CLOEXEC
#undef O_CLOEXEC
#endif
#define O_CLOEXEC 0x01000000

#ifdef O_CREAT
#undef O_CREAT
#endif
#define O_CREAT 0x02000000

#ifdef O_TRUNC
#undef O_TRUNC
#endif
#define O_TRUNC 0x04000000

#ifdef O_EXCL
#undef O_EXCL
#endif
#define O_EXCL 0x08000000

#endif
