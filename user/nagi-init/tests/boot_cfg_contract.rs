const MAIN_RS: &str = include_str!("../src/main.rs");

const DESKTOP_ONLY_CFG: &str = concat!(
    "#[cfg(all(",
    "feature=\"m10-desktop\"",
    ",not(any(",
    "feature=\"m11-security\"",
    ",feature=\"m12-network\"",
    ",feature=\"m13-posix\"",
    "))",
    "))]",
);

const NAGI_DESKTOP_ONLY_CFG: &str = concat!(
    "#[cfg(all(",
    "target_os=\"nagi\"",
    ",feature=\"m10-desktop\"",
    ",not(any(",
    "feature=\"m11-security\"",
    ",feature=\"m12-network\"",
    ",feature=\"m13-posix\"",
    "))",
    "))]",
);

fn compact(source: &str) -> String {
    source.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn assert_directly_guarded(source: &str, label: &str, statement: &str) {
    let contract = format!("{DESKTOP_ONLY_CFG}{statement}");
    assert!(
        source.contains(&contract),
        "{label} must use the M10 desktop-only cfg guard"
    );
}

fn matching_brace(source: &str, opening: usize) -> usize {
    let mut depth = 0_usize;
    for (offset, byte) in source.as_bytes()[opening..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return opening + offset;
                }
            }
            _ => {}
        }
    }
    panic!("guarded block is missing its closing brace");
}

#[test]
fn boot_renderer_is_guarded_by_m10_desktop_only_cfg() {
    let source = compact(MAIN_RS);

    assert!(
        source.contains(&format!("{NAGI_DESKTOP_ONLY_CFG}modboot;")),
        "boot module must use the Nagi M10 desktop-only cfg guard"
    );

    assert_directly_guarded(
        &source,
        "boot screen construction",
        "letmutboot_screen=matchboot::BootScreen::new(display_capability)",
    );
    for (label, stage) in [
        ("platform stage", "Platform"),
        ("core-services stage", "CoreServices"),
        ("storage stage", "Storage"),
    ] {
        assert_directly_guarded(
            &source,
            label,
            &format!("ifboot_screen.present_stage(libnagi::boot::BootStage::{stage})"),
        );
    }

    let desktop_run = source
        .find("desktop::run(display_capability,input_capability);")
        .expect("desktop handoff must remain present");
    let guard_start = source[..desktop_run]
        .rfind(DESKTOP_ONLY_CFG)
        .expect("desktop handoff must have an M10 desktop-only cfg guard");
    let block_start = guard_start + DESKTOP_ONLY_CFG.len();
    assert_eq!(
        source.as_bytes().get(block_start),
        Some(&b'{'),
        "desktop-only cfg must guard a block"
    );
    let block_end = matching_brace(&source, block_start);
    let desktop_block = &source[block_start..=block_end];

    for (label, call) in [
        ("graphics stage", "BootStage::Graphics"),
        ("session stage", "BootStage::Session"),
        ("desktop transition", "boot_screen.finish_to_desktop()"),
        (
            "desktop handoff",
            "desktop::run(display_capability,input_capability);",
        ),
    ] {
        assert!(
            desktop_block.contains(call),
            "{label} must remain inside the M10 desktop-only block"
        );
    }
}

#[test]
fn linker_script_captures_large_code_model_section_families() {
    let linker = include_str!("../linker.ld");

    assert!(
        linker.contains(".ltext"),
        "text must capture large-model sections"
    );
    assert!(
        linker.contains(".lrodata"),
        "rodata must capture large-model sections"
    );
}
