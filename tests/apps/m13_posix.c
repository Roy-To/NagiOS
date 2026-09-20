#include <stddef.h>

extern void *nagi_posix_malloc(size_t size);
extern void nagi_posix_free(void *pointer);
extern long nagi_posix_write(int fd, const unsigned char *bytes, size_t length);
extern long nagi_posix_fork(void);
extern void *nagi_posix_mmap(size_t length, int protection);
extern int nagi_posix_mprotect(void *address, size_t length, int protection);
extern int nagi_posix_munmap(void *address, size_t length);
extern int nagi_posix_poll(int timeout_ms);
extern int nagi_posix_sleep_ns(unsigned long long duration);
extern int socket(int domain, int type, int protocol);
extern int connect(int fd, const void *address, size_t address_length);
extern long send(int fd, const void *bytes, size_t length, int flags);
extern long recv(int fd, void *bytes, size_t length, int flags);
extern int close(int fd);
extern int nagi_posix_default_gateway(void *address);
extern int nagi_posix_resolve_ipv4(const char *name, void *address);
extern int nagi_posix_poll_fds(void *fds, size_t count, int timeout_ms);
typedef unsigned long nagi_pthread_t;
typedef unsigned long nagi_pthread_key_t;
extern int pthread_key_create(nagi_pthread_key_t *key, void *destructor);
extern int pthread_key_delete(nagi_pthread_key_t key);
extern void *pthread_getspecific(nagi_pthread_key_t key);
extern int pthread_setspecific(nagi_pthread_key_t key, const void *value);
extern int pthread_create(nagi_pthread_t *thread, const void *attributes,
                          void *(*start)(void *), void *argument);
extern int pthread_join(nagi_pthread_t thread, void **result);
extern nagi_pthread_t pthread_self(void);
struct nagi_spawn_request {
    size_t entry;
    size_t argument;
    unsigned long long parent_rights;
    unsigned long long requested_rights;
};
extern int nagi_posix_spawn_entry(const struct nagi_spawn_request *request,
                                  unsigned long long *thread);
extern int nagi_posix_spawn_wait(unsigned long long thread,
                                 unsigned long long *result);

#define AF_INET 2
#define SOCK_STREAM 1
#define POLLIN 0x0001
#define POLLOUT 0x0004

struct nagi_ipv4_address {
    unsigned char octets[4];
};

struct nagi_sockaddr_ipv4 {
    unsigned short family;
    unsigned short port_be;
    unsigned char address[4];
};

struct nagi_pollfd {
    int fd;
    short events;
    short revents;
};

static unsigned short to_be16(unsigned short value) {
    return (unsigned short)((value >> 8) | (value << 8));
}

static int contains_bytes(const unsigned char *bytes, size_t length,
                          const unsigned char *needle, size_t needle_length) {
    size_t start;
    if (needle_length == 0 || length < needle_length) {
        return 0;
    }
    for (start = 0; start <= length - needle_length; ++start) {
        size_t index;
        for (index = 0; index < needle_length; ++index) {
            if (bytes[start + index] != needle[index]) {
                break;
            }
        }
        if (index == needle_length) {
            return 1;
        }
    }
    return 0;
}

static int socket_dns_http_test(void) {
    struct nagi_ipv4_address gateway = {{0, 0, 0, 0}};
    struct nagi_ipv4_address resolved = {{0, 0, 0, 0}};
    struct nagi_sockaddr_ipv4 endpoint;
    struct nagi_pollfd pollfd;
    // This probe runs in the one serialized bootstrap process. Keep the
    // large response outside the small initial user stack while retaining
    // the real socket/recv path.
    static unsigned char response[1536];
    const unsigned char request[] = "GET /nagi-m12.txt HTTP/1.0\r\nHost: nagi\r\nConnection: close\r\n\r\n";
    const unsigned char expected[] = "NAGI_M12_HTTP_FIXTURE_PASS";
    size_t response_length = 0;
    int fd;
    if (nagi_posix_default_gateway(&gateway) != 0) {
        return 10;
    }
    if (nagi_posix_resolve_ipv4("example.com", &resolved) != 0 ||
        (resolved.octets[0] == 0 && resolved.octets[1] == 0 &&
         resolved.octets[2] == 0 && resolved.octets[3] == 0)) {
        return 10;
    }
    endpoint.family = AF_INET;
    endpoint.port_be = to_be16(18080);
    endpoint.address[0] = gateway.octets[0];
    endpoint.address[1] = gateway.octets[1];
    endpoint.address[2] = gateway.octets[2];
    endpoint.address[3] = gateway.octets[3];
    fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0 || connect(fd, &endpoint, sizeof(endpoint)) != 0) {
        return 11;
    }
    pollfd.fd = fd;
    pollfd.events = POLLOUT;
    pollfd.revents = 0;
    if (nagi_posix_poll_fds(&pollfd, 1, 1000) != 1 || !(pollfd.revents & POLLOUT) ||
        send(fd, request, sizeof(request) - 1, 0) != (long)(sizeof(request) - 1)) {
        close(fd);
        return 12;
    }
    pollfd.events = POLLIN;
    pollfd.revents = 0;
    if (nagi_posix_poll_fds(&pollfd, 1, 2000) != 1 || !(pollfd.revents & POLLIN)) {
        close(fd);
        return 13;
    }
    while (response_length < sizeof(response) &&
           !contains_bytes(response, response_length, expected, sizeof(expected) - 1)) {
        long count = recv(fd, response + response_length,
                          sizeof(response) - response_length, 0);
        if (count <= 0) {
            break;
        }
        response_length += (size_t)count;
    }
    close(fd);
    return contains_bytes(response, response_length, expected, sizeof(expected) - 1) ? 0 : 14;
}

