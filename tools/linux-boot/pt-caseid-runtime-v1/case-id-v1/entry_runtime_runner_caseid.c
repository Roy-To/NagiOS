#define _GNU_SOURCE
#include <errno.h>
#include <signal.h>
#include <stdint.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

struct result {
    int code;
    int child_code;
    int wait_error;
    int read_error;
    int write_error;
    int allocation_error;
    int eof;
    unsigned read_eintr;
    unsigned write_eintr;
    unsigned short_writes;
    char *text;
};

struct capture_result {
    char *text;
    size_t length;
    size_t capacity;
    int read_error;
    int write_error;
    int allocation_error;
    int eof;
    unsigned read_eintr;
    unsigned write_eintr;
    unsigned short_writes;
};

static int runner_output_error;

#ifdef ENTRY_RUNTIME_RUNNER_HOST_TEST
static int runner_capture_output_active;
#endif

#ifndef ENTRY_RUNTIME_READ
#define ENTRY_RUNTIME_READ read
#endif
#ifndef ENTRY_RUNTIME_WRITE
#define ENTRY_RUNTIME_WRITE write
#endif

static int write_all(int fd, const char *data, size_t length,
                     struct capture_result *stats) {
    while (length > 0) {
        ssize_t n = ENTRY_RUNTIME_WRITE(fd, data, length);
        if (n < 0 && errno == EINTR) {
            if (stats) stats->write_eintr++;
            continue;
        }
        if (n < 0) return errno ? errno : EIO;
        if (n == 0 || (size_t)n > length) return EIO;
        if ((size_t)n < length && stats) stats->short_writes++;
        data += n;
        length -= (size_t)n;
    }
    return 0;
}

static int runner_dprintf(int fd, const char *format, ...) {
    va_list args, copy;
    va_start(args, format);
    va_copy(copy, args);
    int length = vsnprintf(NULL, 0, format, copy);
    va_end(copy);
    if (length < 0) {
        va_end(args);
        if (fd == STDOUT_FILENO) runner_output_error = EIO;
        return EIO;
    }
    char *text = malloc((size_t)length + 1);
    if (!text) {
        va_end(args);
        if (fd == STDOUT_FILENO) runner_output_error = ENOMEM;
        return ENOMEM;
    }
    (void)vsnprintf(text, (size_t)length + 1, format, args);
    va_end(args);
    int error = write_all(fd, text, (size_t)length, NULL);
    free(text);
    if (error && fd == STDOUT_FILENO) runner_output_error = error;
    return error;
}

#define dprintf runner_dprintf

static int exit_code(int status) {
    if (WIFEXITED(status)) return WEXITSTATUS(status);
    if (WIFSIGNALED(status)) return 128 + WTERMSIG(status);
    return -1;
}

static int wait_child(pid_t child, int *code) {
    int status = 0;
    for (;;) {
        pid_t waited = waitpid(child, &status, 0);
        if (waited == child) {
            *code = exit_code(status);
            return 0;
        }
        if (waited < 0 && errno == EINTR) continue;
        *code = -127;
        return waited < 0 ? errno : ECHILD;
    }
}

