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
struct nothrow_t {};
enum class align_val_t : nagi_size_t;
} // namespace std

extern "C" nagi_uintptr_t __stack_chk_guard = 0xd048c37519fcadfeULL;

// Some target objects use the GNU spelling of the standard nothrow object even
// though Nagi's normal headers are libc++.  The object is an empty tag, so a
// Nagi-owned instance is sufficient for the ABI and does not import a host
// C++ runtime.
extern "C" const std::nothrow_t nagi_gnu_nothrow
    __asm__("_ZSt7nothrow") = {};

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

// The pinned target objects retain a small amount of GNU RTTI surface even
// though the Nagi build disables new RTTI emission. Support identity casts and
// the single-inheritance chain represented by __si_class_type_info. The
// dynamic-cast hint is the ABI-provided offset of the source subobject in the
// most-derived object; unsupported multiple/virtual-inheritance metadata
// fails closed instead of returning an invented pointer.
struct nagi_type_info_layout {
    const void *vtable;
    const char *name;
};

struct nagi_si_type_info_layout {
    nagi_type_info_layout type;
    const nagi_type_info_layout *base_type;
};

extern "C" const void *nagi_si_type_info_vtable[]
    __asm__("_ZTVN10__cxxabiv120__si_class_type_infoE");

static bool nagi_type_is_single_inheritance_base(
    const nagi_type_info_layout *derived,
    const nagi_type_info_layout *base,
    unsigned depth) {
    if (derived == nullptr || base == nullptr || depth > 32) {
        return false;
    }
    if (derived == base) {
        return true;
    }
    const void *si_vtable = nagi_si_type_info_vtable + 2;
    if (derived->vtable != si_vtable) {
        return false;
    }
    const auto *single = reinterpret_cast<const nagi_si_type_info_layout *>(derived);
    return nagi_type_is_single_inheritance_base(single->base_type, base, depth + 1);
}

extern "C" void *nagi_gnu_dynamic_cast(
    const void *static_pointer,
    const void *static_type,
    const void *dynamic_type,
    long source_to_destination_offset)
    __asm__("__dynamic_cast");

extern "C" void *nagi_gnu_dynamic_cast(
    const void *static_pointer,
    const void *static_type,
    const void *dynamic_type,
    long source_to_destination_offset) {
    if (static_pointer == nullptr || static_type == nullptr || dynamic_type == nullptr) {
        return nullptr;
    }
    if (static_type == dynamic_type) {
        return const_cast<void *>(static_pointer);
    }
    if (source_to_destination_offset < 0 ||
        !nagi_type_is_single_inheritance_base(
            static_cast<const nagi_type_info_layout *>(dynamic_type),
            static_cast<const nagi_type_info_layout *>(static_type), 0)) {
        return nullptr;
    }
    return const_cast<char *>(static_cast<const char *>(static_pointer)) -
           source_to_destination_offset;
}

// GNU libstdc++'s tree iterator helper operates on this stable node prefix:
// color/padding at offset zero followed by parent, left, and right pointers.
// Implement the actual in-order successor used by tree iterators rather than
// satisfying the linker with a no-op.
struct nagi_gnu_rb_tree_node_base {
    unsigned color;
    unsigned padding;
    nagi_gnu_rb_tree_node_base *parent;
    nagi_gnu_rb_tree_node_base *left;
    nagi_gnu_rb_tree_node_base *right;
};

extern "C" nagi_gnu_rb_tree_node_base *nagi_gnu_rb_tree_increment(
    nagi_gnu_rb_tree_node_base *node)
    __asm__("_ZSt18_Rb_tree_incrementPSt18_Rb_tree_node_base");

extern "C" nagi_gnu_rb_tree_node_base *nagi_gnu_rb_tree_increment(
    nagi_gnu_rb_tree_node_base *node) {
    if (node == nullptr) {
        return nullptr;
    }
    if (node->right != nullptr) {
        node = node->right;
        while (node->left != nullptr) {
            node = node->left;
        }
        return node;
    }
    auto *parent = node->parent;
    while (parent != nullptr && node == parent->right) {
        node = parent;
        parent = parent->parent;
    }
    return parent;
}

// A small subset of the pinned target objects is emitted with the GNU
// libstdc++ ABI even though the normal Nagi C++ headers are libc++.  The
// no-exception target still needs the concrete length-error entrypoint when a
// standard container detects an invalid size.  Terminating through Nagi's
// real abort boundary is the only valid behavior for this freestanding image;
// importing a host exception runtime would cross the OS boundary.
extern "C" [[noreturn]] void nagi_gnu_throw_length_error(const char *)
    __asm__("_ZSt20__throw_length_errorPKc");

extern "C" [[noreturn]] void nagi_gnu_throw_length_error(const char *) {
    abort();
}

