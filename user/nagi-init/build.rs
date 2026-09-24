use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=NAGI_M16_PACKAGE");
    println!("cargo:rerun-if-env-changed=NAGI_TARGET_CLANG");
    println!("cargo:rerun-if-env-changed=NAGI_MESA_BUILD");
    println!("cargo:rerun-if-env-changed=NAGI_CXX_HEADERS");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let package_output = out_dir.join("m16-package.xapp");
    if env::var_os("CARGO_FEATURE_M16_PACKAGE").is_some() {
        let package = env::var_os("NAGI_M16_PACKAGE")
            .map(PathBuf::from)
            .expect("M16 requires NAGI_M16_PACKAGE from the package service build");
        println!("cargo:rerun-if-changed={}", package.display());
        let bytes = fs::read(&package).expect("read M16 package artifact");
        if bytes.is_empty() || bytes.len() > 8192 {
            panic!("M16 package artifact is empty or oversized");
        }
        fs::write(package_output, bytes).expect("stage M16 package artifact");
    } else {
        fs::write(package_output, []).expect("write empty M16 package placeholder");
    }

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("nagi") {
        return;
    }

    if env::var_os("CARGO_FEATURE_M17_SERVO").is_some() {
        // Keep rust-lld from truncating the undefined-symbol inventory at its
        // default error cap. This is the authoritative M17 target link; seeing
        // every unresolved symbol lets one CI run expose a whole repair group.
        println!("cargo:rustc-link-arg-bin=nagi-init=--error-limit=0");

        let mesa_build = env::var_os("NAGI_MESA_BUILD")
            .map(PathBuf::from)
            .expect("NAGI_MESA_BUILD must point to the guest Mesa build for m17-servo");
        let mesa_archive_root = mesa_build.parent().unwrap_or(&mesa_build);
        let mesa_archive = mesa_archive_root.join("libnagi_mesa.a");
        if !mesa_archive.is_file() {
            panic!(
                "NAGI_MESA_BUILD does not contain the target-owned Mesa archive: {}",
                mesa_archive.display()
            );
        }
        println!(
            "cargo:rustc-link-search=native={}",
            mesa_archive_root.display()
        );
        // Keep archive extraction selective. The aggregated Mesa archive contains
        // static dependencies that may also be reachable through another archive;
        // forcing every member out creates duplicate Softpipe symbols at final
        // link. The real EGL/Softpipe symbols referenced by Servo are still
        // resolved from this target-owned archive normally. The state tracker
        // entry points below can be reached only through a later archive
        // member in rust-lld's single archive scan, so explicitly seed the
        // real Mesa glthread function through the Nagi-owned link anchor and
        // seed the state tracker function directly. This remains selective
        // archive extraction; it does not force every Mesa member out or
        // import a host graphics implementation.
        println!(
            "cargo:rustc-link-arg-bin=nagi-init=--undefined=nagi_mesa_glthread_finish_link_anchor"
        );
        println!("cargo:rustc-link-arg-bin=nagi-init=--undefined=st_context_flush");
        // The target-owned archive intentionally preserves Mesa's static
        // dependency graph instead of forcing every object into the image.
        // Seed the real Softpipe loader/winsys entry points whose providers
        // occur after their users in that single archive scan.
        for symbol in [
            "sw_screen_create_vk",
            "wrapper_sw_winsys_wrap_pipe_screen",
            "null_sw_create",
            // Shader compiler and preprocessor providers are later members in
            // the combined target Mesa archive. Seed the real implementations
            // so the archive scan retains GLSL and SPIR-V compilation.
            "glcpp_preprocess",
            "spirv_to_nir",
            "spirv_verify_gl_specialization_constants",
        ] {
            println!("cargo:rustc-link-arg-bin=nagi-init=--undefined={symbol}");
        }
        // The pinned MozJS build emits the real three-argument UniquePtr
        // ArrayBuffer forwarding wrapper into the jsglue archive. Seed its
        // exact Itanium ABI symbol before the archive scan so rust-lld extracts
        // that object instead of leaving the inline JSAPI wrapper unresolved.
        println!(
            "cargo:rustc-link-arg-bin=nagi-init=--undefined=_ZN2JS26NewArrayBufferWithContentsEP9JSContextmSt10unique_ptrIvNS_10FreePolicyEE"
        );
        // These exact SpiderMonkey providers live in the later js_static
        // archive members; seed their Itanium ABI names so the documented
        // MozJS archive rescan can extract the real implementations.
        for symbol in [
            "_ZN2JS21RestoreMicroTaskQueueEP9JSContextNSt3__110unique_ptrINS_19SavedMicroTaskQueueENS_12DeletePolicyIS4_EEEE",
            "_ZN2JS22InitAsyncTaskCallbacksEP9JSContextPFbPvONSt3__110unique_ptrINS_12DispatchableENS_12DeletePolicyIS5_EEEEEPFbS2_S9_jEPFvS2_PS5_ESG_S2_",
            "_ZN2JS12Dispatchable3RunEP9JSContextONSt3__110unique_ptrIS0_NS_12DeletePolicyIS0_EEEENS0_17MaybeShuttingDownE",
        ] {
            println!("cargo:rustc-link-arg-bin=nagi-init=--undefined={symbol}");
        }
        // The libc++ pthread backend used by the pinned Servo/Mesa graph
        // reaches these real relibc entry points from the Nagi-owned C++
        // runtime object. Seed only those providers before the relibc archive
        // scan; this is selective archive extraction, not a host fallback or
        // whole-archive import.
        for symbol in [
            "pthread_mutex_lock",
            "pthread_mutex_trylock",
            "pthread_mutex_unlock",
            "pthread_mutex_destroy",
            "pthread_cond_signal",
            "pthread_cond_broadcast",
            "pthread_cond_wait",
            "pthread_cond_destroy",
            // Rust std queries the real current guest stack through the
            // Nagi-owned POSIX attribute bridge. Seed these providers before
            // the static archive scan, just like the other target pthread
            // entry points above.
            "pthread_getattr_np",
            "pthread_attr_getstack",
            // This ctype provider lives in the Nagi relibc backend. Seed it
            // before the static archive scan because Mesa's C++ archive can
            // introduce the use after relibc's normal extraction point.
            "islower",
            "nearbyint",
            "nearbyintf",
            "mktime",
            "gmtime_r",
            "readlink",
            // These are real relibc/POSIX providers for the complete #157
            // target link inventory. They can be introduced by later Mesa,
            // MozJS, and SQLite archive members, so seed their exact C ABI
            // names before the one-pass Rust static archive scan.
            "remove",
            "madvise",
            "getrusage",
            "fsync",
            "ftruncate",
            "fchmod",
            "fchown",
            "utimes",
            "__fpclassifyf",
            "getc",
            "ferror",
            "clearerr",
            "stdin",
            "fileno",
            "strtok",
            "strtok_r",
            "llabs",
            "__program_invocation_short_name",
            "log10",
            "sigfillset",
            "sigdelset",
            "pthread_sigmask",
            "pthread_barrier_init",
            "pthread_barrier_destroy",
            "pthread_barrier_wait",
            "fdopen",
            // The guest has no dynamic loader. These relibc ABI providers
            // fail closed with a per-thread dlerror message and must be
            // extracted when downstream archives introduce their references.
            "dlopen",
            "dlerror",
            "dlclose",
        ] {
            println!("cargo:rustc-link-arg-bin=nagi-init=--undefined={symbol}");
        }
        println!(
            "cargo:rustc-link-arg-bin=nagi-init=--undefined=_ZNSt3__111__call_onceERVmPvPFvS2_E"
        );
        println!("cargo:rustc-link-lib=static=nagi_mesa_roots");
        println!("cargo:rustc-link-lib=static=nagi_mesa");
    }

    let app_directory =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
            .join("..")
            .join("..")
            .join("tests")
            .join("apps");
    let sources = [
        app_directory.join("m13_posix.c"),
        app_directory.join("m13_relibc.c"),
    ];
    for source in &sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }
    let cxx_runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("mesa")
        .join("nagi-cxx-runtime.cpp");
    println!("cargo:rerun-if-changed={}", cxx_runtime.display());

    let compiler = env::var_os("NAGI_TARGET_CLANG")
        .map(PathBuf::from)
        .or_else(|| {
            if cfg!(windows) {
                env::var_os("ProgramFiles").map(|program_files| {
                    PathBuf::from(program_files)
                        .join("LLVM")
                        .join("bin")
                        .join("clang++.exe")
                })
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            PathBuf::from(if cfg!(windows) {
                "clang++.exe"
            } else {
                "clang"
            })
        });
    for source in &sources {
        let stem = source.file_stem().expect("C source stem");
        let output = out_dir.join(stem).with_extension("o");
        let status = Command::new(&compiler)
            .args([
                "--target=x86_64-unknown-none",
                "-x",
                "c",
                "-ffreestanding",
                "-fno-stack-protector",
                "-fno-builtin",
                "-fno-asynchronous-unwind-tables",
                "-fno-exceptions",
                "-fno-rtti",
                "-mcmodel=large",
                "-c",
            ])
            .arg(source)
            .arg("-o")
            .arg(&output)
            .status()
            .unwrap_or_else(|error| panic!("failed to start {}: {error}", compiler.display()));
        if !status.success() {
            panic!("{} failed with {status}", compiler.display());
        }
        println!("cargo:rustc-link-arg-bin=nagi-init={}", output.display());
    }

    let cxx_output = out_dir.join("nagi-cxx-runtime.o");
    let status = Command::new(&compiler)
        .args([
            "--target=x86_64-unknown-none",
            "-x",
            "c++",
            "-ffreestanding",
            "-fno-stack-protector",
            "-fno-builtin",
            "-fno-asynchronous-unwind-tables",
            "-fno-exceptions",
            "-fno-rtti",
            "-nostdinc",
            "-mcmodel=large",
            "-c",
        ])
        .arg(&cxx_runtime)
        .arg("-o")
        .arg(&cxx_output)
        .status()
        .unwrap_or_else(|error| panic!("failed to start {}: {error}", compiler.display()));
    if !status.success() {
        panic!("{} failed with {status}", compiler.display());
    }
    println!(
        "cargo:rustc-link-arg-bin=nagi-init={}",
        cxx_output.display()
    );

    // Some pinned Servo/MozJS objects use libc++ extern-template entrypoints
    // which are normally supplied by libc++.a. Nagi deliberately has no host
    // C++ runtime, so instantiate the exact required algorithms/string method
    // from the target's libc++ headers and provide sleep_for through the real
    // guest POSIX clock bridge.
    let cxx_abi_source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("mesa")
        .join("nagi-libcpp-abi.cpp");
    println!("cargo:rerun-if-changed={}", cxx_abi_source.display());
    let cxx_sort_source = cxx_abi_source.with_file_name("nagi-libcpp-sort.cpp");
    println!("cargo:rerun-if-changed={}", cxx_sort_source.display());
    let target_cc_wrapper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("nagi-target-cc.sh");
    for (source, stem) in [
        (&cxx_abi_source, "nagi-libcpp-abi"),
        (&cxx_sort_source, "nagi-libcpp-sort"),
    ] {
        let output = out_dir.join(format!("{stem}.o"));
        let status = Command::new("bash")
            .arg(&target_cc_wrapper)
            .args([
                "-x",
                "c++",
                "-fno-asynchronous-unwind-tables",
                "-fno-exceptions",
                "-fno-rtti",
                // The custom target triple cannot select libc++'s pthread
                // backend or default rune table on its own. Match the
                // target flags used by the pinned MozJS C++ build.
                "-D_LIBCPP_HAS_THREAD_API_PTHREAD=1",
                "-D_LIBCPP_PROVIDES_DEFAULT_RUNE_TABLE=1",
                "-c",
            ])
            .arg(source)
            .arg("-o")
            .arg(&output)
            .status()
            .unwrap_or_else(|error| panic!("failed to compile libc++ ABI object: {error}"));
        if !status.success() {
            panic!("Nagi libc++ ABI compilation failed with {status}");
        }
        println!("cargo:rustc-link-arg-bin=nagi-init={}", output.display());
    }
}
