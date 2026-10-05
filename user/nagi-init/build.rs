use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=NAGI_M16_PACKAGE");
    println!("cargo:rerun-if-env-changed=NAGI_FONT_DIR");
    println!("cargo:rerun-if-env-changed=NAGI_ACCEPTANCE_PACKAGES");
    if env::var_os("CARGO_FEATURE_ISOLATED_PROCESS_ACCEPTANCE").is_some()
        || env::var_os("CARGO_FEATURE_M19_SEARCH_IPC").is_some()
    {
        let packages = env::var_os("NAGI_ACCEPTANCE_PACKAGES")
            .map(PathBuf::from)
            .expect(
                "isolated applications require NAGI_ACCEPTANCE_PACKAGES (signed .xapp directory)",
            );
        println!("cargo:rerun-if-changed={}", packages.display());
        for entry in fs::read_dir(&packages).expect("read acceptance package directory") {
            let path = entry.expect("package entry").path();
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    println!("cargo:rerun-if-env-changed=NAGI_TARGET_CLANG");
    println!("cargo:rerun-if-env-changed=NAGI_MESA_BUILD");
    println!("cargo:rerun-if-env-changed=NAGI_LLAMA_BUILD");
    println!("cargo:rerun-if-env-changed=NAGI_LLAMA_SOURCE");
    println!("cargo:rerun-if-env-changed=NAGI_WHISPER_BUILD");
    println!("cargo:rerun-if-env-changed=NAGI_WHISPER_SOURCE");
    println!("cargo:rerun-if-env-changed=NAGI_M25_WHISPER_PCM_FIXTURE");
    println!("cargo:rerun-if-env-changed=NAGI_M25_WHISPER_EXPECTED_TEXT_FILE");
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

    stage_system_fonts(&out_dir);

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
        // These exact SpiderMonkey providers live in the real MozJS archives.
        // Seed their Itanium ABI names before the final native archive group
        // below so the group extracts their implementations selectively.
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
            "localtime",
            "gmtime",
            "gmtime_r",
            "readlink",
            // Mesa's real printf formatter reaches this POSIX search helper.
            "strpbrk",
            // Mesa's NIR helper archive calls the context-aware POSIX sort;
            // relibc exports the matching Nagi target ABI below.
            "qsort_r",
            // These are real relibc/POSIX providers for the target link
            // inventory. They can be introduced by later Mesa,
            // MozJS, and SQLite archive members, so seed their exact C ABI
            // names before the one-pass Rust static archive scan.
            "ntohs",
            "ntohl",
            "htons",
            "htonl",
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
        // The dependency build script's rustc-link-search paths reach this
        // final binary, but its native archive link arguments do not. The
        // target link therefore needs to name MozJS's real archives here.
        // Rescanning this small group resolves cycles between SpiderMonkey,
        // JSAPI, and Nagi's glue while extracting only referenced objects.
        println!("cargo:rustc-link-arg-bin=nagi-init=-Bstatic");
        println!("cargo:rustc-link-arg-bin=nagi-init=--start-group");
        println!("cargo:rustc-link-arg-bin=nagi-init=-ljs_static");
        println!("cargo:rustc-link-arg-bin=nagi-init=-ljsapi");
        println!("cargo:rustc-link-arg-bin=nagi-init=-ljsglue");
        println!("cargo:rustc-link-arg-bin=nagi-init=--end-group");
        println!("cargo:rustc-link-arg-bin=nagi-init=-Bdynamic");
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

    // ADR 0058: guest llama inference links the target-built libc++ archive.
    // The Nagi runtime then provides only the Itanium ABI boundary that
    // libc++ is configured without, so their definitions never overlap.
    println!("cargo:rerun-if-env-changed=NAGI_LIBCXX_ARCHIVE");
    let libcxx_archive = if env::var_os("CARGO_FEATURE_M20_LLAMA_INFERENCE_ACCEPTANCE").is_some() {
        let archive = env::var_os("NAGI_LIBCXX_ARCHIVE")
            .map(PathBuf::from)
            .expect("NAGI_LIBCXX_ARCHIVE must point to the Nagi-target libc++.a (tools/libcxx)");
        if !archive.is_file() {
            panic!("NAGI_LIBCXX_ARCHIVE is missing: {}", archive.display());
        }
        println!("cargo:rerun-if-changed={}", archive.display());
        Some(archive)
    } else {
        None
    };
    let cxx_output = out_dir.join("nagi-cxx-runtime.o");
    let mut cxx_runtime_command = Command::new(&compiler);
    if libcxx_archive.is_some() {
        cxx_runtime_command.arg("-DNAGI_CXX_RUNTIME_WITH_LIBCXX");
    }
    let status = cxx_runtime_command
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

    if env::var_os("CARGO_FEATURE_M20_LLAMA_LINK_SMOKE").is_some()
        || env::var_os("CARGO_FEATURE_M20_LLAMA_INFERENCE_ACCEPTANCE").is_some()
    {
        let llama_build = env::var_os("NAGI_LLAMA_BUILD")
            .map(PathBuf::from)
            .expect("NAGI_LLAMA_BUILD must point to the Nagi-target llama.cpp build");
        let repository_root =
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
                .join("..")
                .join("..");
        let llama_source = repository_root.join("tools").join("llama");
        let generated_source = env::var_os("NAGI_LLAMA_SOURCE")
            .map(PathBuf::from)
            .unwrap_or_else(|| repository_root.join("out/cache/llama-cpp-nagi"));
        let archives = [
            llama_build.join("src/libllama.a"),
            llama_build.join("ggml/src/libggml.a"),
            llama_build.join("ggml/src/libggml-cpu.a"),
            llama_build.join("ggml/src/libggml-base.a"),
        ];
        for archive in &archives {
            if !archive.is_file() {
                panic!(
                    "NAGI_LLAMA_BUILD is missing a required target archive: {}",
                    archive.display()
                );
            }
            println!("cargo:rerun-if-changed={}", archive.display());
        }
        let llama_header = generated_source.join("include/llama.h");
        let ggml_header = generated_source.join("ggml/include/ggml-backend.h");
        for header in [&llama_header, &ggml_header] {
            if !header.is_file() {
                panic!("pinned llama.cpp header is missing: {}", header.display());
            }
            println!("cargo:rerun-if-changed={}", header.display());
        }
        let target_cc_wrapper = repository_root.join("tools/nagi-target-cc.sh");
        let smoke_object = if env::var_os("CARGO_FEATURE_M20_LLAMA_LINK_SMOKE").is_some() {
            let smoke_source = llama_source.join("nagi-backend-smoke.c");
            println!("cargo:rerun-if-changed={}", smoke_source.display());
            let smoke_object = out_dir.join("nagi-llama-backend-smoke.o");
            let status = Command::new("bash")
                .arg(&target_cc_wrapper)
                .args(["-x", "c", "-fno-asynchronous-unwind-tables", "-c"])
                .arg("-I")
                .arg(generated_source.join("include"))
                .arg("-I")
                .arg(generated_source.join("ggml/include"))
                .arg(&smoke_source)
                .arg("-o")
                .arg(&smoke_object)
                .status()
                .unwrap_or_else(|error| {
                    panic!("failed to compile Nagi llama smoke adapter: {error}")
                });
            if !status.success() {
                panic!("Nagi llama smoke adapter compilation failed with {status}");
            }
            Some(smoke_object)
        } else {
            None
        };
        let provider_object =
            if env::var_os("CARGO_FEATURE_M20_LLAMA_INFERENCE_ACCEPTANCE").is_some() {
                let provider_source = repository_root.join("tools/llama/nagi-provider-adapter.cpp");
                println!("cargo:rerun-if-changed={}", provider_source.display());
                let provider_object = out_dir.join("nagi-llama-provider-adapter.o");
                let status = Command::new("bash")
                    .arg(&target_cc_wrapper)
                    .args([
                        "-x",
                        "c++",
                        "-fno-asynchronous-unwind-tables",
                        "-fno-exceptions",
                        "-fno-rtti",
                        "-c",
                    ])
                    .arg("-I")
                    .arg(generated_source.join("include"))
                    .arg("-I")
                    .arg(generated_source.join("ggml/include"))
                    .arg(&provider_source)
                    .arg("-o")
                    .arg(&provider_object)
                    .status()
                    .unwrap_or_else(|error| {
                        panic!("failed to compile Nagi llama provider adapter: {error}")
                    });
                if !status.success() {
                    panic!("Nagi llama provider adapter compilation failed with {status}");
                }
                Some(provider_object)
            } else {
                None
            };
        // llama.cpp's real CPU backend reaches C/POSIX and math functions from
        // the target relibc archive. Root only those implemented providers so
        // rust-lld extracts them before scanning the static C++ archives.
        for symbol in [
            "stdout",
            "stderr",
            "fflush",
            "snprintf",
            "vsnprintf",
            "fprintf",
            "fputs",
            "printf",
            "ldexpf",
            "log2f",
            "expf",
            "logf",
            "tanhf",
            "expm1f",
            "lroundf",
            "log2",
            "powf",
            "cosf",
            "sinf",
            "atoi",
            "abs",
            "__errno_location",
            "fclose",
            "fdopen",
            "feof",
            "ferror",
            "fopen",
            "fread",
            "fseeko",
            "ftello",
            "isascii",
            "isprint",
            "isspace",
            "isxdigit",
            "memchr",
            "pow",
            "puts",
            "qsort",
            "sscanf",
            "strerror",
            "strncpy",
            "strstr",
            "strtol",
            "toupper",
            "wmemcmp",
            "wmemchr",
            "dlopen",
            "dlclose",
            "dlsym",
            "dlerror",
            "tolower",
            "strcmp",
            "erff",
        ] {
            println!("cargo:rustc-link-arg-bin=nagi-init=--undefined={symbol}");
        }

        // Without the target libc++ archive (the link smoke), the pinned CPU
        // feature provider's libc++ string ABI comes from a Nagi adapter
        // compiled against the configured target headers.
        if libcxx_archive.is_none() {
            let cxx_abi_source = llama_source.join("nagi-libcpp-llama.cpp");
            println!("cargo:rerun-if-changed={}", cxx_abi_source.display());
            let cxx_abi_object = out_dir.join("nagi-libcpp-llama.o");
            let status = Command::new("bash")
                .arg(&target_cc_wrapper)
                .args([
                    "-x",
                    "c++",
                    "-fno-asynchronous-unwind-tables",
                    "-fno-exceptions",
                    "-fno-rtti",
                    "-c",
                ])
                .arg(&cxx_abi_source)
                .arg("-o")
                .arg(&cxx_abi_object)
                .status()
                .unwrap_or_else(|error| {
                    panic!("failed to compile llama libc++ ABI adapter: {error}")
                });
            if !status.success() {
                panic!("Nagi llama libc++ ABI adapter compilation failed with {status}");
            }
            println!(
                "cargo:rustc-link-arg-bin=nagi-init={}",
                cxx_abi_object.display()
            );
        }

        if let Some(provider_object) = provider_object {
            println!(
                "cargo:rustc-link-arg-bin=nagi-init={}",
                provider_object.display()
            );
        }

        println!("cargo:rustc-link-arg-bin=nagi-init=--error-limit=0");
        println!("cargo:rustc-link-arg-bin=nagi-init=--gc-sections");
        if let Some(smoke_object) = smoke_object {
            println!(
                "cargo:rustc-link-arg-bin=nagi-init={}",
                smoke_object.display()
            );
        }
        println!("cargo:rustc-link-arg-bin=nagi-init=-Bstatic");
        println!("cargo:rustc-link-arg-bin=nagi-init=--start-group");
        for archive in archives.iter().chain(libcxx_archive.as_ref()) {
            println!("cargo:rustc-link-arg-bin=nagi-init={}", archive.display());
        }
        println!("cargo:rustc-link-arg-bin=nagi-init=--end-group");
        println!("cargo:rustc-link-arg-bin=nagi-init=-Bdynamic");
    }

    if env::var_os("CARGO_FEATURE_M25_WHISPER_INFERENCE_ACCEPTANCE").is_some() {
        let whisper_build = env::var_os("NAGI_WHISPER_BUILD")
            .map(PathBuf::from)
            .expect("NAGI_WHISPER_BUILD must point to the Nagi-target whisper.cpp build");
        let repository_root =
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
                .join("..")
                .join("..");
        let whisper_source = env::var_os("NAGI_WHISPER_SOURCE")
            .map(PathBuf::from)
            .unwrap_or_else(|| repository_root.join("out/cache/whisper-cpp-nagi"));
        let archives = [
            whisper_build.join("src/libwhisper.a"),
            whisper_build.join("ggml/src/libggml.a"),
            whisper_build.join("ggml/src/libggml-cpu.a"),
            whisper_build.join("ggml/src/libggml-base.a"),
        ];
        for archive in &archives {
            if !archive.is_file() {
                panic!(
                    "NAGI_WHISPER_BUILD is missing a required target archive: {}",
                    archive.display()
                );
            }
            println!("cargo:rerun-if-changed={}", archive.display());
        }
        let whisper_header = whisper_source.join("include/whisper.h");
        if !whisper_header.is_file() {
            panic!(
                "pinned whisper.cpp header is missing: {}",
                whisper_header.display()
            );
        }
        println!("cargo:rerun-if-changed={}", whisper_header.display());

        let pcm_fixture = env::var_os("NAGI_M25_WHISPER_PCM_FIXTURE")
            .map(PathBuf::from)
            .expect("NAGI_M25_WHISPER_PCM_FIXTURE must identify raw mono S16LE at 16 kHz");
        println!("cargo:rerun-if-changed={}", pcm_fixture.display());
        let pcm = fs::read(&pcm_fixture).expect("read M25 Whisper PCM fixture");
        const MAX_PCM_BYTES: usize = 1_048_576;
        if pcm.is_empty() || pcm.len() > MAX_PCM_BYTES || !pcm.len().is_multiple_of(2) {
            panic!("M25 Whisper PCM fixture must be nonempty, even-sized, and at most 1 MiB");
        }
        fs::write(out_dir.join("m25-whisper-input.pcm"), &pcm)
            .expect("stage M25 Whisper PCM fixture");

        let expected_text_file = env::var_os("NAGI_M25_WHISPER_EXPECTED_TEXT_FILE")
            .map(PathBuf::from)
            .expect("NAGI_M25_WHISPER_EXPECTED_TEXT_FILE must identify expected Japanese text");
        println!("cargo:rerun-if-changed={}", expected_text_file.display());
        let expected_text = fs::read_to_string(&expected_text_file)
            .expect("read expected M25 Whisper Japanese text");
        if expected_text.is_empty()
            || expected_text.len() > 1024
            || expected_text
                .chars()
                .any(|character| matches!(character, '\n' | '\r' | '\0'))
        {
            panic!("expected M25 Whisper text must be 1–1024 UTF-8 bytes without line breaks");
        }
        fs::write(
            out_dir.join("m25-whisper-expected.txt"),
            expected_text.as_bytes(),
        )
        .expect("stage expected M25 Whisper text");

        let adapter_source = repository_root.join("tools/whisper/nagi-provider-adapter.cpp");
        let adapter_object = out_dir.join("nagi-whisper-provider-adapter.o");
        println!("cargo:rerun-if-changed={}", adapter_source.display());
        let target_cc_wrapper = repository_root.join("tools/nagi-target-cc.sh");
        let status = Command::new("bash")
            .arg(&target_cc_wrapper)
            .args([
                "-x",
                "c++",
                "-fno-asynchronous-unwind-tables",
                "-fno-exceptions",
                "-fno-rtti",
                "-c",
            ])
            .arg("-I")
            .arg(whisper_source.join("include"))
            .arg("-I")
            .arg(whisper_source.join("ggml/include"))
            .arg(&adapter_source)
            .arg("-o")
            .arg(&adapter_object)
            .status()
            .unwrap_or_else(|error| panic!("failed to compile Nagi Whisper adapter: {error}"));
        if !status.success() {
            panic!("Nagi Whisper provider adapter compilation failed with {status}");
        }

        let cxx_abi_source = repository_root.join("tools/whisper/nagi-libcpp-whisper.cpp");
        let cxx_abi_object = out_dir.join("nagi-whisper-libcpp-abi.o");
        println!("cargo:rerun-if-changed={}", cxx_abi_source.display());
        let status = Command::new("bash")
            .arg(&target_cc_wrapper)
            .args([
                "-x",
                "c++",
                "-fno-asynchronous-unwind-tables",
                "-fno-exceptions",
                "-fno-rtti",
                "-c",
            ])
            .arg(&cxx_abi_source)
            .arg("-o")
            .arg(&cxx_abi_object)
            .status()
            .unwrap_or_else(|error| {
                panic!("failed to compile Whisper libc++ ABI adapter: {error}")
            });
        if !status.success() {
            panic!("Nagi Whisper libc++ ABI adapter compilation failed with {status}");
        }

        // The statically linked Whisper CPU path uses these target-owned C/POSIX
        // and math implementations. It never asks the host to open the model.
        for symbol in [
            "stdout",
            "stderr",
            "fflush",
            "snprintf",
            "vsnprintf",
            "fprintf",
            "fputs",
            "printf",
            "ldexpf",
            "log2f",
            "expf",
            "logf",
            "tanhf",
            "expm1f",
            "lroundf",
            "log2",
            "powf",
            "cosf",
            "sinf",
            "atoi",
            "dlclose",
            "tolower",
            "strcmp",
            "erff",
        ] {
            println!("cargo:rustc-link-arg-bin=nagi-init=--undefined={symbol}");
        }
        println!("cargo:rustc-link-arg-bin=nagi-init=--error-limit=0");
        println!("cargo:rustc-link-arg-bin=nagi-init=--gc-sections");
        println!(
            "cargo:rustc-link-arg-bin=nagi-init={}",
            adapter_object.display()
        );
        println!(
            "cargo:rustc-link-arg-bin=nagi-init={}",
            cxx_abi_object.display()
        );
        // Whisper's target libc++ headers declare __sort as an extern-template
        // ABI entrypoint. Reuse Nagi's allocation-free sort implementation
        // instead of linking a host or libc++ archive. The M17 feature already
        // compiles and links this object for its Servo/MozJS ABI boundary.
        if env::var_os("CARGO_FEATURE_M17_SERVO").is_none() {
            let sort_source = repository_root.join("tools/mesa/nagi-libcpp-sort.cpp");
            let sort_object = out_dir.join("nagi-libcpp-sort.o");
            println!("cargo:rerun-if-changed={}", sort_source.display());
            let status = Command::new("bash")
                .arg(&target_cc_wrapper)
                .args([
                    "-x",
                    "c++",
                    "-fno-asynchronous-unwind-tables",
                    "-fno-exceptions",
                    "-fno-rtti",
                    "-c",
                ])
                .arg(&sort_source)
                .arg("-o")
                .arg(&sort_object)
                .status()
                .unwrap_or_else(|error| {
                    panic!("failed to compile Whisper libc++ sort ABI: {error}")
                });
            if !status.success() {
                panic!("Nagi Whisper libc++ sort ABI compilation failed with {status}");
            }
            println!(
                "cargo:rustc-link-arg-bin=nagi-init={}",
                sort_object.display()
            );
        }
        println!("cargo:rustc-link-arg-bin=nagi-init=-Bstatic");
        println!("cargo:rustc-link-arg-bin=nagi-init=--start-group");
        for archive in &archives {
            println!("cargo:rustc-link-arg-bin=nagi-init={}", archive.display());
        }
        println!("cargo:rustc-link-arg-bin=nagi-init=--end-group");
        println!("cargo:rustc-link-arg-bin=nagi-init=-Bdynamic");
    }

    if env::var_os("CARGO_FEATURE_M17_SERVO").is_some() {
        // Some pinned Servo/MozJS objects use libc++ extern-template entrypoints
        // which are normally supplied by libc++.a. Nagi deliberately has no host
        // C++ runtime, so instantiate the exact required algorithms/string method
        // from the target's libc++ headers and provide sleep_for through the real
        // guest POSIX clock bridge. These objects are M17-only: M0 image builds
        // do not generate the relibc headers they require and do not link Servo.
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
}

/// Generate `system_fonts.rs`: the pinned fonts from `./nagi fetch`
/// (`NAGI_FONT_DIR/manifest.tsv`) for Servo builds, or an empty list.
/// `./nagi fetch` verified each file's SHA-256; the size is checked again here.
fn stage_system_fonts(out_dir: &std::path::Path) {
    let mut generated = String::from(
        "/// Generated by user/nagi-init/build.rs from out/cache/fonts/manifest.tsv.\n\
         pub static SYSTEM_FILES: &[(&[u8], &[u8])] = &[\n",
    );
    if env::var_os("CARGO_FEATURE_M17_SERVO").is_some() {
        let font_dir = env::var_os("NAGI_FONT_DIR")
            .map(PathBuf::from)
            .expect("m17-servo requires NAGI_FONT_DIR from `./nagi fetch`");
        let manifest_path = font_dir.join("manifest.tsv");
        println!("cargo:rerun-if-changed={}", manifest_path.display());
        let manifest = fs::read_to_string(&manifest_path)
            .expect("read system font manifest; run `./nagi fetch`");
        let staged = out_dir.join("system-fonts");
        fs::create_dir_all(&staged).expect("create staged font directory");
        for line in manifest.lines().filter(|line| !line.is_empty()) {
            let fields: Vec<&str> = line.split('\t').collect();
            let [guest_path, file_name, size, _sha256] = fields[..] else {
                panic!("malformed system font manifest line: {line}");
            };
            assert!(
                guest_path.starts_with("/system/fonts/")
                    && !file_name.contains('/')
                    && !file_name.contains('"'),
                "unexpected system font manifest entry: {line}"
            );
            let source = font_dir.join(file_name);
            println!("cargo:rerun-if-changed={}", source.display());
            let bytes = fs::read(&source).expect("read pinned system font");
            let expected: usize = size.parse().expect("font size");
            assert_eq!(bytes.len(), expected, "{file_name} does not match its pin");
            fs::write(staged.join(file_name), bytes).expect("stage system font");
            generated.push_str(&format!(
                "    (b\"{guest_path}\", include_bytes!(concat!(env!(\"OUT_DIR\"), \"/system-fonts/{file_name}\"))),\n"
            ));
        }
    }
    generated.push_str("];\n");
    fs::write(out_dir.join("system_fonts.rs"), generated).expect("write system font table");
}
