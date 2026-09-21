// Nagi-owned freestanding C++ ABI boundary for the M17 guest.
//
// This translation unit deliberately includes no host or C++ standard
// library headers. It connects the ABI-required allocation and stack-protector
// symbols to the real Nagi POSIX allocator and abort path.

using nagi_size_t = __SIZE_TYPE__;
using nagi_uintptr_t = __UINTPTR_TYPE__;

extern "C" void *nagi_posix_malloc(nagi_size_t size);
extern "C" void nagi_posix_free(void *pointer);
extern "C" int nagi_posix_sleep_ns(nagi_uintptr_t duration);
extern "C" [[noreturn]] void abort();

namespace std {
struct nothrow_t;
enum class align_val_t : nagi_size_t;
} // namespace std

extern "C" nagi_uintptr_t __stack_chk_guard = 0xd048c37519fcadfeULL;

extern "C" [[noreturn]] void __stack_chk_fail() {
    abort();
}

// libc++ uses this freestanding diagnostic entrypoint for invariant failures
// even when exceptions are disabled. Keep the ABI real and terminate the
// guest through Nagi's process boundary; do not import a host libc++abi.
extern "C" [[noreturn]] void nagi_cxx_verbose_abort(const char *, ...)
    __asm__("_ZNSt3__121__libcpp_verbose_abortEPKcz");

extern "C" [[noreturn]] void nagi_cxx_verbose_abort(const char *, ...) {
    abort();
}

static void *nagi_allocate(nagi_size_t size) {
    void *pointer = nagi_posix_malloc(size == 0 ? 1 : size);
    if (pointer == nullptr) {
        abort();
    }
    return pointer;
}

void *operator new(nagi_size_t size) {
    return nagi_allocate(size);
}

void *operator new[](nagi_size_t size) {
    return nagi_allocate(size);
}

void *operator new(nagi_size_t size, const std::nothrow_t &) noexcept {
    return nagi_posix_malloc(size == 0 ? 1 : size);
}

void *operator new[](nagi_size_t size, const std::nothrow_t &) noexcept {
    return nagi_posix_malloc(size == 0 ? 1 : size);
}

void *operator new(nagi_size_t size, std::align_val_t) {
    return nagi_allocate(size);
}

void *operator new[](nagi_size_t size, std::align_val_t) {
    return nagi_allocate(size);
}

void *operator new(nagi_size_t size, std::align_val_t, const std::nothrow_t &) noexcept {
    return nagi_posix_malloc(size == 0 ? 1 : size);
}

void *operator new[](nagi_size_t size, std::align_val_t, const std::nothrow_t &) noexcept {
    return nagi_posix_malloc(size == 0 ? 1 : size);
}

void operator delete(void *pointer) noexcept {
    nagi_posix_free(pointer);
}

void operator delete[](void *pointer) noexcept {
    nagi_posix_free(pointer);
}

void operator delete(void *pointer, nagi_size_t) noexcept {
    nagi_posix_free(pointer);
}

void operator delete[](void *pointer, nagi_size_t) noexcept {
    nagi_posix_free(pointer);
}

void operator delete(void *pointer, const std::nothrow_t &) noexcept {
    nagi_posix_free(pointer);
}

void operator delete[](void *pointer, const std::nothrow_t &) noexcept {
    nagi_posix_free(pointer);
}

void operator delete(void *pointer, std::align_val_t) noexcept {
    nagi_posix_free(pointer);
}

void operator delete[](void *pointer, std::align_val_t) noexcept {
    nagi_posix_free(pointer);
}

void operator delete(void *pointer, nagi_size_t, std::align_val_t) noexcept {
    nagi_posix_free(pointer);
}

void operator delete[](void *pointer, nagi_size_t, std::align_val_t) noexcept {
    nagi_posix_free(pointer);
}

// libc++'s freestanding Nagi path still references this concrete duration
// overload. Keep the ABI entrypoint in the Nagi-owned runtime and delegate to
// the existing GuestClock-backed POSIX sleep boundary.
extern "C" void nagi_cxx_sleep_for(const long long *duration)
    __asm__("_ZNSt3__111this_thread9sleep_forERKNS_6chrono8durationIxNS2_5ratioILl1ELl1000000000EEEE");

extern "C" void nagi_cxx_sleep_for(const long long *duration) {
    if (duration != nullptr && *duration > 0) {
        (void)nagi_posix_sleep_ns(static_cast<nagi_uintptr_t>(*duration));
    }
}
