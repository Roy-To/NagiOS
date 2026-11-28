/* Static inspection only: do not build or execute on this host. */
#include <sys/inotify.h>
#include <unistd.h>
int watch_changes(const char *path) {
    int descriptor = inotify_init1(IN_NONBLOCK | IN_CLOEXEC);
    if (descriptor < 0) return -1;
    if (inotify_add_watch(descriptor, path, IN_MODIFY) < 0) {
        close(descriptor);
        return -1;
    }
    return descriptor; /* Caller owns and closes the descriptor. */
}
