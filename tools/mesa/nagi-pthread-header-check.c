#include <pthread.h>
#include <fcntl.h>

_Static_assert(sizeof(pthread_rwlock_t) == 4, "Nagi rwlock ABI must remain four bytes");
_Static_assert(O_RDONLY == 0x00010000, "Nagi O_RDONLY ABI must remain stable");
_Static_assert(O_TRUNC == 0x04000000, "Nagi O_TRUNC ABI must remain stable");

static pthread_rwlock_t nagi_header_rwlock = PTHREAD_RWLOCK_INITIALIZER;

int main(void) {
    return pthread_rwlock_init(&nagi_header_rwlock, 0);
}
