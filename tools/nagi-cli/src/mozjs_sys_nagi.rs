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
}