static struct capture_result read_and_print(int fd) {
    struct capture_result result = { .capacity = 4096 };
    result.text = malloc(result.capacity);
    if (!result.text) {
        result.capacity = 0;
        result.allocation_error = ENOMEM;
    }
    for (;;) {
        char chunk[2048];
        ssize_t n = ENTRY_RUNTIME_READ(fd, chunk, sizeof(chunk));
        if (n < 0 && errno == EINTR) {
            result.read_eintr++;
            continue;
        }
        if (n < 0) {
            result.read_error = errno ? errno : EIO;
            break;
        }
        if (n == 0) {
            result.eof = 1;
            break;
        }
        if ((size_t)n > sizeof(chunk)) {
            result.read_error = EIO;
            break;
        }
        if (result.text) {
            size_t needed = result.length + (size_t)n + 1;
            if (needed > result.capacity) {
                size_t next = result.capacity;
                while (next < needed && next <= SIZE_MAX / 2) next *= 2;
                if (next < needed) next = needed;
                char *grown = realloc(result.text, next);
                if (!grown) {
                    result.allocation_error = ENOMEM;
                    free(result.text);
                    result.text = NULL;
                    result.length = 0;
                    result.capacity = 0;
                } else {
                    result.text = grown;
                    result.capacity = next;
                }
            }
            if (result.text) {
                memcpy(result.text + result.length, chunk, (size_t)n);
                result.length += (size_t)n;
            }
        }
#ifdef ENTRY_RUNTIME_RUNNER_HOST_TEST
        runner_capture_output_active = 1;
#endif
        int write_error = write_all(STDOUT_FILENO, chunk, (size_t)n, &result);
#ifdef ENTRY_RUNTIME_RUNNER_HOST_TEST
        runner_capture_output_active = 0;
#endif
        if (write_error && !result.write_error) result.write_error = write_error;
    }
    if (result.text) result.text[result.length] = '\0';
    return result;
}

static struct result run_case(const char *label, char *const argv[]) {
    dprintf(STDOUT_FILENO, "CASE_BEGIN label=%s argv0=%s\n", label, argv[0]);
    int output_pipe[2];
    if (pipe(output_pipe) != 0) {
        dprintf(STDOUT_FILENO, "CASE_END label=%s pipe_error=%d\n", label, errno);
        return (struct result){ .code = -127, .child_code = -127, .text = strdup("") };
    }
    pid_t child = fork();
    if (child == 0) {
        close(output_pipe[0]);
        dup2(output_pipe[1], STDOUT_FILENO);
        dup2(output_pipe[1], STDERR_FILENO);
        close(output_pipe[1]);
        execv(argv[0], argv);
        dprintf(STDERR_FILENO, "execv failed: %s errno=%d\n", argv[0], errno);
        _exit(127);
    }
    close(output_pipe[1]);
    if (child < 0) {
        close(output_pipe[0]);
        dprintf(STDOUT_FILENO, "CASE_END label=%s fork_error=%d\n", label, errno);
        return (struct result){ .code = -127, .child_code = -127, .text = strdup("") };
    }
    struct capture_result capture = read_and_print(output_pipe[0]);
    close(output_pipe[0]);
    int child_code = -127;
    int wait_error = wait_child(child, &child_code);
    int capture_error = capture.read_error || capture.write_error ||
                        capture.allocation_error || !capture.eof || !capture.text;
    int code = wait_error || capture_error ? -127 : child_code;
    if (wait_error) {
        dprintf(STDOUT_FILENO, "CASE_END label=%s wait_error=%d child_exit=unverified read_error=%d write_error=%d allocation_error=%d eof=%d\n",
                label, wait_error, capture.read_error, capture.write_error,
                capture.allocation_error, capture.eof);
    } else if (capture_error) {
        dprintf(STDOUT_FILENO, "CASE_END label=%s child_exit=%d result=unverified read_error=%d write_error=%d allocation_error=%d eof=%d read_eintr=%u write_eintr=%u short_writes=%u\n",
                label, child_code, capture.read_error, capture.write_error,
                capture.allocation_error, capture.eof, capture.read_eintr,
                capture.write_eintr, capture.short_writes);
    } else {
        dprintf(STDOUT_FILENO, "CASE_END label=%s exit=%d eof=%d read_eintr=%u write_eintr=%u short_writes=%u\n",
                label, code, capture.eof, capture.read_eintr,
                capture.write_eintr, capture.short_writes);
    }
    return (struct result){
        .code = code,
        .child_code = wait_error ? -127 : child_code,
        .wait_error = wait_error,
        .read_error = capture.read_error,
        .write_error = capture.write_error,
        .allocation_error = capture.allocation_error,
        .eof = capture.eof,
        .read_eintr = capture.read_eintr,
        .write_eintr = capture.write_eintr,
        .short_writes = capture.short_writes,
        .text = capture.text
    };
}

