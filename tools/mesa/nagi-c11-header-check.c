#include <stdatomic.h>
#include <termios.h>

_Static_assert(sizeof(struct termios) == 48, "Nagi termios ABI must match relibc");

int nagi_c11_header_check(void) {
    _Atomic(unsigned int) value = 0;
    unsigned int expected = 0;
    atomic_load(&value);
    atomic_compare_exchange_weak(&value, &expected, 1);
    return (int)sizeof(struct termios);
}
