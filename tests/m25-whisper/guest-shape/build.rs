// Stages minimal placeholders for the files nagi-init's build.rs stages, so the
// guest module type-checks. They are never executed or compared.
fn main() {
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out_dir.join("m25-whisper-input.pcm"), [0_u8; 2]).unwrap();
    std::fs::write(out_dir.join("m25-whisper-expected.txt"), "-").unwrap();
}
