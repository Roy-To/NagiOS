#define _GNU_SOURCE
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/reboot.h>
#include <unistd.h>

struct PosixTimespec {
    int64_t tv_sec;
    int64_t tv_nsec;
};

extern int32_t nagi_pal_posix_clock_gettime(int32_t, struct PosixTimespec *);
extern int32_t nagi_pal_posix_errno(void);

static int fail(int code, const char *message) {
    fprintf(stderr, "POSIX_PAL_CLOCK_FAIL code=%d reason=%s\n", code, message);
    return code;
}

int main(void) {
    struct PosixTimespec first = {-1, -1};
    struct PosixTimespec second = {-1, -1};
    struct PosixTimespec sentinel = {17, 19};

    if (nagi_pal_posix_clock_gettime(4, &first) != 0 || first.tv_sec < 0 ||
        first.tv_nsec < 0 || first.tv_nsec >= 1000000000) {
        return fail(21, "first monotonic read");
    }
    if (nagi_pal_posix_clock_gettime(4, &second) != 0 || second.tv_sec < first.tv_sec ||
        (second.tv_sec == first.tv_sec && second.tv_nsec < first.tv_nsec)) {
        return fail(22, "monotonic ordering");
    }

    if (nagi_pal_posix_clock_gettime(1, &sentinel) != -1 ||
        nagi_pal_posix_errno() != ENOSYS || sentinel.tv_sec != 17 || sentinel.tv_nsec != 19) {
        return fail(23, "unsupported realtime must fail closed");
    }
    if (nagi_pal_posix_clock_gettime(99, &sentinel) != -1 ||
        nagi_pal_posix_errno() != EINVAL || sentinel.tv_sec != 17 || sentinel.tv_nsec != 19) {
        return fail(24, "unknown clock id must fail closed");
    }
    if (nagi_pal_posix_clock_gettime(4, NULL) != -1 ||
        nagi_pal_posix_errno() != EINVAL || sentinel.tv_sec != 17 || sentinel.tv_nsec != 19) {
        return fail(25, "null output must fail closed");
    }

    printf("POSIX_PAL_LINUX_CLOCK PASS monotonic=%lld.%09lld unsupported=ENOSYS invalid=EINVAL\n",
           (long long)second.tv_sec, (long long)second.tv_nsec);
    fflush(stdout);
    sync();
    if (reboot(RB_POWER_OFF) != 0) {
        return fail(26, "PID 1 poweroff");
    }
    return 27;
}