// libstdc++'s C++11 basic_string ABI stores the data pointer at offset zero,
// the length at offset eight, and either the allocated capacity or the
// 16-byte local buffer at offset sixteen on the x86-64 target.  Its destructor
// calls _M_dispose, which must release an allocated buffer through the same
// Nagi allocator used by target operator new.  This is an ABI implementation,
// not a no-op shim: local strings are retained and heap-backed strings are
// released at the real allocator boundary.
struct nagi_gnu_basic_string_layout {
    char *data;
    nagi_size_t length;
    union {
        nagi_size_t capacity;
        char local[16];
    } storage;
};

extern "C" void nagi_gnu_basic_string_dispose(void *object)
    __asm__("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE10_M_disposeEv");

extern "C" void nagi_gnu_basic_string_dispose(void *object) {
    if (object == nullptr) {
        return;
    }

    auto *string = static_cast<nagi_gnu_basic_string_layout *>(object);
    char *local = reinterpret_cast<char *>(object) + 16;
    if (string->data != nullptr && string->data != local) {
        nagi_posix_free(string->data);
    }
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

// The pinned target objects retain the Itanium ABI type-info vtable
// references even though the Nagi build disables RTTI and exceptions. Keep
// the ABI boundary self-contained instead of importing libc++abi. The virtual
// slot order follows __shim_type_info/__class_type_info: destructor pair,
// noop1, noop2, can_catch, search_above_dst, search_below_dst, and
// has_unambiguous_public_base. These methods are not a browser-facing RTTI
// service; they make the target's statically linked ABI objects well-formed.
namespace __cxxabiv1 {
struct nagi_dynamic_cast_info;

class __class_type_info {
  public:
    virtual ~__class_type_info();
    virtual void noop1() const;
    virtual void noop2() const;
    virtual bool can_catch(const void *, void *&) const;
    virtual void search_above_dst(nagi_dynamic_cast_info *, const void *,
                                  const void *, int, bool) const;
    virtual void search_below_dst(nagi_dynamic_cast_info *, const void *, int,
                                  bool) const;
    virtual void has_unambiguous_public_base(nagi_dynamic_cast_info *, void *,
                                             int) const;
};

__class_type_info::~__class_type_info() {}
void __class_type_info::noop1() const {}
void __class_type_info::noop2() const {}
bool __class_type_info::can_catch(const void *, void *&) const { return false; }
void __class_type_info::search_above_dst(nagi_dynamic_cast_info *, const void *,
                                         const void *, int, bool) const {}
void __class_type_info::search_below_dst(nagi_dynamic_cast_info *, const void *,
                                         int, bool) const {}
void __class_type_info::has_unambiguous_public_base(nagi_dynamic_cast_info *,
                                                    void *, int) const {}

class __si_class_type_info final : public __class_type_info {
  public:
    ~__si_class_type_info() override;
    void search_above_dst(nagi_dynamic_cast_info *, const void *, const void *,
                          int, bool) const override;
    void search_below_dst(nagi_dynamic_cast_info *, const void *, int,
                          bool) const override;
    void has_unambiguous_public_base(nagi_dynamic_cast_info *, void *,
                                     int) const override;
};

__si_class_type_info::~__si_class_type_info() {}
void __si_class_type_info::search_above_dst(nagi_dynamic_cast_info *,
                                             const void *, const void *, int,
                                             bool) const {}
void __si_class_type_info::search_below_dst(nagi_dynamic_cast_info *,
                                             const void *, int, bool) const {}
void __si_class_type_info::has_unambiguous_public_base(
    nagi_dynamic_cast_info *, void *, int) const {}

// Multiple-inheritance metadata is present in the pinned Servo/MozJS C++
// objects even though the Nagi build disables new RTTI emission. Keep the
// Itanium ABI vtable in the target-owned runtime so the final link does not
// import libc++abi. M17 does not expose a general RTTI service: the existing
// fail-closed ABI methods remain the only supported behavior for this
// freestanding boundary.
class __vmi_class_type_info final : public __class_type_info {
  public:
    ~__vmi_class_type_info() override;
    void search_above_dst(nagi_dynamic_cast_info *, const void *, const void *,
                          int, bool) const override;
    void search_below_dst(nagi_dynamic_cast_info *, const void *, int,
                          bool) const override;
    void has_unambiguous_public_base(nagi_dynamic_cast_info *, void *,
                                     int) const override;
};

__vmi_class_type_info::~__vmi_class_type_info() {}
void __vmi_class_type_info::search_above_dst(
    nagi_dynamic_cast_info *, const void *, const void *, int, bool) const {}
void __vmi_class_type_info::search_below_dst(nagi_dynamic_cast_info *,
                                              const void *, int, bool) const {}
void __vmi_class_type_info::has_unambiguous_public_base(
    nagi_dynamic_cast_info *, void *, int) const {}
} // namespace __cxxabiv1
