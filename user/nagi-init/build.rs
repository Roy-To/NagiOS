use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=NAGI_M16_PACKAGE");
    println!("cargo:rerun-if-env-changed=NAGI_TARGET_CLANG");
    println!("cargo:rerun-if-env-changed=NAGI_MESA_BUILD");
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
        ] {
            println!(
                "cargo:rustc-link-arg-bin=nagi-init=--undefined={symbol}"
            );
        }
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
}
