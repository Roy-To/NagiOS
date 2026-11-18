use std::{env, fs, path::PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux")
        || env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86_64")
    {
        return;
    }
    println!("cargo:rerun-if-env-changed=NAGI_LINUX_BOOT_CATALOG_PUBLIC_KEY");
    let key = env::var_os("NAGI_LINUX_BOOT_CATALOG_PUBLIC_KEY")
        .map(|path| {
            let path = PathBuf::from(path);
            println!("cargo:rerun-if-changed={}", path.display());
            let key = fs::read(path).expect("read the pinned boot catalog public key");
            assert_eq!(key.len(), 32, "expected 32 raw boot catalog key bytes");
            key
        })
        .unwrap_or_default();
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(out.join("recovery-boot-catalog-key"), key).expect("stage pinned boot catalog key");
}
