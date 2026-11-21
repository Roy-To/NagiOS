use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("nagi") {
        return;
    }

    // The legacy Nagi kernel loads app ELFs in the same user image range as
    // init. Keep the layout on this package's binaries, not in global flags.
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let linker_script = manifest_dir.join("../nagi-init/linker.ld");
    assert!(
        linker_script.is_file(),
        "Nagi user linker script is missing"
    );
    println!("cargo:rerun-if-changed={}", linker_script.display());
    println!("cargo:rustc-link-arg-bins=-T");
    println!("cargo:rustc-link-arg-bins={}", linker_script.display());
}
