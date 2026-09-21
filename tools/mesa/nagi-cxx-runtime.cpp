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

// C++11 function-local statics use the Itanium ABI guard protocol. The
// freestanding Nagi image does not link a host libc++abi, so provide the
// protocol at the same boundary as the other C++ runtime entrypoints. Bit 0
// means initialized and bit 8 means that one thread owns initialization. The
// atomic operations are emitted inline for the x86-64 target; no libatomic or
// host synchronization provider is imported.
using nagi_guard_t = unsigned long long;

static constexpr nagi_guard_t NAGI_GUARD_INITIALIZED = 1;
static constexpr nagi_guard_t NAGI_GUARD_IN_PROGRESS = 1ULL << 8;

static void nagi_guard_pause() {
    __asm__ volatile("pause" ::: "memory");
}

extern "C" int __cxa_guard_acquire(nagi_guard_t *guard) {
    if (guard == nullptr) {
        abort();
    }

    for (;;) {
        const nagi_guard_t state =
            __atomic_load_n(guard, __ATOMIC_ACQUIRE);
        if ((state & NAGI_GUARD_INITIALIZED) != 0) {
            return 0;
        }

        if ((state & NAGI_GUARD_IN_PROGRESS) == 0) {
            nagi_guard_t expected = state;
            if (__atomic_compare_exchange_n(
                    guard, &expected, state | NAGI_GUARD_IN_PROGRESS, false,
                    __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
                return 1;
            }
            continue;
        }

        // Another Nagi thread owns initialization. __cxa_guard_abort clears
        // this bit so a later iteration may retry after an exception path.
        nagi_guard_pause();
    }
}

extern "C" void __cxa_guard_release(nagi_guard_t *guard) {
    if (guard == nullptr) {
        abort();
    }
    __atomic_store_n(guard, NAGI_GUARD_INITIALIZED, __ATOMIC_RELEASE);
}

extern "C" void __cxa_guard_abort(nagi_guard_t *guard) {
    if (guard == nullptr) {
        abort();
    }
    __atomic_store_n(guard, 0, __ATOMIC_RELEASE);
}

// libc++ uses this freestanding diagnostic entrypoint for invariant failures
// even when exceptions are disabled. Keep the ABI real and terminate the
// guest through Nagi's process boundary; do not import a host libc++abi.
extern "C" [[noreturn]] void nagi_cxx_verbose_abort(const char *, ...)
    __asm__("_ZNSt3__122__libcpp_verbose_abortEPKcz");

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

// Nagi's M17 C++ boundary deliberately has no host locale database. The
// pinned libc++ headers still require the stable classic-locale identity and
// the ctype<char> locale-id object when stream machinery is instantiated.
// Keep those ABI objects in Nagi-owned storage. Unicode and locale-sensitive
// browser behavior remains owned by the pinned ICU/MozJS path; this object is
// the target's C/POSIX locale identity, not a host locale or a fabricated web
// rendering result.
struct nagi_libcpp_locale_identity {
    nagi_uintptr_t implementation;
};

static nagi_libcpp_locale_identity nagi_classic_locale = {0};

// A C++ reference is passed as the address of the referred object in this
// ABI. Use the equivalent pointer spelling here so the freestanding C
// linkage declaration remains warning-free while preserving that ABI shape.
extern "C" const void *nagi_cxx_locale_classic()
    __asm__("_ZNSt3__16locale7classicEv");

extern "C" const void *nagi_cxx_locale_classic() {
    return &nagi_classic_locale;
}

// libc++'s locale::id is zero-initialized before its first assigned facet
// number. A pointer-sized target object matches the pinned x86-64 libc++ ABI.
extern "C" nagi_uintptr_t nagi_ctype_char_id
    __asm__("_ZNSt3__15ctypeIcE2idE") = 0;