struct nagi_timespec {
    long tv_sec;
    long tv_nsec;
};
extern int clock_gettime(int clock_id, struct nagi_timespec *output);

static int elapsed_time_test(void) {
    struct nagi_timespec before = {0, 0};
    struct nagi_timespec after = {0, 0};
    struct nagi_timespec realtime = {0, 0};
    if (clock_gettime(4, &before) != 0 || clock_gettime(1, &realtime) != 0) {
        return 10;
    }
    if (realtime.tv_sec < 0 || realtime.tv_nsec < 0 || realtime.tv_nsec >= 1000000000) {
        return 11;
    }
    if (nagi_posix_sleep_ns(20ULL * 1000ULL * 1000ULL) != 0 ||
        clock_gettime(4, &after) != 0) {
        return 12;
    }
    long before_ns = before.tv_sec * 1000000000L + before.tv_nsec;
    long after_ns = after.tv_sec * 1000000000L + after.tv_nsec;
    return after_ns >= before_ns + 10 * 1000 * 1000 ? 0 : 13;
}

static int mapping_test(void) {
    unsigned char *mapping = (unsigned char *)nagi_posix_mmap(4096, 3);
    if (mapping == (void *)0) {
        return 20;
    }
    mapping[0] = 'M';
    mapping[4095] = 'P';
    if (mapping[0] != 'M' || mapping[4095] != 'P' ||
        nagi_posix_mprotect(mapping, 4096, 1) != 0 ||
        nagi_posix_mprotect(mapping, 4096, 3) != 0 ||
        nagi_posix_munmap(mapping, 4096) != 0) {
        return 21;
    }
    return 0;
}

static nagi_pthread_key_t thread_key;
static int parent_tls_value;
static int child_tls_value;

static void *thread_tls_worker(void *argument) {
    (void)argument;
    if (pthread_self() != 1 || pthread_getspecific(thread_key) != (void *)0) {
        return (void *)0x1001;
    }
    if (pthread_setspecific(thread_key, &child_tls_value) != 0 ||
        pthread_getspecific(thread_key) != &child_tls_value) {
        return (void *)0x1002;
    }
    return (void *)0xc0de;
}

static int thread_tls_test(void) {
    nagi_pthread_t thread = 0;
    void *result = (void *)0;
    if (pthread_key_create(&thread_key, (void *)0) != 0 ||
        pthread_setspecific(thread_key, &parent_tls_value) != 0 ||
        pthread_getspecific(thread_key) != &parent_tls_value) {
        return 30;
    }
    if (pthread_create(&thread, (void *)0, thread_tls_worker, (void *)0) != 0 ||
        thread != 1) {
        pthread_key_delete(thread_key);
        return 31;
    }
    if (pthread_getspecific(thread_key) != &parent_tls_value ||
        pthread_join(thread, &result) != 0 || result != (void *)0xc0de ||
        pthread_getspecific(thread_key) != &parent_tls_value) {
        pthread_key_delete(thread_key);
        return 32;
    }
    if (pthread_key_delete(thread_key) != 0) {
        return 33;
    }
    return 0;
}

static unsigned long long native_spawn_entry(size_t argument,
                                             unsigned long long rights) {
    return argument == 0x55 && rights == 0x05 ? 0x5a : 0xffff;
}

static int native_spawn_test(void) {
    struct nagi_spawn_request request = {
        (size_t)&native_spawn_entry, 0x55, 0x07, 0x05
    };
    unsigned long long thread = 0;
    unsigned long long result = 0;
    if (nagi_posix_spawn_entry(&request, &thread) != 0) {
        return 42;
    }
    if (thread != 1) {
        return 43;
    }
    if (nagi_posix_spawn_wait(thread, &result) != 0) {
        return 44;
    }
    if (result != 0x5a) {
        return 45;
    }
    request.requested_rights = 0x08;
    if (nagi_posix_spawn_entry(&request, &thread) == 0) {
        return 41;
    }
    return 0;
}

int nagi_m13_c_posix_test(void) {
    unsigned char *buffer = (unsigned char *)nagi_posix_malloc(8);
    if (buffer == (void *)0) {
        return 1;
    }
    buffer[0] = 'C';
    buffer[1] = 'P';
    buffer[2] = 'O';
    buffer[3] = 'S';
    buffer[4] = 'I';
    buffer[5] = 'X';
    if (nagi_posix_write(1, buffer, 6) != 6) {
        nagi_posix_free(buffer);
        return 2;
    }
    nagi_posix_free(buffer);
    if (nagi_posix_fork() != -1) {
        return 3;
    }
    if (mapping_test() != 0) {
        return 4;
    }
    if (thread_tls_test() != 0) {
        return 8;
    }
    if (native_spawn_test() != 0) {
        return 9;
    }
    if (elapsed_time_test() != 0) {
        return 5;
    }
    if (nagi_posix_poll(0) != 0) {
        return 6;
    }
    if (socket_dns_http_test() != 0) {
        return 7;
    }
    return 0;
}