static int parse_note_id(const char *text, char *destination, size_t capacity) {
    if (!text || capacity == 0) return 0;
    const char *found = strstr(text, "note_id=");
    if (!found) return 0;
    found += strlen("note_id=");
    size_t length = 0;
    while (found[length] && found[length] != ' ' && found[length] != '\n' &&
           found[length] != '\r' && length + 1 < capacity) {
        destination[length] = found[length];
        length++;
    }
    destination[length] = '\0';
    return length > 0;
}

struct entry_case_contract {
    const char *entry;
    size_t count;
    const char *ids[4];
};

static const struct entry_case_contract ENTRY_CASE_CONTRACTS[] = {
    { "linux_model_clock_consumer", 1, { "linux_model_clock_consumer" } },
    { "notes-host-preview", 2, { "notes-host-preview.save", "notes-host-preview.read" } },
    { "in_process_echo", 1, { "in_process_echo" } },
    { "sdk-development-smoke", 1, { "sdk-development-smoke" } },
    { "installed_service_clock_smoke", 1, { "installed_service_clock_smoke" } },
    { "notes-event-bridge-smoke", 2, { "notes-event-bridge-smoke.save", "notes-event-bridge-smoke.read" } },
    { "desktop-host-preview-smoke", 1, { "desktop-host-preview-smoke" } },
    { "desktop-evdev-stream-smoke", 4, {
        "desktop-evdev-stream-smoke.create-empty",
        "desktop-evdev-stream-smoke.producer",
        "desktop-evdev-stream-smoke.stdin",
        "desktop-evdev-stream-smoke.read"
    } },
    { "desktop-evdev-integrity-smoke", 3, {
        "desktop-evdev-integrity-smoke.integrity-seed-v1",
        "desktop-evdev-integrity-smoke.integrity-save",
        "desktop-evdev-integrity-smoke.integrity-check"
    } },
};

static const struct entry_case_contract *find_entry_case_contract(const char *name) {
    for (size_t i = 0; i < sizeof(ENTRY_CASE_CONTRACTS) / sizeof(ENTRY_CASE_CONTRACTS[0]); i++) {
        if (strcmp(name, ENTRY_CASE_CONTRACTS[i].entry) == 0) return &ENTRY_CASE_CONTRACTS[i];
    }
    return NULL;
}

static int entry_result(const char *name, const int *codes, size_t count) {
    const struct entry_case_contract *contract = find_entry_case_contract(name);
    if (!contract || contract->count != count || count > 4) {
        runner_output_error = runner_output_error ? runner_output_error : EINVAL;
        dprintf(STDOUT_FILENO, "CASE_RESULT_MAP_ERROR entry=%s count=%zu\n", name, count);
        return 1;
    }
    int all_expected = count > 0 && runner_output_error == 0;
    for (size_t i = 0; i < count; i++) {
        int case_pass = codes[i] == 0 && runner_output_error == 0;
        int marker_error = dprintf(STDOUT_FILENO,
            "CASE_RESULT id=%s exit=%d expected=0 outcome=%s\n",
            contract->ids[i], codes[i], case_pass ? "PASS" : "FAIL");
        if (codes[i] != 0 || marker_error) all_expected = 0;
    }
    dprintf(STDOUT_FILENO, "ENTRY_RESULT name=%s cases=", name);
    for (size_t i = 0; i < count; i++) {
        dprintf(STDOUT_FILENO, "%s%d/expected0", i ? "," : "", codes[i]);
    }
    dprintf(STDOUT_FILENO, " outcome=%s\n", all_expected ? "PASS" : "FAIL");
    return all_expected && runner_output_error == 0 ? 0 : 1;
}

