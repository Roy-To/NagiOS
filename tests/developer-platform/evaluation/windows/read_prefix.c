/* Static inspection only: do not build or execute on this host. */
#include <windows.h>
BOOL read_prefix(const wchar_t *path, void *buffer, DWORD capacity, DWORD *size) {
    HANDLE file = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL,
                             OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE) return FALSE;
    BOOL result = ReadFile(file, buffer, capacity, size, NULL);
    CloseHandle(file);
    return result;
}
