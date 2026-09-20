#ifndef NAGI_RELIBC_TIME_H
#define NAGI_RELIBC_TIME_H

/*
 * The generated relibc time.h contains the common POSIX declarations, but
 * cbindgen cannot select relibc's target_os=nagi re-export when it is invoked
 * on the header module directly.  Keep the clock IDs aligned with the real
 * Nagi POSIX ABI (user/nagi-posix/src/abi.rs), not with Linux's IDs.
 */
#include_next <time.h>

#ifdef CLOCK_REALTIME
#undef CLOCK_REALTIME
#endif
#define CLOCK_REALTIME 1

#ifdef CLOCK_MONOTONIC
#undef CLOCK_MONOTONIC
#endif
#define CLOCK_MONOTONIC 4

#endif
