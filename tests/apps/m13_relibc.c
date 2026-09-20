#include <stddef.h>

extern void *malloc(size_t size);
extern void free(void *pointer);
extern int open(const char *path, int flags, ...);
extern long write(int fd, const void *bytes, size_t length);
extern long read(int fd, void *bytes, size_t length);
extern long lseek(int fd, long offset, int whence);
extern int close(int fd);
extern int pthread_mutex_init(void *mutex, const void *attr);
extern int pthread_mutex_lock(void *mutex);
extern int pthread_mutex_unlock(void *mutex);
extern int pthread_key_create(size_t *key, void (*destructor)(void *));
extern int pthread_setspecific(size_t key, const void *value);
extern const void *pthread_getspecific(size_t key);

#define O_RDWR 0x00030000
#define O_CREAT 0x02000000
#define O_TRUNC 0x04000000
#define SEEK_SET 0

int nagi_m13_relibc_test(void) {
    unsigned char *buffer = (unsigned char *)malloc(7);
    if (buffer == (void *)0) {
        return 1;
    }
    buffer[0] = 'R';
    buffer[1] = 'E';
    buffer[2] = 'L';
    buffer[3] = 'I';
    buffer[4] = 'B';
    buffer[5] = 'C';
    buffer[6] = 0;

    int fd = open("/m13-relibc.txt", O_RDWR | O_CREAT | O_TRUNC, 0);
    if (fd < 0 || write(fd, buffer, 6) != 6 || lseek(fd, 0, SEEK_SET) != 0) {
        if (fd >= 0) {
            close(fd);
        }
        free(buffer);
        return 2;
    }

    unsigned char round_trip[7] = {0};
    if (read(fd, round_trip, 6) != 6 || close(fd) != 0) {
        free(buffer);
        return 3;
    }
    for (size_t index = 0; index < 6; ++index) {
        if (round_trip[index] != buffer[index]) {
            free(buffer);
            return 4;
        }
    }

    unsigned int mutex = 0;
    size_t key = 0;
    if (pthread_mutex_init(&mutex, (const void *)0) != 0 ||
        pthread_mutex_lock(&mutex) != 0 ||
        pthread_mutex_unlock(&mutex) != 0 ||
        pthread_key_create(&key, (void (*)(void *))0) != 0 ||
        pthread_setspecific(key, buffer) != 0 ||
        pthread_getspecific(key) != buffer) {
        free(buffer);
        return 5;
    }

    free(buffer);
    return 0;
}
