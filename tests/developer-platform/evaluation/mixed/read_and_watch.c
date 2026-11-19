/* Static inspection only: do not build or execute on this host. */
#ifdef _WIN32
#include <windows.h>
BOOL read_input(const wchar_t *path, void *buffer, DWORD capacity, DWORD *size) {
    HANDLE file = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL,
                             OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE) return FALSE;
    BOOL result = ReadFile(file, buffer, capacity, size, NULL);
    CancelIoEx(file, NULL);
    CloseHandle(file);
    return result;
}
#else
#define _GNU_SOURCE
#include <sys/inotify.h>
#include <unistd.h>
int watch_input(const char *path) {
    int descriptor = inotify_init1(IN_NONBLOCK | IN_CLOEXEC);
    if (descriptor < 0) return -1;
    int readable = euidaccess(path, R_OK);
    if (readable < 0 || inotify_add_watch(descriptor, path, IN_MODIFY) < 0) {
        close(descriptor);
        return -1;
    }
    return descriptor;
}
#endif
