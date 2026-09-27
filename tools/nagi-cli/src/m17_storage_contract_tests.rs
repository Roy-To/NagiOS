#[test]
fn m17_mounts_the_guest_vfs_and_prepares_tmp_before_servo() {
    let init = include_str!("../../../user/nagi-init/src/main.rs");
    let start = init
        .find("pub extern \"C\" fn _start")
        .expect("nagi-init entry point");
    let init_entry = &init[start..];
    let m17_start = init_entry
        .find("#[cfg(feature = \"m17-servo\")]")
        .expect("M17 init branch");
    let m17_branch = &init_entry[m17_start..];
    let m17_end = m17_branch
        .find("#[cfg(not(any(")
        .expect("generic init branch after M17");
    let m17_branch = &m17_branch[..m17_end];
    let storage = m17_branch
        .find("run_m7_storage_acceptance(block_capability)")
        .expect("M17 first boot storage acceptance");
    let release_m7_volume = m17_branch
        .find("drop(volume)")
        .expect("release the M7 acceptance mount before POSIX mounts it");
    let initialize_posix = m17_branch
        .find("nagi_posix::nagi_posix_initialize_filesystem(block_capability)")
        .expect("mount the real guest VFS for Servo");
    let ensure_tmp = m17_branch
        .find("nagi_posix::nagi_posix_ensure_directory(c\"/tmp\".as_ptr())")
        .expect("create or verify the real guest temporary directory");
    let pixel = m17_branch
        .find("run_first_web_pixel(display_capability)")
        .expect("M17 first web pixel path");

    assert!(
        storage < release_m7_volume
            && release_m7_volume < initialize_posix
            && initialize_posix < ensure_tmp
            && ensure_tmp < pixel,
        "Servo must mount the persistent guest VFS and prepare /tmp before construction"
    );
    assert!(
        m17_branch[storage..pixel].contains("libnagi::exit(exit_code)"),
        "the first persistent-write boot must stop before the pixel boot"
    );
}
