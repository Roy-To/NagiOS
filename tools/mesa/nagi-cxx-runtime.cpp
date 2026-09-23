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

extern "C" [[noreturn]] void __cxa_end_catch() {
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
