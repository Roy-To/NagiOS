#ifndef NAGI_MESA_COMPAT_H
#define NAGI_MESA_COMPAT_H

#if defined(__NAGI__) && !defined(__ASSEMBLER__)
#include <stdlib.h>

/* Meson's freestanding link probes permit unresolved symbols, so a declared
 * host API can look linkable even when the Nagi target ABI does not provide
 * it. Keep Mesa on its portable temporary-file fallback instead of enabling
 * the Linux memfd path from that false positive.
 */
#undef HAVE_MEMFD_CREATE

/* Nagi's static userspace has no program-header iterator or ELF link.h. */
#undef HAVE_DL_ITERATE_PHDR

/* relibc exposes sched.h, but Nagi has no process-affinity operation. */
#undef HAS_SCHED_GETAFFINITY

/* Mesa's EGL pointer check has an existing fallback when mincore is absent. */
#undef HAVE_MINCORE

/* Nagi has no setuid-style secure execution identity. Keep the target's
 * portable secure_getenv fallback explicit and prevent Mesa from defining a
 * second copy in u_debug.h.
 */
#ifndef HAVE_SECURE_GETENV
#define HAVE_SECURE_GETENV 1
static inline char *secure_getenv(const char *name)
{
    return getenv(name);
}
#endif
#endif

#endif