static void split_stream_pipeline(const char *root, const char *id, int *producer_code,
                                  int *consumer_code) {
    int records[2], output_pipe[2];
    dprintf(STDOUT_FILENO, "CASE_BEGIN label=desktop-evdev-stream-smoke.producer-stdin-pipeline mode=split-save\n");
    if (pipe(records) != 0) {
        *producer_code = *consumer_code = -127;
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-evdev-stream-smoke.pipeline pipe_error=%d\n", errno);
        return;
    }
    if (pipe(output_pipe) != 0) {
        int error = errno;
        close(records[0]);
        close(records[1]);
        *producer_code = *consumer_code = -127;
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-evdev-stream-smoke.pipeline pipe_error=%d\n", error);
        return;
    }

    pid_t producer = fork();
    if (producer == 0) {
        close(records[0]);
        close(output_pipe[0]);
        close(output_pipe[1]);
        dup2(records[1], STDOUT_FILENO);
        close(records[1]);
        char *argv[] = { "/bin/desktop-evdev-stream-smoke", "producer", "x86_64", "split-save", NULL };
        execv(argv[0], argv);
        dprintf(STDERR_FILENO, "producer execv failed errno=%d\n", errno);
        _exit(127);
    }
    if (producer < 0) {
        close(records[0]); close(records[1]); close(output_pipe[0]); close(output_pipe[1]);
        *producer_code = *consumer_code = -127;
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-evdev-stream-smoke.pipeline fork_producer_error=%d\n", errno);
        return;
    }

    pid_t consumer = fork();
    if (consumer == 0) {
        close(records[1]);
        close(output_pipe[0]);
        dup2(records[0], STDIN_FILENO);
        dup2(output_pipe[1], STDOUT_FILENO);
        dup2(output_pipe[1], STDERR_FILENO);
        close(records[0]);
        close(output_pipe[1]);
        char *argv[] = { "/bin/desktop-evdev-stream-smoke", "stdin", (char *)root,
                         (char *)id, "x86_64", NULL };
        execv(argv[0], argv);
        dprintf(STDERR_FILENO, "stdin consumer execv failed errno=%d\n", errno);
        _exit(127);
    }
    close(records[0]);
    close(records[1]);
    close(output_pipe[1]);
    if (consumer < 0) {
        close(output_pipe[0]);
        int fork_error = errno;
        kill(producer, SIGTERM);
        int killed_code = -127;
        int wait_error = wait_child(producer, &killed_code);
        *producer_code = *consumer_code = -127;
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-evdev-stream-smoke.pipeline fork_consumer_error=%d producer_wait_error=%d producer_exit=%d\n", fork_error, wait_error, killed_code);
        return;
    }
    struct capture_result capture = read_and_print(output_pipe[0]);
    close(output_pipe[0]);
    int producer_exit = -127, consumer_exit = -127;
    int producer_wait_error = wait_child(producer, &producer_exit);
    int consumer_wait_error = wait_child(consumer, &consumer_exit);
    int capture_error = capture.read_error || capture.write_error ||
                        capture.allocation_error || !capture.eof || !capture.text;
    *producer_code = producer_wait_error ? -127 : producer_exit;
    *consumer_code = consumer_wait_error || capture_error ? -127 : consumer_exit;
    if (producer_wait_error) {
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-evdev-stream-smoke.producer wait_error=%d child_exit=unverified\n", producer_wait_error);
    } else {
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-evdev-stream-smoke.producer exit=%d\n", producer_exit);
    }
    if (consumer_wait_error) {
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-evdev-stream-smoke.stdin wait_error=%d child_exit=unverified\n", consumer_wait_error);
    } else if (capture_error) {
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-evdev-stream-smoke.stdin child_exit=%d result=unverified read_error=%d write_error=%d allocation_error=%d eof=%d read_eintr=%u write_eintr=%u short_writes=%u\n",
                consumer_exit, capture.read_error, capture.write_error,
                capture.allocation_error, capture.eof, capture.read_eintr,
                capture.write_eintr, capture.short_writes);
    } else {
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-evdev-stream-smoke.stdin exit=%d eof=%d read_eintr=%u write_eintr=%u short_writes=%u\n",
                *consumer_code, capture.eof, capture.read_eintr,
                capture.write_eintr, capture.short_writes);
    }
    free(capture.text);
}

