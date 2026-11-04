// Links the host whisper.cpp build and the Nagi provider adapter for the
// `real-engine` feature. Without the feature nothing is compiled or linked.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=NAGI_WHISPER_HOST_BUILD");
    println!("cargo:rerun-if-env-changed=NAGI_WHISPER_HOST_SOURCE");
    if env::var_os("CARGO_FEATURE_REAL_ENGINE").is_none() {
        return;
    }
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repository_root = manifest_dir.join("../..");
    let build = PathBuf::from(
        env::var_os("NAGI_WHISPER_HOST_BUILD")
            .expect("NAGI_WHISPER_HOST_BUILD must point to the host whisper.cpp build"),
    );
    let source = PathBuf::from(
        env::var_os("NAGI_WHISPER_HOST_SOURCE")
            .expect("NAGI_WHISPER_HOST_SOURCE must point to the patched whisper.cpp source"),
    );
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let adapter = repository_root.join("tools/whisper/nagi-provider-adapter.cpp");
    println!("cargo:rerun-if-changed={}", adapter.display());
    let object = out_dir.join("nagi-provider-adapter.o");
    let compiler = env::var_os("CXX").unwrap_or_else(|| "c++".into());
    let status = Command::new(compiler)
        .args(["-std=c++17", "-O2", "-fPIC", "-c"])
        .arg(&adapter)
        .arg("-I")
        .arg(source.join("include"))
        .arg("-I")
        .arg(source.join("ggml/include"))
        .arg("-o")
        .arg(&object)
        .status()
        .expect("start host C++ compiler");
    assert!(status.success(), "compile nagi-provider-adapter.cpp");
    let archive = out_dir.join("libnagi_whisper_adapter.a");
    let _ = std::fs::remove_file(&archive);
    let status = Command::new("ar")
        .arg("crs")
        .arg(&archive)
        .arg(&object)
        .status()
        .expect("start ar");
    assert!(status.success(), "archive adapter");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!(
        "cargo:rustc-link-search=native={}",
        build.join("src").display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        build.join("ggml/src").display()
    );
    for library in [
        "nagi_whisper_adapter",
        "whisper",
        "ggml",
        "ggml-cpu",
        "ggml-base",
    ] {
        println!("cargo:rustc-link-lib=static={library}");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else {
        println!("cargo:rustc-link-lib=dylib=stdc++");
        println!("cargo:rustc-link-lib=dylib=pthread");
        println!("cargo:rustc-link-lib=dylib=m");
    }
}
