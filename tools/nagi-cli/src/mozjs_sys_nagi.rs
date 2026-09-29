use std::path::{Path, PathBuf};

use crate::registry_source::{ensure_registry_checkout, RegistrySourceSpec};

const SPEC: RegistrySourceSpec = RegistrySourceSpec {
    section: "sources.mozjs_sys_nagi",
    component: "mozjs-sys-nagi",
    package: "mozjs_sys",
    version: "153.0.0-2",
    repository: "https://crates.io/crates/mozjs_sys/153.0.0-2",
    registry_archive: "https://crates.io/api/v1/crates/mozjs_sys/153.0.0-2/download",
    source_hash: "sha256:28adaa4255fd0d42133b993ff81d391df41de1d718777f2e3f0aee5ba8636f10",
    license: "MPL-2.0",
    vendored_path: "third_party/mozjs-sys-nagi",
    patch_path: "third_party/mozjs-sys-nagi-patches",
};

pub(crate) fn ensure_mozjs_sys_nagi_checkout(root: &Path) -> Result<PathBuf, String> {
    ensure_registry_checkout(root, &SPEC)
}

#[cfg(test)]
mod tests {
    use super::SPEC;
    use crate::registry_source::validate_source_lock;

    #[test]
    fn mozjs_sys_nagi_lock_is_pinned() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        validate_source_lock(root, &SPEC).expect("pinned mozjs_sys source lock");
    }

    #[test]
    fn nagi_init_runs_retained_elf_constructor_arrays_before_its_body() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let linker = std::fs::read_to_string(root.join("user/nagi-init/linker.ld"))
            .expect("Nagi user linker script");
        for symbol in [
            "PROVIDE_HIDDEN(__preinit_array_start = .);",
            "PROVIDE_HIDDEN(__preinit_array_end = .);",
            "PROVIDE_HIDDEN(__init_array_start = .);",
            "PROVIDE_HIDDEN(__init_array_end = .);",
            "KEEP(*(SORT_BY_INIT_PRIORITY(.preinit_array.*)))",
            "KEEP(*(.preinit_array))",
            "KEEP(*(SORT_BY_INIT_PRIORITY(.init_array.*)))",
            "KEEP(*(.init_array))",
        ] {
            assert!(
                linker.contains(symbol),
                "Nagi user linker script must retain constructor boundary: {symbol}"
            );
        }
        let preinit = linker
            .find(".preinit_array ALIGN(8)")
            .expect("preinit array output section");
        let init = linker
            .find(".init_array ALIGN(8)")
            .expect("init array output section");
        assert!(preinit < init, "preinit array must precede init array");

        let main = std::fs::read_to_string(root.join("user/nagi-init/src/main.rs"))
            .expect("Nagi user entrypoint");
        for symbol in [
            "__preinit_array_start",
            "__preinit_array_end",
            "__init_array_start",
            "__init_array_end",
        ] {
            assert!(
                main.contains(symbol),
                "Nagi process startup must walk {symbol}"
            );
        }
        for declaration in [
            "static __preinit_array_start: u8;",
            "static __preinit_array_end: u8;",
            "static __init_array_start: u8;",
            "static __init_array_end: u8;",
        ] {
            assert!(
                main.contains(declaration),
                "ELF array boundary must be declared as an address marker: {declaration}"
            );
        }
        assert!(
            main.contains("(cursor as *const extern \"C\" fn()).read()"),
            "ELF initializer entries must be read from their linker-defined address"
        );
        assert!(
            main.contains("cursor += core::mem::size_of::<extern \"C\" fn()>();"),
            "ELF initializer iteration must advance by one function pointer"
        );
        let initializer_runner = main
            .find("unsafe fn run_elf_initializers()")
            .expect("user-space ELF initializer runner");
        let entry = main
            .find("pub extern \"C\" fn _start(")
            .expect("capability-aware Nagi entrypoint");
        let runner = &main[initializer_runner..entry];
        let preinit_walk = runner
            .find("addr_of!(__preinit_array_start)")
            .expect("preinit array walk");
        let init_walk = runner
            .find("addr_of!(__init_array_start)")
            .expect("init array walk");
        assert!(preinit_walk < init_walk, "preinit array must run first");

        let entry_body = &main[entry..];
        let initializers = entry_body
            .find("run_elf_initializers()")
            .expect("entrypoint must run ELF initializers");
        let first_body_marker = entry_body
            .find("Nagi M17 trace: user entry reached")
            .expect("first M17 entry marker");
        let constructors_completed = entry_body
            .find("Nagi M17 trace: ELF constructors completed")
            .expect("M17 constructor completion marker");
        assert!(
            initializers < first_body_marker,
            "ELF constructors must run before the capability-aware entry body"
        );
        assert!(
            initializers < constructors_completed && constructors_completed < first_body_marker,
            "M17 must report completed ELF constructors before its entry marker"
        );
    }

    #[test]
    fn mozjs_configure_uses_supported_freestanding_triplet() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let patch = std::fs::read_to_string(root.join(
            "third_party/mozjs-sys-nagi-patches/0001-nagi-freestanding-configure-target.patch",
        ))
        .expect("mozjs configure patch");
        assert!(patch.contains("--target=x86_64-unknown-nagi"));
        assert!(patch.contains("AR = llvm-ar"));
        assert!(!patch.contains("--target=x86_64-unknown-linux-gnu"));

        let native_os_patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0002-nagi-native-configure-os.patch"),
        )
        .expect("mozjs native Nagi configure patch");
        assert!(native_os_patch.contains("canonical_os = canonical_kernel = \"Nagi\""));
        assert!(native_os_patch.contains("\"Nagi\": \"__NAGI__\""));

        let time_patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0004-nagi-timestamp-platform.patch"),
        )
        .expect("mozjs Nagi timestamp patch");
        assert!(time_patch.contains("CONFIG[\"OS_TARGET\"] == \"Nagi\""));
        assert!(time_patch.contains("/mozglue/misc/TimeStamp_posix.cpp"));

        let compiler_wrapper = std::fs::read_to_string(root.join("tools/nagi-target-cc.sh"))
            .expect("Nagi target compiler wrapper");
        assert!(compiler_wrapper.contains("NAGI_CXX_HEADERS"));
        assert!(compiler_wrapper.contains("-isystem"));
        assert!(compiler_wrapper.contains("-idirafter"));
        assert!(compiler_wrapper.contains("compiler_args+=(-isystem \"$NAGI_CXX_HEADERS\")"));
        assert!(compiler_wrapper
            .contains("if [[ -n \"${NAGI_CXX_HEADERS:-}\" && \"$target_is_cxx\" == true ]]; then"));
        assert!(compiler_wrapper.contains(
            "compiler_args+=(-idirafter \"$repo_root/tools/mesa/nagi-headers\" -idirafter \"$relibc_headers\")"
        ));

        let mozjs_build = std::fs::read_to_string(root.join("third_party/mozjs-sys-nagi/build.rs"))
            .expect("Nagi mozjs_sys build script");
        for boundary in [
            "configure_nagi_bindgen",
            "--target=x86_64-unknown-elf",
            "NAGI_CXX_HEADERS",
            "NAGI_RELIBC_HEADERS",
            "-nostdinc",
        ] {
            assert!(
                mozjs_build.contains(boundary),
                "missing Nagi bindgen boundary: {boundary}"
            );
        }
        let bindgen_patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0012-nagi-bindgen-header-boundary.patch"),
        )
        .expect("mozjs Nagi bindgen boundary patch");
        let cxx_order = bindgen_patch
            .find(".clang_arg(cxx_headers.to_string_lossy().into_owned())")
            .expect("bindgen libc++ header order");
        let resource_order = bindgen_patch
            .find(".clang_arg(resource_dir + \"/include\")")
            .expect("bindgen clang resource order");
        assert!(
            cxx_order < resource_order,
            "bindgen must place libc++ before clang builtin headers"
        );

        let thread_patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0005-nagi-libcxx-thread-api.patch"),
        )
        .expect("mozjs libc++ thread API patch");
        assert!(thread_patch.contains("_LIBCPP_HAS_THREAD_API_PTHREAD=1"));

        let rune_patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0006-nagi-libcxx-rune-table.patch"),
        )
        .expect("mozjs libc++ rune table patch");
        assert!(rune_patch.contains("_LIBCPP_PROVIDES_DEFAULT_RUNE_TABLE=1"));

        let localization_patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0007-nagi-libcxx-no-localization.patch"),
        )
        .expect("mozjs libc++ localization patch");
        assert!(localization_patch.contains("_LIBCPP_HAS_NO_LOCALIZATION=1"));

        let locale_compat_patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0008-nagi-libcxx-locale-compat.patch"),
        )
        .expect("mozjs libc++ locale compatibility patch");
        assert!(locale_compat_patch.contains("-U_LIBCPP_HAS_NO_LOCALIZATION"));

        let malloc_patch =
            std::fs::read_to_string(root.join(
                "third_party/mozjs-sys-nagi-patches/0009-nagi-malloc-usable-size-header.patch",
            ))
            .expect("mozjs Nagi malloc header patch");
        assert!(malloc_patch.contains("defined(__NAGI__)") && malloc_patch.contains("<malloc.h>"));

        let condition_variable_patch = std::fs::read_to_string(root.join(
            "third_party/mozjs-sys-nagi-patches/0010-nagi-condition-variable-clock-api.patch",
        ))
        .expect("mozjs Nagi condition-variable patch");
        assert!(
            condition_variable_patch.contains("defined(__NAGI__)")
                && condition_variable_patch.contains("CLOCK_REALTIME")
        );

        let mmap_fault_handler_patch = std::fs::read_to_string(root.join(
            "third_party/mozjs-sys-nagi-patches/0011-nagi-disable-unsupported-mmap-signal-handler.patch",
        ))
        .expect("mozjs Nagi mmap fault-handler patch");
        assert!(
            mmap_fault_handler_patch.contains("defined(__NAGI__)")
                && mmap_fault_handler_patch.contains("__wasi__) || defined(__NAGI__)")
                && mmap_fault_handler_patch.contains("MmapFaultHandler.cpp")
        );

        let jsglue_malloc_patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0013-nagi-jsglue-malloc-platform.patch"),
        )
        .expect("mozjs Nagi jsglue malloc platform patch");
        assert!(
            jsglue_malloc_patch.contains("defined(__NAGI__)")
                && jsglue_malloc_patch.contains("malloc_usable_size")
        );

        let js_init_trace_patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0014-nagi-m17-js-init-traces.patch"),
        )
        .expect("mozjs M17 JS initialization trace patch");
        for stage in [
            "SpiderMonkey JS_Init entered",
            "SpiderMonkey GC memory initialization started",
            "SpiderMonkey address-limit search started",
            "SpiderMonkey JIT initialization started",
            "SpiderMonkey JIT executable memory map started",
            "SpiderMonkey JIT executable memory map completed",
            "SpiderMonkey JS_Init completed",
        ] {
            assert!(
                js_init_trace_patch.contains(stage),
                "missing SpiderMonkey initialization trace stage: {stage}"
            );
        }
        assert!(js_init_trace_patch.contains("nagi_m17_console_trace(trace_stage"));

        let wasm_init_trace_patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0016-nagi-m17-wasm-init-traces.patch"),
        )
        .expect("mozjs M17 Wasm initialization trace patch");
        for stage in [
            "SpiderMonkey Wasm::Init entered",
            "SpiderMonkey Wasm system page-size lookup completed",
            "SpiderMonkey Wasm huge-memory configuration completed",
            "SpiderMonkey Wasm code-block map allocation completed",
            "SpiderMonkey Wasm static type definitions initialization completed",
            "SpiderMonkey Wasm built-in module functions initialization completed",
            "SpiderMonkey Wasm static tag types initialization completed",
            "SpiderMonkey Wasm::Init completed",
        ] {
            assert!(
                wasm_init_trace_patch.contains(stage),
                "missing SpiderMonkey Wasm initialization trace stage: {stage}"
            );
        }
        assert!(wasm_init_trace_patch.contains("nagi_m17_console_trace(trace_stage"));

        assert!(
            !root
                .join(
                    "third_party/mozjs-sys-nagi-patches/0014-nagi-arraybuffer-unique-ptr-wrapper.patch"
                )
                .exists(),
            "use SpiderMonkey's upstream ArrayBuffer ownership wrapper"
        );

        let stdlib_cbindgen = std::fs::read_to_string(
            root.join("third_party/relibc/src/header/stdlib/cbindgen.toml"),
        )
        .expect("relibc stdlib cbindgen configuration");
        for symbol in [
            "strtod_l",
            "strtof_l",
            "strtoll_l",
            "strtoull_l",
            "strtold_l",
        ] {
            assert!(
                stdlib_cbindgen.contains(symbol),
                "missing locale ABI declaration: {symbol}"
            );
        }

        let nagi_backend = std::fs::read_to_string(root.join("third_party/relibc/src/nagi.rs"))
            .expect("Nagi relibc backend");
        for symbol in ["strtod_l", "strtof_l", "strtoll_l", "strtoull_l"] {
            assert!(
                nagi_backend.contains(symbol),
                "missing Nagi locale ABI: {symbol}"
            );
        }

        let servo_font_patch = std::fs::read_to_string(
            root.join("third_party/servo-patches/0006-nagi-font-platform.patch"),
        )
        .expect("Nagi Servo font platform patch");
        assert!(servo_font_patch.contains("target_os = \"nagi\""));
        assert!(servo_font_patch.contains("components/fonts/platform/nagi/font_list.rs"));
        assert!(servo_font_patch.contains("real FreeType backend"));
        assert!(servo_font_patch.contains("font_identifier.rs"));

        let nagi_mmap = std::fs::read_to_string(root.join("crates/nagi-abi/src/lib.rs"))
            .expect("Nagi mmap ABI");
        assert!(nagi_mmap.contains("SYS_MEMORY_MAP_AT: u64 = 27"));
        assert!(nagi_mmap.contains("PROT_READ: u64 = 0x4"));
        let relibc_nagi = std::fs::read_to_string(root.join("third_party/relibc/src/nagi.rs"))
            .expect("Nagi relibc mmap adapter");
        assert!(relibc_nagi.contains("nagi_posix_mmap_at"));
        assert!(relibc_nagi.contains("MAP_FIXED: c_int = 0x0010"));
        let pthread_backend =
            std::fs::read_to_string(root.join("third_party/relibc/src/header/pthread/mod.rs"))
                .expect("Nagi relibc pthread adapter");
        assert!(pthread_backend.contains("pthread_setname_np"));
        assert!(pthread_backend.contains("pthread_getname_np"));

        let mman_header = std::fs::read_to_string(root.join("tools/mesa/nagi-headers/sys/mman.h"))
            .expect("Nagi Mesa mmap header overlay");
        assert!(mman_header.contains("#define PROT_NONE 0x0000"));
        assert!(mman_header.contains("#define MAP_FIXED 0x0010"));
    }

    #[test]
    fn mozjs_wasm_static_type_init_patch_traces_allocator_and_canonicalization() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let patch = std::fs::read_to_string(root.join(
            "third_party/mozjs-sys-nagi-patches/0017-nagi-m17-wasm-static-type-traces.patch",
        ))
        .expect("mozjs M17 Wasm static-type trace patch");
        for stage in [
            "SpiderMonkey Wasm TypeContext allocation completed",
            "SpiderMonkey Wasm array MutI16 type creation completed",
            "SpiderMonkey Wasm exception parameter append completed",
            "SpiderMonkey Wasm exception tag type creation completed",
            "SpiderMonkey Wasm canonical type-set lock acquired",
            "SpiderMonkey Wasm canonical type-set insertion started",
            "SpiderMonkey Wasm RecGroup hash started",
            "SpiderMonkey Wasm RecGroup hash completed",
            "SpiderMonkey Wasm TypeIdSet lookupForAdd started",
            "SpiderMonkey Wasm TypeIdSet lookupForAdd completed",
            "SpiderMonkey Wasm TypeIdSet HashSet add started",
            "SpiderMonkey Wasm TypeIdSet table pod_malloc started",
            "SpiderMonkey Wasm TypeIdSet table pod_malloc completed",
            "SpiderMonkey Wasm TypeIdSet table slot initialization started",
            "SpiderMonkey Wasm TypeIdSet table slot initialization completed",
            "SpiderMonkey Wasm TypeIdSet HashTable createTable started",
            "SpiderMonkey Wasm TypeIdSet HashTable createTable completed",
            "SpiderMonkey Wasm TypeIdSet HashTable changeTableSize started",
            "SpiderMonkey Wasm TypeIdSet HashTable changeTableSize completed",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot started",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot completed",
            "SpiderMonkey Wasm TypeIdSet setLive started",
            "SpiderMonkey Wasm TypeIdSet setLive completed",
            "SpiderMonkey Wasm TypeIdSet HashSet add completed",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot primary index computed",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot primary slot computed",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot primary slot state index=0x",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot primary liveness read started",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot primary key hash=0x",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot primary slot is live",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot primary slot is free",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot collision path entered",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot hash2 computed",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot first collision mark completed",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot first probe slot computed",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot first probe liveness read started",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot first probe slot is live",
            "SpiderMonkey Wasm TypeIdSet findNonLiveSlot first probe slot is free",
            "SpiderMonkey Wasm canonical type-set insertion completed",
            "SpiderMonkey Wasm StaticTypeDefs::init completed",
        ] {
            assert!(
                patch.contains(stage),
                "missing SpiderMonkey static-type trace stage: {stage}"
            );
        }
        assert!(patch.contains("#if defined(__NAGI__)"));
        assert!(patch.contains("+  using TypeIdSetAllocPolicy = SystemAllocPolicy;"));
        assert!(patch.contains("nagi_m17_console_trace(trace_stage"));
        assert!(patch.contains(
            "-  ExclusiveData<TypeIdSet>::Guard locked = typeIdSet.lock();\n+  NAGI_M17_TRACE(\"SpiderMonkey Wasm canonical type-set lock started\");\n+  ExclusiveData<TypeIdSet>::Guard locked = typeIdSet.lock();"
        ));
        assert!(patch.contains(
            "-    Set::AddPtr p = set_.lookupForAdd(recGroup);\n+    NAGI_M17_TRACE(\"SpiderMonkey Wasm TypeIdSet lookupForAdd started\");\n+    Set::AddPtr p = set_.lookupForAdd(recGroup);\n+    NAGI_M17_TRACE(\"SpiderMonkey Wasm TypeIdSet lookupForAdd completed\");"
        ));
        assert!(patch.contains(
            "-    aPtr.mSlot.setLive(aPtr.mKeyHash, std::forward<Args>(aArgs)...);\n+    if constexpr (requires { AllocPolicy::traceM17SetLiveStarted(); }) {"
        ));
        assert!(patch.contains("requires { AllocPolicy::traceM17SetLiveStarted(); }"));
        assert!(patch.contains("const HashNumber primaryKeyHash = *slot.mKeyHash;"));
        assert!(patch.contains("const bool primarySlotIsLive = Slot::isLiveHash(primaryKeyHash);"));
        assert!(patch.contains("+      char indexMessage[128];"));
        assert!(patch.contains("+      char addressMessage[96];"));
        let find_non_live = patch
            .find("  Slot findNonLiveSlot(HashNumber aKeyHash) {")
            .expect("findNonLiveSlot patch context");
        let find_non_live_patch = &patch[find_non_live..];
        let ordered_stages = [
            "traceM17FindNonLiveSlotPrimaryIndexComputed",
            "traceM17FindNonLiveSlotPrimarySlotComputed",
            "traceM17FindNonLiveSlotPrimarySlotState",
            "traceM17FindNonLiveSlotPrimaryLivenessReadStarted",
            "traceM17FindNonLiveSlotPrimaryHashValue",
            "traceM17FindNonLiveSlotPrimarySlotIsLive",
            "traceM17FindNonLiveSlotPrimarySlotIsFree",
            "traceM17FindNonLiveSlotCollisionPathEntered",
            "traceM17FindNonLiveSlotHash2Computed",
            "traceM17FindNonLiveSlotFirstCollisionMarkCompleted",
            "traceM17FindNonLiveSlotFirstProbeSlotComputed",
            "traceM17FindNonLiveSlotFirstProbeLivenessReadStarted",
            "traceM17FindNonLiveSlotFirstProbeSlotIsLive",
            "traceM17FindNonLiveSlotFirstProbeSlotIsFree",
        ];
        let mut previous = 0;
        for stage in ordered_stages {
            let position = find_non_live_patch
                .find(stage)
                .unwrap_or_else(|| panic!("missing ordered TypeIdSet probe checkpoint: {stage}"));
            assert!(
                position > previous,
                "TypeIdSet probe checkpoint is out of order: {stage}"
            );
            previous = position;
        }
    }

    #[test]
    fn mozjs_m17_js_context_patch_traces_context_creation_stages() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0018-nagi-m17-js-context-traces.patch"),
        )
        .expect("mozjs M17 JSContext creation trace patch");
        for stage in [
            "SpiderMonkey JS_NewContext entered",
            "SpiderMonkey JS_NewContext dispatch started",
            "SpiderMonkey js::NewContext entered",
            "SpiderMonkey JSRuntime allocation started",
            "SpiderMonkey JSRuntime allocation completed",
            "SpiderMonkey JSContext allocation started",
            "SpiderMonkey JSContext allocation completed",
            "SpiderMonkey JSContext initialization started",
            "SpiderMonkey JSContext initialization completed",
            "SpiderMonkey JSRuntime initialization started",
            "SpiderMonkey JSRuntime initialization completed",
            "SpiderMonkey js::NewContext returned",
            "SpiderMonkey JS_NewContext returned",
        ] {
            assert!(
                patch.contains(stage),
                "missing JSContext trace stage: {stage}"
            );
        }
        assert!(patch.contains("#if defined(__NAGI__)"));
        assert!(patch.contains("NAGI_M17_TRACE(stage)"));
        assert!(patch.contains("nagi_m17_console_trace(trace_stage"));
    }

    #[test]
    fn mozjs_m17_js_runtime_patch_traces_runtime_and_helper_initialization() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let patch =
            std::fs::read_to_string(root.join(
                "third_party/mozjs-sys-nagi-patches/0019-nagi-m17-js-runtime-init-traces.patch",
            ))
            .expect("mozjs M17 JSRuntime initialization trace patch");
        for stage in [
            "SpiderMonkey JSRuntime init entered",
            "SpiderMonkey JSRuntime extra-thread policy checked",
            "SpiderMonkey JSRuntime helper-thread initialization started",
            "SpiderMonkey JSRuntime helper-thread initialization completed",
            "SpiderMonkey JSRuntime GC initialization started",
            "SpiderMonkey JSRuntime GC initialization completed",
            "SpiderMonkey JSRuntime number-state initialization started",
            "SpiderMonkey JSRuntime number-state initialization completed",
            "SpiderMonkey JSRuntime time-zone reset started",
            "SpiderMonkey JSRuntime time-zone reset completed",
            "SpiderMonkey JSRuntime set-prop cache allocation started",
            "SpiderMonkey JSRuntime set-prop cache allocation completed",
            "SpiderMonkey helper-thread state initialization entered",
            "SpiderMonkey helper-thread lock acquisition started",
            "SpiderMonkey helper-thread lock acquisition completed",
            "SpiderMonkey internal helper-pool initialization started",
            "SpiderMonkey internal helper-pool initialization completed",
            "SpiderMonkey helper-thread count initialization started",
            "SpiderMonkey helper-thread count initialization completed",
            "SpiderMonkey helper-thread pool allocation started",
            "SpiderMonkey helper-thread pool allocation completed",
            "SpiderMonkey helper-thread creation started",
            "SpiderMonkey helper-thread creation completed",
            "SpiderMonkey helper-thread main entered",
            "SpiderMonkey helper-thread main lock acquisition completed",
        ] {
            assert!(
                patch.contains(stage),
                "missing SpiderMonkey runtime trace stage: {stage}"
            );
        }
        assert!(patch.contains("#if defined(__NAGI__)"));
        assert!(patch.contains("nagi_m17_console_trace(trace_stage"));
    }

    #[test]
    fn mozjs_m17_gc_runtime_patch_traces_initialization_stages() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let patch =
            std::fs::read_to_string(root.join(
                "third_party/mozjs-sys-nagi-patches/0020-nagi-m17-gc-runtime-init-traces.patch",
            ))
            .expect("mozjs M17 GC runtime initialization trace patch");
        for stage in [
            "SpiderMonkey GC runtime initialization entered",
            "SpiderMonkey GC initialization-state assertion started",
            "SpiderMonkey GC initialization-state assertion completed",
            "SpiderMonkey GC system-page-size assertion started",
            "SpiderMonkey GC system-page-size assertion completed",
            "SpiderMonkey GC arena static assertions started",
            "SpiderMonkey GC arena static assertions completed",
            "SpiderMonkey GC arena lookup-table checks started",
            "SpiderMonkey GC arena lookup-table checks completed",
            "SpiderMonkey GC thread-context initialization started",
            "SpiderMonkey GC thread-context initialization completed",
            "SpiderMonkey GC helper-thread count update started",
            "SpiderMonkey GC helper-thread count update completed",
            "SpiderMonkey GC marker vector resize started",
            "SpiderMonkey GC marker vector resize completed",
            "SpiderMonkey GC background-allocation lock acquisition started",
            "SpiderMonkey GC background-allocation lock acquisition completed",
            "SpiderMonkey GC nursery initialization started",
            "SpiderMonkey GC nursery initialization completed",
            "SpiderMonkey GC marker initialization started",
            "SpiderMonkey GC marker initialization completed",
            "SpiderMonkey GC sweep-action initialization started",
            "SpiderMonkey GC sweep-action initialization completed",
            "SpiderMonkey GC atoms-zone allocation started",
            "SpiderMonkey GC atoms-zone initialization started",
            "SpiderMonkey GC atoms-zone initialization completed",
            "SpiderMonkey GC zones-vector reserve started",
            "SpiderMonkey GC runtime initialization completed",
        ] {
            assert!(
                patch.contains(stage),
                "missing SpiderMonkey GC runtime trace stage: {stage}"
            );
        }
        assert!(patch.contains("#if defined(__NAGI__)"));
        assert!(patch.contains("nagi_m17_console_trace(trace_stage"));
    }

    #[test]
    fn mozjs_m17_nursery_patch_traces_initialization_stages() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0021-nagi-m17-nursery-init-traces.patch"),
        )
        .expect("mozjs M17 nursery initialization trace patch");
        for stage in [
            "SpiderMonkey nursery profiling configuration started",
            "SpiderMonkey nursery report-stats configuration started",
            "SpiderMonkey nursery pretenuring configuration started",
            "SpiderMonkey nursery sweep-task allocation started",
            "SpiderMonkey nursery decommit-task allocation started",
            "SpiderMonkey nursery StoreBuffer enable started",
            "SpiderMonkey nursery first-chunk initialization started",
            "SpiderMonkey nursery initial capacity set started",
            "SpiderMonkey nursery decommit chunk reservation started",
            "SpiderMonkey nursery first chunk allocation started",
            "SpiderMonkey nursery to-space chunk-vector reserve started",
            "SpiderMonkey nursery GC chunk acquisition started",
            "SpiderMonkey GC arena chunk allocation started",
            "SpiderMonkey GC aligned-page mapping started",
            "SpiderMonkey GC base memory mapping started",
        ] {
            assert!(
                patch.contains(stage),
                "missing SpiderMonkey nursery trace stage: {stage}"
            );
        }
        assert!(patch.contains("#if defined(__NAGI__)"));
        assert!(patch.contains("nagi_m17_console_trace(trace_stage"));
    }

    #[test]
    fn nagi_init_rescans_real_mozjs_archives_in_m17_link() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let build_script = std::fs::read_to_string(root.join("user/nagi-init/build.rs"))
            .expect("nagi-init build script");
        let m17_guard = build_script
            .find("if env::var_os(\"CARGO_FEATURE_M17_SERVO\").is_some()")
            .expect("M17-only target link block");
        let required = [
            "cargo:rustc-link-arg-bin=nagi-init=-Bstatic",
            "cargo:rustc-link-arg-bin=nagi-init=--start-group",
            "cargo:rustc-link-arg-bin=nagi-init=-ljs_static",
            "cargo:rustc-link-arg-bin=nagi-init=-ljsapi",
            "cargo:rustc-link-arg-bin=nagi-init=-ljsglue",
            "cargo:rustc-link-arg-bin=nagi-init=--end-group",
            "cargo:rustc-link-arg-bin=nagi-init=-Bdynamic",
        ];
        let mut previous = m17_guard;
        for directive in required {
            let position = build_script
                .find(directive)
                .unwrap_or_else(|| panic!("missing final MozJS link directive: {directive}"));
            assert!(
                position > previous,
                "MozJS archive group directive is out of order: {directive}"
            );
            previous = position;
        }
    }

    #[test]
    fn m18_target_link_supplies_servo_time_and_mesa_sort_abis() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let build_script = std::fs::read_to_string(root.join("user/nagi-init/build.rs"))
            .expect("nagi-init build script");
        let target_libc = std::fs::read_to_string(root.join("third_party/relibc/src/nagi.rs"))
            .expect("Nagi relibc backend");

        for symbol in ["localtime", "gmtime", "qsort_r"] {
            assert!(
                build_script.contains(&format!("\"{symbol}\"")),
                "nagi-init must retain the target archive provider for {symbol}"
            );
            assert!(
                target_libc.contains(&format!("pub unsafe extern \"C\" fn {symbol}(")),
                "the Nagi target libc must export the {symbol} ABI"
            );
        }
    }

    #[test]
    fn m17_cpp_build_scripts_use_the_nagi_target_wrapper() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let commands = std::fs::read_to_string(root.join("tools/nagi-cli/src/commands.rs"))
            .expect("Nagi CLI build command");
        for variable in [
            "CC_x86_64_unknown_nagi_user",
            "CXX_x86_64_unknown_nagi_user",
        ] {
            assert!(
                commands.contains(variable),
                "M17 CLI must route {variable} through the Nagi target wrapper"
            );
        }
        assert!(commands.contains("(\"HOST_CC\", Path::new(\"cc\"))"));
        assert!(commands.contains("(\"HOST_CXX\", Path::new(\"c++\"))"));
        assert!(commands.contains("(\"NAGI_CXX_HEADERS\", cxx_headers.as_path())"));

        let wrapper = std::fs::read_to_string(root.join("tools/nagi-target-cc.sh"))
            .expect("Nagi target compiler wrapper");
        for setting in [
            "*.cc|*.cpp|*.cxx|*.c++|*.C|*.mm) target_is_cxx=true",
            "-D_LIBCPP_HAS_THREAD_API_PTHREAD=1",
            "-D_LIBCPP_PROVIDES_DEFAULT_RUNE_TABLE=1",
            "-frtti) target_rtti_enabled=true",
            "-fno-rtti) target_rtti_enabled=false",
            "cxx_runtime_flags=(-fno-exceptions)",
            "cxx_runtime_flags+=(-fno-rtti)",
        ] {
            assert!(
                wrapper.contains(setting),
                "C++ target wrapper is missing libc++ setting: {setting}"
            );
        }

        let workflow = std::fs::read_to_string(root.join(".github/workflows/ci.yml"))
            .expect("Nagi CI workflow");
        let cxx_setting = workflow
            .lines()
            .find(|line| line.contains("CXX_x86_64_unknown_nagi_user:"))
            .expect("CI target C++ compiler setting");
        assert!(cxx_setting.contains("tools/nagi-target-cc.sh"));
    }

    #[test]
    fn mozjs_nagi_target_suppresses_all_host_cxx_runtime_links() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0003-nagi-no-host-cxx-runtime.patch"),
        )
        .expect("mozjs host C++ runtime patch");

        assert!(
            patch.contains("builder.cpp_link_stdlib(None);"),
            "cc-rs must not infer stdc++ for the Nagi target"
        );
        assert!(
            patch.contains("if target.contains(\"nagi-user\")"),
            "MozJS's explicit link boundary must recognize the Nagi user target"
        );
    }

    #[test]
    fn mozjs_random_bytes_use_nagi_virtio_entropy() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0015-nagi-virtio-rng.patch"),
        )
        .expect("MozJS Nagi VirtIO RNG patch");
        for contract in [
            "diff --git a/mozjs/mfbt/RandomNum.cpp",
            "#if defined(__NAGI__)",
            "extern \"C\" int __nagi_random_fill(void* buffer, size_t length);",
            "return __nagi_random_fill(aBuffer, aLength) == 0;",
        ] {
            assert!(
                patch.contains(contract),
                "missing MozJS Nagi entropy contract: {contract}"
            );
        }

        let libnagi =
            std::fs::read_to_string(root.join("user/libnagi/src/lib.rs")).expect("libnagi source");
        assert!(libnagi.contains("pub unsafe extern \"C\" fn __nagi_random_fill"));
        assert!(libnagi.contains("pub unsafe extern \"C\" fn __nagi_std_random_fill"));
        assert!(libnagi.contains("unsafe { __nagi_random_fill(destination, length) }"));
        assert!(libnagi.contains("inlateout(\"rax\") result"));
        assert!(libnagi.contains("SYS_RANDOM_GET"));
    }

    #[test]
    fn mozjs_nagi_disables_scattershot_for_bounded_first_fit_mappings() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0022-nagi-m17-bounded-gc-mapping.patch"),
        )
        .expect("MozJS Nagi bounded GC mapping patch");

        for contract in [
            "diff --git a/mozjs/js/src/gc/Memory.cpp",
            "bool UsingScattershotAllocator()",
            "#if defined(__NAGI__)",
            "Nagi's bounded first-fit arena ignores address hints",
            "return numAddressBits >= MinAddressBitsForRandomAlloc;",
        ] {
            assert!(
                patch.contains(contract),
                "missing MozJS Nagi bounded-mapping contract: {contract}"
            );
        }
    }

    #[test]
    fn mozjs_m17_alignment_trace_patch_brackets_mapping_operations() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0023-nagi-m17-alignment-traces.patch"),
        )
        .expect("MozJS Nagi M17 alignment trace patch");

        for contract in [
            "diff --git a/mozjs/js/src/gc/Memory.cpp",
            "#if defined(__NAGI__)",
            "bool traceM17GCChunk = false",
            "constexpr bool traceM17GCChunk = false;",
            "length == ChunkSize && alignment == ChunkSize",
            "MapMemoryAt(regionEnd, offsetUpper, traceM17GCChunk)",
            "MapMemoryAt(lowerStart, offsetLower, traceM17GCChunk)",
            "SpiderMonkey GC exact-hint mmap started",
            "SpiderMonkey GC exact-hint mmap completed",
            "SpiderMonkey GC mismatched-hint unmap started",
            "SpiderMonkey GC mismatched-hint unmap completed",
            "SpiderMonkey GC chunk alignment entered",
            "SpiderMonkey GC upward hint mapping started",
            "SpiderMonkey GC upward hint mapping completed",
            "SpiderMonkey GC upward prefix unmap started",
            "SpiderMonkey GC upward prefix unmap completed",
            "SpiderMonkey GC lower hint mapping started",
            "SpiderMonkey GC lower hint mapping completed",
            "SpiderMonkey GC lower tail unmap started",
            "SpiderMonkey GC lower tail unmap completed",
            "SpiderMonkey GC replacement mapping started",
            "SpiderMonkey GC replacement mapping completed",
        ] {
            assert!(
                patch.contains(contract),
                "missing MozJS Nagi M17 alignment trace contract: {contract}"
            );
        }

        for (start, complete) in [
            (
                "SpiderMonkey GC exact-hint mmap started",
                "SpiderMonkey GC exact-hint mmap completed",
            ),
            (
                "SpiderMonkey GC mismatched-hint unmap started",
                "SpiderMonkey GC mismatched-hint unmap completed",
            ),
            (
                "SpiderMonkey GC upward hint mapping started",
                "SpiderMonkey GC upward hint mapping completed",
            ),
            (
                "SpiderMonkey GC upward prefix unmap started",
                "SpiderMonkey GC upward prefix unmap completed",
            ),
            (
                "SpiderMonkey GC lower hint mapping started",
                "SpiderMonkey GC lower hint mapping completed",
            ),
            (
                "SpiderMonkey GC lower tail unmap started",
                "SpiderMonkey GC lower tail unmap completed",
            ),
            (
                "SpiderMonkey GC replacement mapping started",
                "SpiderMonkey GC replacement mapping completed",
            ),
        ] {
            let started = patch.find(start).expect("alignment start marker");
            let completed = patch.find(complete).expect("alignment completion marker");
            assert!(started < completed, "misordered trace pair: {start}");
        }
    }
}