#ifndef ENTRY_RUNTIME_RUNNER_HOST_TEST
int main(void) {
    int runner_failed = 0;
    mkdir("/tmp/nagi-runtime-v5", 0777);
    dprintf(STDOUT_FILENO, "RUNTIME_V5_RUNNER_BEGIN pid=1 scope=task-2-initramfs-only\n");

    char *clock_argv[] = { "/bin/linux_model_clock_consumer", NULL };
    struct result clock = run_case("linux_model_clock_consumer", clock_argv);
    int clock_codes[] = { clock.code };
    runner_failed |= entry_result("linux_model_clock_consumer", clock_codes, 1);
    free(clock.text);

    const char *host_root = "/tmp/nagi-runtime-v5/host-preview";
    char *host_save_argv[] = { "/bin/notes-host-preview", "save", (char *)host_root,
                               "Linux runtime note", "saved in isolated guest", NULL };
    struct result host_save = run_case("notes-host-preview.save", host_save_argv);
    char host_id[128] = "";
    int have_host_id = parse_note_id(host_save.text, host_id, sizeof(host_id));
    int host_read_code = -127;
    if (have_host_id) {
        char *host_read_argv[] = { "/bin/notes-host-preview", "read", (char *)host_root, host_id, NULL };
        struct result host_read = run_case("notes-host-preview.read", host_read_argv);
        host_read_code = host_read.code;
        free(host_read.text);
    } else {
        dprintf(STDOUT_FILENO, "CASE_END label=notes-host-preview.read skipped=no-note-id\n");
    }
    int host_codes[] = { host_save.code, host_read_code };
    runner_failed |= entry_result("notes-host-preview", host_codes, 2);
    free(host_save.text);

    char *ipc_argv[] = { "/bin/in_process_echo", NULL };
    struct result ipc = run_case("in_process_echo", ipc_argv);
    int ipc_codes[] = { ipc.code };
    runner_failed |= entry_result("in_process_echo", ipc_codes, 1);
    free(ipc.text);

    char *sdk_argv[] = { "/bin/sdk-development-smoke", NULL };
    struct result sdk = run_case("sdk-development-smoke", sdk_argv);
    int sdk_codes[] = { sdk.code };
    runner_failed |= entry_result("sdk-development-smoke", sdk_codes, 1);
    free(sdk.text);

    char *installed_clock_argv[] = { "/bin/installed_service_clock_smoke", NULL };
    struct result installed_clock = run_case("installed_service_clock_smoke", installed_clock_argv);
    int installed_clock_codes[] = { installed_clock.code };
    runner_failed |= entry_result("installed_service_clock_smoke", installed_clock_codes, 1);
    free(installed_clock.text);

    const char *bridge_root = "/tmp/nagi-runtime-v5/event-bridge";
    char *bridge_save_argv[] = { "/bin/notes-event-bridge-smoke", "save", (char *)bridge_root,
                                 "Bridge runtime note", "30", "48", NULL };
    struct result bridge_save = run_case("notes-event-bridge-smoke.save", bridge_save_argv);
    char bridge_id[128] = "";
    int have_bridge_id = parse_note_id(bridge_save.text, bridge_id, sizeof(bridge_id));
    int bridge_read_code = -127;
    if (have_bridge_id) {
        char *bridge_read_argv[] = { "/bin/notes-event-bridge-smoke", "read", (char *)bridge_root,
                                     bridge_id, NULL };
        struct result bridge_read = run_case("notes-event-bridge-smoke.read", bridge_read_argv);
        bridge_read_code = bridge_read.code;
        free(bridge_read.text);
    } else {
        dprintf(STDOUT_FILENO, "CASE_END label=notes-event-bridge-smoke.read skipped=no-note-id\n");
    }
    int bridge_codes[] = { bridge_save.code, bridge_read_code };
    runner_failed |= entry_result("notes-event-bridge-smoke", bridge_codes, 2);
    free(bridge_save.text);

    int desktop_preview_code = -127;
    if (have_host_id) {
        char *desktop_argv[] = { "/bin/desktop-host-preview-smoke", (char *)host_root, host_id, NULL };
        struct result desktop = run_case("desktop-host-preview-smoke", desktop_argv);
        desktop_preview_code = desktop.code;
        free(desktop.text);
    } else {
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-host-preview-smoke skipped=no-note-id\n");
    }
    int desktop_codes[] = { desktop_preview_code };
    runner_failed |= entry_result("desktop-host-preview-smoke", desktop_codes, 1);

    const char *evdev_root = "/tmp/nagi-runtime-v5/evdev-stream";
    char *evdev_create_argv[] = { "/bin/desktop-evdev-stream-smoke", "create-empty",
                                  (char *)evdev_root, "Split stream runtime note", NULL };
    struct result evdev_create = run_case("desktop-evdev-stream-smoke.create-empty", evdev_create_argv);
    char evdev_id[128] = "";
    int have_evdev_id = parse_note_id(evdev_create.text, evdev_id, sizeof(evdev_id));
    int producer_code = -127, stream_code = -127, evdev_read_code = -127;
    if (have_evdev_id) {
        split_stream_pipeline(evdev_root, evdev_id, &producer_code, &stream_code);
        char *evdev_read_argv[] = { "/bin/desktop-evdev-stream-smoke", "read", (char *)evdev_root,
                                    evdev_id, NULL };
        struct result evdev_read = run_case("desktop-evdev-stream-smoke.read", evdev_read_argv);
        evdev_read_code = evdev_read.code;
        free(evdev_read.text);
    } else {
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-evdev-stream-smoke.pipeline skipped=no-note-id\n");
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-evdev-stream-smoke.read skipped=no-note-id\n");
    }
    int evdev_codes[] = { evdev_create.code, producer_code, stream_code, evdev_read_code };
    runner_failed |= entry_result("desktop-evdev-stream-smoke", evdev_codes, 4);
    free(evdev_create.text);

    const char *integrity_root = "/tmp/nagi-runtime-v5/integrity";
    int integrity_seed_code = -127, integrity_save_code = -127, integrity_check_code = -127;
    if (have_evdev_id) {
        char *seed_argv[] = { "/bin/desktop-evdev-integrity-smoke", "integrity-seed-v1",
                              (char *)integrity_root, evdev_id, "Integrity v1 note", NULL };
        struct result seed = run_case("desktop-evdev-integrity-smoke.integrity-seed-v1", seed_argv);
        integrity_seed_code = seed.code;
        free(seed.text);
        char *save_argv[] = { "/bin/desktop-evdev-integrity-smoke", "integrity-save",
                              (char *)integrity_root, evdev_id, NULL };
        struct result save = run_case("desktop-evdev-integrity-smoke.integrity-save", save_argv);
        integrity_save_code = save.code;
        free(save.text);
        char *check_argv[] = { "/bin/desktop-evdev-integrity-smoke", "integrity-check",
                               (char *)integrity_root, evdev_id, NULL };
        struct result check = run_case("desktop-evdev-integrity-smoke.integrity-check", check_argv);
        integrity_check_code = check.code;
        free(check.text);
    } else {
        dprintf(STDOUT_FILENO, "CASE_END label=desktop-evdev-integrity-smoke skipped=no-note-id\n");
    }
    int integrity_codes[] = { integrity_seed_code, integrity_save_code, integrity_check_code };
    runner_failed |= entry_result("desktop-evdev-integrity-smoke", integrity_codes, 3);

    runner_failed |= runner_output_error != 0;
    int final_status = runner_failed ? 1 : 0;
    dprintf(STDOUT_FILENO, "RUNNER_DONE entries=9 status=%d output_error=%d scope=isolated-linux-userspace smoke_only=true\n",
            final_status, runner_output_error);
    return final_status || runner_output_error ? 1 : 0;
}
#endif

#undef dprintf
