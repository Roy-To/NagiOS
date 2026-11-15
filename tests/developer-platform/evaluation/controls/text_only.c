/* Negative control: CreateFileW, ReadFile, inotify_init1, inotify_add_watch. */
const char *api_names_as_text(void) {
    return "CloseHandle close CreateFileW inotify_add_watch";
}
