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

// Keep the real Mesa glthread implementation reachable from the aggregated
// static archive. The pointer is a link-only anchor; it is never called as a
// Nagi substitute for Mesa and therefore cannot provide synthetic rendering.
extern "C" void nagi_mesa_glthread_finish(void *context)
    __asm__("_mesa_glthread_finish");
extern "C" void (*nagi_mesa_glthread_finish_link_anchor)(void *) =
    &nagi_mesa_glthread_finish;

namespace std {
struct nothrow_t {};
enum class align_val_t : nagi_size_t;

// libc++'s freestanding exception fallback declares these out-of-line
// methods even when language exceptions are disabled. Define the matching
// unversioned std::exception hierarchy here so the target gets the real
// Itanium constructor/vtable ABI without importing a host C++ runtime. The
// generated symbols include _ZNSt9bad_allocC1Ev, _ZNSt9bad_allocC2Ev, and
// _ZNKSt9bad_alloc4whatEv.
class exception {
  public:
    exception() noexcept = default;
    exception(const exception &) noexcept = default;
    exception &operator=(const exception &) noexcept = default;
    virtual ~exception() noexcept;
    virtual const char *what() const noexcept;
};

inline exception::~exception() noexcept {}

inline const char *exception::what() const noexcept { return "std::exception"; }

class bad_alloc : public exception {
  public:
    bad_alloc() noexcept;
    bad_alloc(const bad_alloc &) noexcept = default;
    bad_alloc &operator=(const bad_alloc &) noexcept = default;
    ~bad_alloc() noexcept override;
    const char *what() const noexcept override;
};

bad_alloc::bad_alloc() noexcept = default;
bad_alloc::~bad_alloc() noexcept = default;
const char *bad_alloc::what() const noexcept { return "std::bad_alloc"; }
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

// Static objects in the pinned C++ dependencies register their destructors
// through the Itanium ABI. Keep that lifecycle contract in the target image
// instead of silently discarding destructors or importing libc++abi. The
// table is bounded because M17's freestanding image has no heap-backed C++
// runtime; registration reports failure when the explicit capacity is full.
using nagi_cxx_destructor = void (*)(void *);

struct nagi_cxx_atexit_record {
    nagi_cxx_destructor destructor;
    void *object;
    void *dso_handle;
    bool active;
};

static constexpr nagi_uintptr_t NAGI_CXX_ATEXIT_CAPACITY = 512;
static nagi_cxx_atexit_record nagi_cxx_atexit_records[NAGI_CXX_ATEXIT_CAPACITY]{};
static nagi_uintptr_t nagi_cxx_atexit_count = 0;
static unsigned char nagi_cxx_atexit_lock = 0;

static void nagi_cxx_atexit_lock_acquire() {
    while (__atomic_test_and_set(&nagi_cxx_atexit_lock, __ATOMIC_ACQUIRE)) {
        __asm__ volatile("pause" ::: "memory");
    }
}

static void nagi_cxx_atexit_lock_release() {
    __atomic_clear(&nagi_cxx_atexit_lock, __ATOMIC_RELEASE);
}

extern "C" int __cxa_atexit(nagi_cxx_destructor destructor, void *object,
                            void *dso_handle) {
    if (destructor == nullptr) {
        return -1;
    }

    nagi_cxx_atexit_lock_acquire();
    if (nagi_cxx_atexit_count >= NAGI_CXX_ATEXIT_CAPACITY) {
        nagi_cxx_atexit_lock_release();
        return -1;
    }
    nagi_cxx_atexit_records[nagi_cxx_atexit_count++] = {
        destructor, object, dso_handle, true};
    nagi_cxx_atexit_lock_release();
    return 0;
}

extern "C" void __cxa_finalize(void *dso_handle) {
    for (;;) {
        nagi_cxx_destructor destructor = nullptr;
        void *object = nullptr;

        nagi_cxx_atexit_lock_acquire();
        for (nagi_uintptr_t index = nagi_cxx_atexit_count; index != 0; --index) {
            nagi_cxx_atexit_record &record = nagi_cxx_atexit_records[index - 1];
            if (record.active &&
                (dso_handle == nullptr || record.dso_handle == dso_handle)) {
                record.active = false;
                destructor = record.destructor;
                object = record.object;
                break;
            }
        }
        nagi_cxx_atexit_lock_release();

        if (destructor == nullptr) {
            return;
        }
        destructor(object);
    }
}

// Nagi's POSIX exit boundary calls this hook before the process-exit syscall.
// A weak no-op is supplied by nagi-posix for non-C++ target images; this strong
// definition activates the real destructor registry for M17's Servo image.
extern "C" void nagi_cxx_finalize() {
    __cxa_finalize(nullptr);
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

static_assert(sizeof(void *) == 8);
static_assert(sizeof(nagi_gnu_rb_tree_node_base) == 32);
static_assert(__builtin_offsetof(nagi_gnu_rb_tree_node_base, parent) == 8);
static_assert(__builtin_offsetof(nagi_gnu_rb_tree_node_base, left) == 16);
static_assert(__builtin_offsetof(nagi_gnu_rb_tree_node_base, right) == 24);

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

extern "C" const nagi_gnu_rb_tree_node_base *nagi_gnu_rb_tree_increment_const(
    const nagi_gnu_rb_tree_node_base *node)
    __asm__("_ZSt18_Rb_tree_incrementPKSt18_Rb_tree_node_base");

extern "C" const nagi_gnu_rb_tree_node_base *nagi_gnu_rb_tree_increment_const(
    const nagi_gnu_rb_tree_node_base *node) {
    return nagi_gnu_rb_tree_increment(
        const_cast<nagi_gnu_rb_tree_node_base *>(node));
}

static constexpr unsigned NAGI_GNU_RB_RED = 0;
static constexpr unsigned NAGI_GNU_RB_BLACK = 1;

struct nagi_gnu_rehash_result {
    bool needs_rehash;
    unsigned char padding[7];
    nagi_size_t bucket_count;
};

struct nagi_gnu_prime_rehash_policy {
    float max_load_factor;
    unsigned padding;
    mutable nagi_size_t next_resize;
};

static_assert(sizeof(nagi_gnu_rehash_result) == 16);
static_assert(__builtin_offsetof(nagi_gnu_rehash_result, bucket_count) == 8);
static_assert(sizeof(nagi_gnu_prime_rehash_policy) == 16);
static_assert(__builtin_offsetof(nagi_gnu_prime_rehash_policy, next_resize) == 8);

static bool nagi_gnu_is_prime(nagi_size_t value) {
    if (value < 2) {
        return false;
    }
    if ((value & 1) == 0) {
        return value == 2;
    }
    for (nagi_size_t divisor = 3; divisor <= value / divisor;
         divisor += 2) {
        if (value % divisor == 0) {
            return false;
        }
    }
    return true;
}

static nagi_size_t nagi_gnu_next_prime(nagi_size_t requested) {
    if (requested <= 2) {
        return 2;
    }
    if ((requested & 1) == 0) {
        ++requested;
    }
    while (!nagi_gnu_is_prime(requested)) {
        if (requested == ~static_cast<nagi_size_t>(0)) {
            abort();
        }
        requested += 2;
    }
    return requested;
}

// libc++ keeps its hash-table growth helper outside the header. The pinned
// target uses the same C++11 ABI namespace as libc++; expose the real
// Nagi-owned prime search under that exact symbol instead of linking a host
// libc++ archive. libc++ returns 0 for 0, while every request in [1, 2]
// resolves to the first usable bucket count, 2.
extern "C" nagi_size_t nagi_libcpp_next_prime(nagi_size_t requested)
    __asm__("_ZNSt3__112__next_primeEm");

extern "C" nagi_size_t nagi_libcpp_next_prime(nagi_size_t requested) {
    if (requested == 0) {
        return 0;
    }
    return nagi_gnu_next_prime(requested);
}

extern "C" nagi_gnu_rehash_result nagi_gnu_prime_need_rehash(
    const nagi_gnu_prime_rehash_policy *policy, nagi_size_t bucket_count,
    nagi_size_t element_count, nagi_size_t insertion_count)
    __asm__("_ZNKSt8__detail20_Prime_rehash_policy14_M_need_rehashEmmm");

extern "C" nagi_gnu_rehash_result nagi_gnu_prime_need_rehash(
    const nagi_gnu_prime_rehash_policy *policy, nagi_size_t bucket_count,
    nagi_size_t element_count, nagi_size_t insertion_count) {
    if (policy == nullptr ||
        insertion_count > ~static_cast<nagi_size_t>(0) - element_count ||
        !(policy->max_load_factor > 0.0f)) {
        abort();
    }
    const nagi_size_t total = element_count + insertion_count;
    if (total <= policy->next_resize) {
        return {false, {}, 0};
    }

    // The Nagi Servo/Mesa objects use the libstdc++ default factor (1.0).
    // Match libstdc++'s double arithmetic here; this is part of the policy's
    // observable bucket-growth behavior, not merely an ABI-shaped stub.
    const float factor = policy->max_load_factor;
    const nagi_size_t initial = policy->next_resize == 0 ? 11 : 0;
    const double minimum_buckets =
        static_cast<double>(total > initial ? total : initial) /
        static_cast<double>(factor);
    const nagi_size_t maximum = ~static_cast<nagi_size_t>(0);
    const nagi_size_t minimum =
        minimum_buckets >= static_cast<double>(maximum)
            ? maximum
            : static_cast<nagi_size_t>(minimum_buckets);
    const nagi_size_t growth =
        bucket_count > (maximum / 2)
            ? maximum
            : bucket_count * 2;
    if (minimum >= bucket_count) {
        const nagi_size_t requested =
            minimum == maximum
                ? minimum
                : (minimum + 1 > growth ? minimum + 1 : growth);
        return {true, {}, nagi_gnu_next_prime(requested)};
    }

    const double next = static_cast<double>(bucket_count) *
        static_cast<double>(factor);
    policy->next_resize = static_cast<nagi_size_t>(next);
    return {false, {}, 0};
}

static void nagi_gnu_rb_rotate_left(
    nagi_gnu_rb_tree_node_base *node,
    nagi_gnu_rb_tree_node_base *&root) {
    auto *replacement = node->right;
    node->right = replacement->left;
    if (replacement->left != nullptr) {
        replacement->left->parent = node;
    }
    replacement->parent = node->parent;
    if (node == root) {
        root = replacement;
    } else if (node == node->parent->left) {
        node->parent->left = replacement;
    } else {
        node->parent->right = replacement;
    }
    replacement->left = node;
    node->parent = replacement;
}

static void nagi_gnu_rb_rotate_right(
    nagi_gnu_rb_tree_node_base *node,
    nagi_gnu_rb_tree_node_base *&root) {
    auto *replacement = node->left;
    node->left = replacement->right;
    if (replacement->right != nullptr) {
        replacement->right->parent = node;
    }
    replacement->parent = node->parent;
    if (node == root) {
        root = replacement;
    } else if (node == node->parent->right) {
        node->parent->right = replacement;
    } else {
        node->parent->left = replacement;
    }
    replacement->right = node;
    node->parent = replacement;
}

// This is the GNU libstdc++ red-black insertion algorithm over the stable
// _Rb_tree_node_base prefix above. It maintains the real tree invariants and
// header min/max links; it is not a linker-only no-op.
extern "C" void nagi_gnu_rb_insert_and_rebalance(
    bool insert_left,
    nagi_gnu_rb_tree_node_base *node,
    nagi_gnu_rb_tree_node_base *parent,
    nagi_gnu_rb_tree_node_base &header)
    __asm__("_ZSt29_Rb_tree_insert_and_rebalancebPSt18_Rb_tree_node_baseS0_RS_");

extern "C" void nagi_gnu_rb_insert_and_rebalance(
    bool insert_left,
    nagi_gnu_rb_tree_node_base *node,
    nagi_gnu_rb_tree_node_base *parent,
    nagi_gnu_rb_tree_node_base &header) {
    auto *&root = header.parent;
    node->parent = parent;
    node->left = nullptr;
    node->right = nullptr;
    node->color = NAGI_GNU_RB_RED;

    if (insert_left) {
        parent->left = node;
        if (parent == &header) {
            root = node;
            header.right = node;
        } else if (parent == header.left) {
            header.left = node;
        }
    } else {
        parent->right = node;
        if (parent == header.right) {
            header.right = node;
        }
    }

    while (node != root && node->parent->color == NAGI_GNU_RB_RED) {
        auto *parent_node = node->parent;
        auto *grandparent = parent_node->parent;
        if (parent_node == grandparent->left) {
            auto *uncle = grandparent->right;
            if (uncle != nullptr && uncle->color == NAGI_GNU_RB_RED) {
                parent_node->color = NAGI_GNU_RB_BLACK;
                uncle->color = NAGI_GNU_RB_BLACK;
                grandparent->color = NAGI_GNU_RB_RED;
                node = grandparent;
            } else {
                if (node == parent_node->right) {
                    node = parent_node;
                    nagi_gnu_rb_rotate_left(node, root);
                    parent_node = node->parent;
                    grandparent = parent_node->parent;
                }
                parent_node->color = NAGI_GNU_RB_BLACK;
                grandparent->color = NAGI_GNU_RB_RED;
                nagi_gnu_rb_rotate_right(grandparent, root);
            }
        } else {
            auto *uncle = grandparent->left;
            if (uncle != nullptr && uncle->color == NAGI_GNU_RB_RED) {
                parent_node->color = NAGI_GNU_RB_BLACK;
                uncle->color = NAGI_GNU_RB_BLACK;
                grandparent->color = NAGI_GNU_RB_RED;
                node = grandparent;
            } else {
                if (node == parent_node->left) {
                    node = parent_node;
                    nagi_gnu_rb_rotate_right(node, root);
                    parent_node = node->parent;
                    grandparent = parent_node->parent;
                }
                parent_node->color = NAGI_GNU_RB_BLACK;
                grandparent->color = NAGI_GNU_RB_RED;
                nagi_gnu_rb_rotate_left(grandparent, root);
            }
        }
    }
    root->color = NAGI_GNU_RB_BLACK;
}

extern "C" nagi_gnu_rb_tree_node_base *nagi_gnu_rb_tree_decrement(
    nagi_gnu_rb_tree_node_base *node)
    __asm__("_ZSt18_Rb_tree_decrementPSt18_Rb_tree_node_base");

extern "C" nagi_gnu_rb_tree_node_base *nagi_gnu_rb_tree_decrement(
    nagi_gnu_rb_tree_node_base *node) {
    if (node == nullptr) {
        return nullptr;
    }
    if (node->color == NAGI_GNU_RB_RED && node->parent != nullptr &&
        node->parent->parent == node) {
        return node->right;
    }
    if (node->left != nullptr) {
        node = node->left;
        while (node->right != nullptr) {
            node = node->right;
        }
        return node;
    }
    auto *parent = node->parent;
    while (parent != nullptr && node == parent->left) {
        node = parent;
        parent = parent->parent;
    }
    return parent;
}

extern "C" const nagi_gnu_rb_tree_node_base *nagi_gnu_rb_tree_decrement_const(
    const nagi_gnu_rb_tree_node_base *node)
    __asm__("_ZSt18_Rb_tree_decrementPKSt18_Rb_tree_node_base");

extern "C" const nagi_gnu_rb_tree_node_base *nagi_gnu_rb_tree_decrement_const(
    const nagi_gnu_rb_tree_node_base *node) {
    return nagi_gnu_rb_tree_decrement(
        const_cast<nagi_gnu_rb_tree_node_base *>(node));
}

// This is the real GNU erase/rebalance operation. The returned node is the
// physical node that the caller must destroy; all root, header, parent, color,
// and black-height invariants are updated before returning.
extern "C" nagi_gnu_rb_tree_node_base *nagi_gnu_rb_rebalance_for_erase(
    nagi_gnu_rb_tree_node_base *node,
    nagi_gnu_rb_tree_node_base &header)
    __asm__("_ZSt28_Rb_tree_rebalance_for_erasePSt18_Rb_tree_node_baseRS_");

extern "C" nagi_gnu_rb_tree_node_base *nagi_gnu_rb_rebalance_for_erase(
    nagi_gnu_rb_tree_node_base *node,
    nagi_gnu_rb_tree_node_base &header) {
    auto *&root = header.parent;
    auto *&leftmost = header.left;
    auto *&rightmost = header.right;
    auto *replacement = node;
    nagi_gnu_rb_tree_node_base *child = nullptr;
    nagi_gnu_rb_tree_node_base *child_parent = nullptr;

    if (replacement->left == nullptr) {
        child = replacement->right;
    } else if (replacement->right == nullptr) {
        child = replacement->left;
    } else {
        replacement = replacement->right;
        while (replacement->left != nullptr) {
            replacement = replacement->left;
        }
        child = replacement->right;
    }

    if (replacement != node) {
        node->left->parent = replacement;
        replacement->left = node->left;
        if (replacement != node->right) {
            child_parent = replacement->parent;
            if (child != nullptr) {
                child->parent = replacement->parent;
            }
            replacement->parent->left = child;
            replacement->right = node->right;
            node->right->parent = replacement;
        } else {
            child_parent = replacement;
        }
        if (root == node) {
            root = replacement;
        } else if (node->parent->left == node) {
            node->parent->left = replacement;
        } else {
            node->parent->right = replacement;
        }
        replacement->parent = node->parent;
        const unsigned color = replacement->color;
        replacement->color = node->color;
        node->color = color;
        replacement = node;
    } else {
        child_parent = replacement->parent;
        if (child != nullptr) {
            child->parent = replacement->parent;
        }
        if (root == node) {
            root = child;
        } else if (node->parent->left == node) {
            node->parent->left = child;
        } else {
            node->parent->right = child;
        }
        if (leftmost == node) {
            if (node->right == nullptr) {
                leftmost = node->parent;
            } else {
                leftmost = child;
                while (leftmost->left != nullptr) {
                    leftmost = leftmost->left;
                }
            }
        }
        if (rightmost == node) {
            if (node->left == nullptr) {
                rightmost = node->parent;
            } else {
                rightmost = child;
                while (rightmost->right != nullptr) {
                    rightmost = rightmost->right;
                }
            }
        }
    }

    if (replacement->color != NAGI_GNU_RB_RED) {
        while (child != root &&
               (child == nullptr || child->color == NAGI_GNU_RB_BLACK)) {
            if (child == child_parent->left) {
                auto *sibling = child_parent->right;
                if (sibling->color == NAGI_GNU_RB_RED) {
                    sibling->color = NAGI_GNU_RB_BLACK;
                    child_parent->color = NAGI_GNU_RB_RED;
                    nagi_gnu_rb_rotate_left(child_parent, root);
                    sibling = child_parent->right;
                }
                if ((sibling->left == nullptr ||
                     sibling->left->color == NAGI_GNU_RB_BLACK) &&
                    (sibling->right == nullptr ||
                     sibling->right->color == NAGI_GNU_RB_BLACK)) {
                    sibling->color = NAGI_GNU_RB_RED;
                    child = child_parent;
                    child_parent = child_parent->parent;
                } else {
                    if (sibling->right == nullptr ||
                        sibling->right->color == NAGI_GNU_RB_BLACK) {
                        sibling->left->color = NAGI_GNU_RB_BLACK;
                        sibling->color = NAGI_GNU_RB_RED;
                        nagi_gnu_rb_rotate_right(sibling, root);
                        sibling = child_parent->right;
                    }
                    sibling->color = child_parent->color;
                    child_parent->color = NAGI_GNU_RB_BLACK;
                    if (sibling->right != nullptr) {
                        sibling->right->color = NAGI_GNU_RB_BLACK;
                    }
                    nagi_gnu_rb_rotate_left(child_parent, root);
                    break;
                }
            } else {
                auto *sibling = child_parent->left;
                if (sibling->color == NAGI_GNU_RB_RED) {
                    sibling->color = NAGI_GNU_RB_BLACK;
                    child_parent->color = NAGI_GNU_RB_RED;
                    nagi_gnu_rb_rotate_right(child_parent, root);
                    sibling = child_parent->left;
                }
                if ((sibling->right == nullptr ||
                     sibling->right->color == NAGI_GNU_RB_BLACK) &&
                    (sibling->left == nullptr ||
                     sibling->left->color == NAGI_GNU_RB_BLACK)) {
                    sibling->color = NAGI_GNU_RB_RED;
                    child = child_parent;
                    child_parent = child_parent->parent;
                } else {
                    if (sibling->left == nullptr ||
                        sibling->left->color == NAGI_GNU_RB_BLACK) {
                        sibling->right->color = NAGI_GNU_RB_BLACK;
                        sibling->color = NAGI_GNU_RB_RED;
                        nagi_gnu_rb_rotate_left(sibling, root);
                        sibling = child_parent->left;
                    }
                    sibling->color = child_parent->color;
                    child_parent->color = NAGI_GNU_RB_BLACK;
                    if (sibling->left != nullptr) {
                        sibling->left->color = NAGI_GNU_RB_BLACK;
                    }
                    nagi_gnu_rb_rotate_right(child_parent, root);
                    break;
                }
            }
        }
        if (child != nullptr) {
            child->color = NAGI_GNU_RB_BLACK;
        }
    }
    return replacement;
}

// Compiler-rt's target-independent 64-bit popcount ABI used by freestanding
// Mesa objects. Keep the operation in this Nagi-owned runtime instead of
// pulling a host compiler runtime into the guest image.
extern "C" int nagi_popcountdi2(unsigned long long value)
    __asm__("__popcountdi2");

extern "C" int nagi_popcountdi2(unsigned long long value) {
    int count = 0;
    while (value != 0) {
        value &= value - 1;
        ++count;
    }
    return count;
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

extern "C" [[noreturn]] void nagi_gnu_throw_out_of_range_fmt(
    const char *, ...)
    __asm__("_ZSt24__throw_out_of_range_fmtPKcz");

extern "C" [[noreturn]] void nagi_gnu_throw_out_of_range_fmt(
    const char *, ...) {
    abort();
}

// The M17 target is built with C++ exceptions disabled and has no host
// libc++abi.  These exception entrypoints are retained by a small number of
// standard-library code paths; if one is reached, terminating through Nagi's
// real abort boundary is the only truthful behavior.  Returning a fabricated
// exception object would make the link pass while violating the target
// runtime contract.
extern "C" void *nagi_cxa_begin_catch(void *exception)
    __asm__("__cxa_begin_catch");

extern "C" void *nagi_cxa_begin_catch(void *) {
    abort();
}

extern "C" void nagi_cxa_rethrow() __asm__("__cxa_rethrow");

extern "C" void nagi_cxa_rethrow() {
    abort();
}

extern "C" [[noreturn]] void nagi_gnu_throw_bad_array_new_length()
    __asm__("_ZSt28__throw_bad_array_new_lengthv");

extern "C" [[noreturn]] void nagi_gnu_throw_bad_array_new_length() {
    abort();
}

// A pure-virtual dispatch is a programming error in the target image. Keep
// the Itanium ABI entrypoint real and terminate through Nagi's process
// boundary instead of importing libc++abi or returning to an invalid vtable.
extern "C" [[noreturn]] void __cxa_pure_virtual() {
    abort();
}

extern "C" [[noreturn]] void nagi_gnu_throw_bad_alloc()
    __asm__("_ZSt17__throw_bad_allocv");

extern "C" [[noreturn]] void nagi_gnu_throw_bad_alloc() {
    abort();
}

extern "C" [[noreturn]] void nagi_cxa_bad_typeid()
    __asm__("__cxa_bad_typeid");

extern "C" [[noreturn]] void nagi_cxa_bad_typeid() {
    abort();
}

extern "C" [[noreturn]] void __cxa_end_catch() {
    abort();
}

// libc++'s target pthread configuration keeps the opaque pthread mutex as
// the first field of std::__1::mutex. Route its out-of-line ABI entrypoints to
// Nagi's real relibc pthread implementation; no host synchronization runtime
// or unlocked success path is substituted.
extern "C" int pthread_mutex_lock(void *mutex);
extern "C" int pthread_mutex_trylock(void *mutex);
extern "C" int pthread_mutex_unlock(void *mutex);
extern "C" int pthread_mutex_destroy(void *mutex);

extern "C" void nagi_libcpp_mutex_lock(void *mutex)
    __asm__("_ZNSt3__15mutex4lockEv");

extern "C" void nagi_libcpp_mutex_lock(void *mutex) {
    if (mutex == nullptr || pthread_mutex_lock(mutex) != 0) {
        abort();
    }
}

extern "C" bool nagi_libcpp_mutex_try_lock(void *mutex)
    __asm__("_ZNSt3__15mutex8try_lockEv");

extern "C" bool nagi_libcpp_mutex_try_lock(void *mutex) {
    return mutex != nullptr && pthread_mutex_trylock(mutex) == 0;
}

extern "C" void nagi_libcpp_mutex_unlock(void *mutex)
    __asm__("_ZNSt3__15mutex6unlockEv");

extern "C" void nagi_libcpp_mutex_unlock(void *mutex) {
    if (mutex == nullptr || pthread_mutex_unlock(mutex) != 0) {
        abort();
    }
}

extern "C" void nagi_libcpp_mutex_destroy(void *mutex)
    __asm__("_ZNSt3__15mutexD1Ev");

extern "C" void nagi_libcpp_mutex_destroy(void *mutex) {
    if (mutex == nullptr || pthread_mutex_destroy(mutex) != 0) {
        abort();
    }
}

extern "C" void nagi_libcpp_mutex_destroy_base(void *mutex)
    __asm__("_ZNSt3__15mutexD2Ev");

extern "C" void nagi_libcpp_mutex_destroy_base(void *mutex) {
    nagi_libcpp_mutex_destroy(mutex);
}

// The Itanium deleting-destructor entrypoint is used when a libc++ mutex is
// destroyed through delete. Run the real pthread destructor first, then free
// the object through Nagi's allocator boundary.
extern "C" void nagi_libcpp_mutex_destroy_deleting(void *mutex)
    __asm__("_ZNSt3__15mutexD0Ev");

extern "C" void nagi_libcpp_mutex_destroy_deleting(void *mutex) {
    nagi_libcpp_mutex_destroy(mutex);
    nagi_posix_free(mutex);
}

extern "C" int pthread_cond_signal(void *condition);
extern "C" int pthread_cond_broadcast(void *condition);
extern "C" int pthread_cond_wait(void *condition, void *mutex);
extern "C" int pthread_cond_destroy(void *condition);

// libc++'s call_once implementation uses one process-wide mutex and
// condition variable to guard the unsigned-long once flag. These zero-filled
// guest objects are consumed by the real Nagi POSIX pthread implementation;
// they are not host synchronization objects or a success-only fallback.
alignas(8) static unsigned char nagi_libcpp_once_mutex[8]{};
alignas(8) static unsigned char nagi_libcpp_once_condition[8]{};

extern "C" void nagi_libcpp_call_once(volatile unsigned long *flag,
                                       void *argument,
                                       void (*function)(void *))
    __asm__("_ZNSt3__111__call_onceERVmPvPFvS2_E");

extern "C" void nagi_libcpp_call_once(volatile unsigned long *flag,
                                       void *argument,
                                       void (*function)(void *)) {
    if (flag == nullptr || function == nullptr) {
        abort();
    }
    if (pthread_mutex_lock(nagi_libcpp_once_mutex) != 0) {
        abort();
    }

    constexpr unsigned long unset = 0;
    constexpr unsigned long pending = 1;
    constexpr unsigned long complete = ~0UL;
    while (__atomic_load_n(flag, __ATOMIC_ACQUIRE) == pending) {
        if (pthread_cond_wait(nagi_libcpp_once_condition,
                              nagi_libcpp_once_mutex) != 0) {
            abort();
        }
    }

    if (__atomic_load_n(flag, __ATOMIC_RELAXED) == unset) {
        __atomic_store_n(flag, pending, __ATOMIC_RELAXED);
        if (pthread_mutex_unlock(nagi_libcpp_once_mutex) != 0) {
            abort();
        }
        function(argument);
        if (pthread_mutex_lock(nagi_libcpp_once_mutex) != 0) {
            abort();
        }
        __atomic_store_n(flag, complete, __ATOMIC_RELEASE);
        if (pthread_mutex_unlock(nagi_libcpp_once_mutex) != 0 ||
            pthread_cond_broadcast(nagi_libcpp_once_condition) != 0) {
            abort();
        }
    } else if (pthread_mutex_unlock(nagi_libcpp_once_mutex) != 0) {
        abort();
    }
}

extern "C" void nagi_libcpp_condition_variable_notify_one(void *condition)
    __asm__("_ZNSt3__118condition_variable10notify_oneEv");

extern "C" void nagi_libcpp_condition_variable_notify_one(void *condition) {
    if (condition == nullptr || pthread_cond_signal(condition) != 0) {
        abort();
    }
}

extern "C" void nagi_libcpp_condition_variable_notify_all(void *condition)
    __asm__("_ZNSt3__118condition_variable10notify_allEv");

extern "C" void nagi_libcpp_condition_variable_notify_all(void *condition) {
    if (condition == nullptr || pthread_cond_broadcast(condition) != 0) {
        abort();
    }
}

// libc++'s unique_lock stores the mutex pointer at offset zero and its
// ownership byte immediately after that pointer. The target libc++ pthread
// backend uses the pthread condition-variable object as the first field of
// std::__1::condition_variable, so this preserves the real unlock/wait/relock
// operation through relibc rather than returning from a synthetic wait.
extern "C" void nagi_libcpp_condition_variable_wait(void *condition,
                                                     void *unique_lock)
    __asm__("_ZNSt3__118condition_variable4waitERNS_11unique_lockINS_5mutexEEE");

extern "C" void nagi_libcpp_condition_variable_wait(void *condition,
                                                     void *unique_lock) {
    if (condition == nullptr || unique_lock == nullptr) {
        abort();
    }
    void *mutex = *reinterpret_cast<void **>(unique_lock);
    const unsigned char owns = *reinterpret_cast<unsigned char *>(
        reinterpret_cast<unsigned char *>(unique_lock) + sizeof(void *));
    if (mutex == nullptr || owns == 0 || pthread_cond_wait(condition, mutex) != 0) {
        abort();
    }
}

extern "C" void nagi_libcpp_condition_variable_destroy(void *condition)
    __asm__("_ZNSt3__118condition_variableD1Ev");

extern "C" void nagi_libcpp_condition_variable_destroy(void *condition) {
    if (condition == nullptr || pthread_cond_destroy(condition) != 0) {
        abort();
    }
}

extern "C" void nagi_libcpp_condition_variable_destroy_base(void *condition)
    __asm__("_ZNSt3__118condition_variableD2Ev");

extern "C" void nagi_libcpp_condition_variable_destroy_base(void *condition) {
    nagi_libcpp_condition_variable_destroy(condition);
}

extern "C" void nagi_libcpp_condition_variable_destroy_deleting(void *condition)
    __asm__("_ZNSt3__118condition_variableD0Ev");

extern "C" void nagi_libcpp_condition_variable_destroy_deleting(void *condition) {
    nagi_libcpp_condition_variable_destroy(condition);
    nagi_posix_free(condition);
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

static_assert(sizeof(nagi_gnu_basic_string_layout) == 32);
static_assert(__builtin_offsetof(nagi_gnu_basic_string_layout, data) == 0);
static_assert(__builtin_offsetof(nagi_gnu_basic_string_layout, length) == 8);
static_assert(__builtin_offsetof(nagi_gnu_basic_string_layout, storage) == 16);

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

static void nagi_copy_bytes(char *destination, const char *source,
                            nagi_size_t count) {
    for (nagi_size_t index = 0; index < count; ++index) {
        destination[index] = source[index];
    }
}

static void nagi_fill_bytes(char *destination, char value, nagi_size_t count) {
    for (nagi_size_t index = 0; index < count; ++index) {
        destination[index] = value;
    }
}

static constexpr nagi_size_t NAGI_GNU_BASIC_STRING_LOCAL_CAPACITY = 15;

static nagi_size_t nagi_gnu_basic_string_capacity(
    const nagi_gnu_basic_string_layout *string) {
    const char *local = reinterpret_cast<const char *>(string) + 16;
    return string->data == local ? NAGI_GNU_BASIC_STRING_LOCAL_CAPACITY
                                 : string->storage.capacity;
}

// These are the concrete GNU C++11 basic_string operations still referenced by
// the pinned target objects.  Implement them over the same Nagi allocator and
// object layout as _M_dispose; providing only the mangled names would make a
// link succeed while leaving the actual string operation undefined.
extern "C" nagi_gnu_basic_string_layout *nagi_gnu_basic_string_append(
    nagi_gnu_basic_string_layout *object, const char *source,
    nagi_size_t count)
    __asm__("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE9_M_appendEPKcm");

extern "C" nagi_gnu_basic_string_layout *nagi_gnu_basic_string_append(
    nagi_gnu_basic_string_layout *object, const char *source,
    nagi_size_t count) {
    if (object == nullptr || (source == nullptr && count != 0)) {
        abort();
    }
    if (count == 0) {
        return object;
    }

    const nagi_size_t maximum = ~static_cast<nagi_size_t>(0);
    if (object->length > maximum - count || object->length + count >= maximum) {
        abort();
    }

    const nagi_size_t old_length = object->length;
    const nagi_size_t required = old_length + count;
    const nagi_size_t capacity = nagi_gnu_basic_string_capacity(object);
    char *old_data = object->data;

    if (required <= capacity) {
        nagi_copy_bytes(old_data + old_length, source, count);
        object->length = required;
        old_data[required] = '\0';
        return object;
    }

    nagi_size_t new_capacity = capacity;
    if (new_capacity <= (maximum - 1) / 2) {
        new_capacity *= 2;
    }
    if (new_capacity < required) {
        new_capacity = required;
    }
    if (new_capacity >= maximum) {
        abort();
    }

    char *new_data = static_cast<char *>(nagi_posix_malloc(new_capacity + 1));
    if (new_data == nullptr) {
        abort();
    }
    nagi_copy_bytes(new_data, old_data, old_length);
    nagi_copy_bytes(new_data + old_length, source, count);
    new_data[required] = '\0';
    const char *local = reinterpret_cast<const char *>(object) + 16;
    if (old_data != local) {
        nagi_posix_free(old_data);
    }
    object->data = new_data;
    object->length = required;
    object->storage.capacity = new_capacity;
    return object;
}

extern "C" char *nagi_gnu_basic_string_create(
    nagi_gnu_basic_string_layout *, nagi_size_t &capacity,
    nagi_size_t old_capacity)
    __asm__("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE9_M_createERmm");

extern "C" char *nagi_gnu_basic_string_create(
    nagi_gnu_basic_string_layout *, nagi_size_t &capacity,
    nagi_size_t old_capacity) {
    const nagi_size_t maximum = ~static_cast<nagi_size_t>(0);
    if (capacity >= maximum) {
        abort();
    }
    if (capacity > old_capacity && capacity < maximum / 2) {
        const nagi_size_t doubled = old_capacity * 2;
        if (doubled > capacity) {
            capacity = doubled;
        }
    }
    if (capacity >= maximum) {
        abort();
    }
    auto *buffer = static_cast<char *>(nagi_posix_malloc(capacity + 1));
    if (buffer == nullptr) {
        abort();
    }
    return buffer;
}

extern "C" nagi_gnu_basic_string_layout *nagi_gnu_basic_string_replace(
    nagi_gnu_basic_string_layout *object, nagi_size_t position,
    nagi_size_t removed, const char *source, nagi_size_t inserted)
    __asm__("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE10_M_replaceEmmPKcm");

extern "C" nagi_gnu_basic_string_layout *nagi_gnu_basic_string_replace(
    nagi_gnu_basic_string_layout *object, nagi_size_t position,
    nagi_size_t removed, const char *source, nagi_size_t inserted) {
    if (object == nullptr || (source == nullptr && inserted != 0) ||
        position > object->length) {
        abort();
    }
    const nagi_size_t available = object->length - position;
    if (removed > available) {
        removed = available;
    }
    const nagi_size_t maximum = ~static_cast<nagi_size_t>(0);
    if (inserted > maximum - (object->length - removed)) {
        abort();
    }
    const nagi_size_t new_length = object->length - removed + inserted;
    const nagi_size_t old_capacity = nagi_gnu_basic_string_capacity(object);
    nagi_size_t new_capacity = new_length;
    char *new_data = nagi_gnu_basic_string_create(object, new_capacity, old_capacity);
    const char *old_data = object->data;
    nagi_copy_bytes(new_data, old_data, position);
    nagi_copy_bytes(new_data + position, source, inserted);
    nagi_copy_bytes(new_data + position + inserted,
                    old_data + position + removed,
                    object->length - position - removed);
    new_data[new_length] = '\0';
    const char *local = reinterpret_cast<const char *>(object) + 16;
    if (old_data != nullptr && old_data != local) {
        nagi_posix_free(object->data);
    }
    object->data = new_data;
    object->length = new_length;
    object->storage.capacity = new_capacity;
    return object;
}

extern "C" void nagi_gnu_basic_string_resize(
    nagi_gnu_basic_string_layout *object, nagi_size_t new_length, char value)
    __asm__("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE6resizeEmc");

extern "C" void nagi_gnu_basic_string_resize(
    nagi_gnu_basic_string_layout *object, nagi_size_t new_length, char value) {
    if (object == nullptr || object->data == nullptr) {
        abort();
    }
    const nagi_size_t maximum = ~static_cast<nagi_size_t>(0);
    if (new_length >= maximum) {
        abort();
    }
    if (new_length <= object->length) {
        object->length = new_length;
        object->data[new_length] = '\0';
        return;
    }

    const nagi_size_t old_length = object->length;
    const nagi_size_t capacity = nagi_gnu_basic_string_capacity(object);
    if (new_length <= capacity) {
        nagi_fill_bytes(object->data + old_length, value,
                        new_length - old_length);
        object->length = new_length;
        object->data[new_length] = '\0';
        return;
    }

    nagi_size_t new_capacity = capacity;
    if (new_capacity <= (maximum - 1) / 2) {
        new_capacity *= 2;
    }
    if (new_capacity < new_length) {
        new_capacity = new_length;
    }
    if (new_capacity >= maximum) {
        abort();
    }
    char *new_data = static_cast<char *>(nagi_posix_malloc(new_capacity + 1));
    if (new_data == nullptr) {
        abort();
    }
    nagi_copy_bytes(new_data, object->data, old_length);
    nagi_fill_bytes(new_data + old_length, value, new_length - old_length);
    new_data[new_length] = '\0';
    const char *local = reinterpret_cast<const char *>(object) + 16;
    if (object->data != local) {
        nagi_posix_free(object->data);
    }
    object->data = new_data;
    object->length = new_length;
    object->storage.capacity = new_capacity;
}

extern "C" nagi_gnu_basic_string_layout *nagi_gnu_basic_string_replace_aux(
    nagi_gnu_basic_string_layout *object, nagi_size_t position,
    nagi_size_t removed, nagi_size_t inserted, char value)
    __asm__("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE14_M_replace_auxEmmmc");

extern "C" void nagi_gnu_basic_string_construct(
    nagi_gnu_basic_string_layout *object, nagi_size_t count, char value)
    __asm__("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE12_M_constructEmc");

extern "C" void nagi_gnu_basic_string_construct(
    nagi_gnu_basic_string_layout *object, nagi_size_t count, char value) {
    if (object == nullptr) {
        abort();
    }
    const nagi_size_t maximum = ~static_cast<nagi_size_t>(0);
    if (count >= maximum) {
        abort();
    }
    const char *local = reinterpret_cast<const char *>(object) + 16;
    if (count <= NAGI_GNU_BASIC_STRING_LOCAL_CAPACITY) {
        object->data = const_cast<char *>(local);
        object->length = count;
        nagi_fill_bytes(object->data, value, count);
        object->data[count] = '\0';
        return;
    }

    char *data = static_cast<char *>(nagi_posix_malloc(count + 1));
    if (data == nullptr) {
        abort();
    }
    nagi_fill_bytes(data, value, count);
    data[count] = '\0';
    object->data = data;
    object->length = count;
    object->storage.capacity = count;
}

extern "C" nagi_gnu_basic_string_layout *nagi_gnu_basic_string_replace_aux(
    nagi_gnu_basic_string_layout *object, nagi_size_t position,
    nagi_size_t removed, nagi_size_t inserted, char value) {
    if (object == nullptr || object->data == nullptr ||
        position > object->length) {
        abort();
    }
    const nagi_size_t available = object->length - position;
    if (removed > available) {
        removed = available;
    }
    const nagi_size_t maximum = ~static_cast<nagi_size_t>(0);
    if (inserted > maximum - (object->length - removed)) {
        abort();
    }
    const nagi_size_t new_length = object->length - removed + inserted;
    if (new_length >= maximum) {
        abort();
    }
    const nagi_size_t old_capacity = nagi_gnu_basic_string_capacity(object);
    nagi_size_t new_capacity = new_length;
    char *new_data = nagi_gnu_basic_string_create(object, new_capacity,
                                                   old_capacity);
    const char *old_data = object->data;
    nagi_copy_bytes(new_data, old_data, position);
    nagi_fill_bytes(new_data + position, value, inserted);
    nagi_copy_bytes(new_data + position + inserted,
                    old_data + position + removed,
                    object->length - position - removed);
    new_data[new_length] = '\0';
    const char *local = reinterpret_cast<const char *>(object) + 16;
    if (old_data != local) {
        nagi_posix_free(object->data);
    }
    object->data = new_data;
    object->length = new_length;
    object->storage.capacity = new_capacity;
    return object;
}

extern "C" nagi_size_t nagi_gnu_basic_string_find(
    const nagi_gnu_basic_string_layout *object, const char *needle,
    nagi_size_t needle_length, nagi_size_t position)
    __asm__("_ZNKSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE4findEPKcmm");

extern "C" nagi_size_t nagi_gnu_basic_string_find(
    const nagi_gnu_basic_string_layout *object, const char *needle,
    nagi_size_t needle_length, nagi_size_t position) {
    const nagi_size_t npos = ~static_cast<nagi_size_t>(0);
    if (object == nullptr || (needle == nullptr && needle_length != 0)) {
        abort();
    }
    if (position > object->length) {
        return npos;
    }
    if (needle_length == 0) {
        return position;
    }
    if (needle_length > object->length - position) {
        return npos;
    }

    for (nagi_size_t candidate = position;
         candidate <= object->length - needle_length; ++candidate) {
        bool matches = true;
        for (nagi_size_t index = 0; index < needle_length; ++index) {
            if (object->data[candidate + index] != needle[index]) {
                matches = false;
                break;
            }
        }
        if (matches) {
            return candidate;
        }
    }
    return npos;
}

// The Nagi target is compiled with C++ exceptions disabled and provides no
// host unwinder. If an incompatible object nevertheless enters an exception
// resume path, fail closed through the real Nagi abort boundary instead of
// importing a host unwind runtime or returning as if unwinding succeeded.
extern "C" [[noreturn]] void nagi_unwind_resume(void *)
    __asm__("_Unwind_Resume");

extern "C" [[noreturn]] void nagi_unwind_resume(void *) {
    abort();
}

// MozJS target objects may retain an EH personality reference even though the
// Nagi build disables C++ exceptions and provides no unwinder. If an unwind
// path is reached, terminate through the real guest boundary rather than
// pretending that a personality search succeeded.
extern "C" int nagi_gxx_personality(int, int, nagi_uintptr_t, void *, void *)
    __asm__("__gxx_personality_v0");

extern "C" int nagi_gxx_personality(int, int, nagi_uintptr_t, void *, void *) {
    abort();
}

// The M17 image has no exception unwinder. These ABI queries are retained so
// target objects cannot pull in a host libunwind; reaching either path is an
// unsupported exception operation and fails closed through Nagi abort.
extern "C" [[noreturn]] nagi_uintptr_t nagi_unwind_get_cfa(void *)
    __asm__("_Unwind_GetCFA");

extern "C" [[noreturn]] nagi_uintptr_t nagi_unwind_get_cfa(void *) {
    abort();
}

extern "C" [[noreturn]] void *nagi_unwind_find_enclosing_function(void *)
    __asm__("_Unwind_FindEnclosingFunction");

extern "C" [[noreturn]] void *nagi_unwind_find_enclosing_function(void *) {
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

// libc++'s locale::use_facet is an out-of-line ABI boundary. Nagi 0.1 has no
// host locale database and the current target image does not yet expose a
// complete facet table, so an attempted access must terminate through the
// real guest abort path rather than return a fabricated facet pointer. This
// keeps unsupported locale use fail-closed while allowing the target linker
// to validate the rest of the Servo/Mesa graph.
extern "C" [[noreturn]] const void *nagi_cxx_locale_use_facet(
    const void *locale, void *id)
    __asm__("_ZNKSt3__16locale9use_facetERNS0_2idE");

extern "C" [[noreturn]] const void *nagi_cxx_locale_use_facet(
    const void *, void *) {
    abort();
}

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
