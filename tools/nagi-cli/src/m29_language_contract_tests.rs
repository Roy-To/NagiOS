const DESKTOP: &str = include_str!("../../../user/nagi-init/src/desktop.rs");
const FONT: &str = include_str!("../../../user/nagi-init/src/font.rs");
const GENERATED_FONT: &str = include_str!("../../../user/nagi-init/src/font_japanese_generated.rs");
const INIT: &str = include_str!("../../../user/nagi-init/src/main.rs");

fn compact(source: &str) -> String {
    source.chars().filter(|ch| !ch.is_whitespace()).collect()
}

#[test]
fn system_language_is_committed_to_user_data_before_desktop_selection() {
    let desktop = compact(DESKTOP);
    assert!(desktop.contains("constSYSTEM_LANGUAGE_PATH:&[u8]=b\"system-language\";"));
    assert!(desktop.contains("volume.open_path(SYSTEM_LANGUAGE_PATH)"));
    assert!(desktop.contains("volume.create_path(SYSTEM_LANGUAGE_PATH)"));
    assert!(desktop.contains("volume.write(handle,locale.code().as_bytes())"));
    assert!(desktop.contains("volume.flush()"));
    assert!(desktop.contains("core::str::from_utf8(&contents[..length])"));
    assert!(desktop.contains(".and_then(nagi_localization::Locale::parse)"));

    let selection = desktop
        .find("fnselect_locale(")
        .map(|offset| &desktop[offset..])
        .expect("locale selection helper");
    let persist = selection
        .find("if!persist_locale(volume,locale)")
        .expect("selection must persist before display state changes");
    let select = selection[persist..]
        .find("self.locale=locale")
        .map(|offset| persist + offset)
        .expect("selected locale is applied to the Desktop");
    assert!(persist < select);
    assert!(desktop.contains(
        "Some(SettingsFocus::Japanese)ifself.settings_open=>{self.select_locale(volume,nagi_localization::Locale::JaJp,keyboard)}"
    ));

    let init = compact(INIT);
    let volume_owner = init
        .find("letSome(volume)=volumeelse")
        .expect("desktop boot must require the initialized User Data volume");
    let desktop_call = init
        .find("desktop::run(block_capability,display_capability,input_capability,volume,)")
        .expect("desktop boot must pass the same User Data volume to locale persistence");
    assert!(volume_owner < desktop_call);
}

#[test]
fn m29_language_setting_is_reachable_and_selectable_by_keyboard() {
    let desktop = compact(DESKTOP);
    assert!(desktop.contains("libnagi::INPUT_KEY_TAB"));
    assert!(desktop.contains("libnagi::INPUT_KEY_DOWN"));
    assert!(desktop.contains("libnagi::INPUT_KEY_ENTER"));
    assert!(desktop.contains("libnagi::INPUT_KEY_ESCAPE"));
    assert!(desktop.contains("SettingsFocus::Button"));
    assert!(desktop.contains("SettingsFocus::Japanese"));

    assert!(desktop.contains("NagiM29keyboardlocaleselectionPASSlocale=ja-JP"));
}

#[test]
fn selected_locale_has_a_localized_text_cue_in_its_option_row() {
    let desktop = compact(DESKTOP);
    let render_start = desktop
        .find("fnrender_locale_option(")
        .expect("locale option renderer");
    let render_end = desktop[render_start..]
        .find("fnfocus_next_desktop_control(")
        .map(|offset| render_start + offset)
        .expect("locale option renderer end");
    let render = &desktop[render_start..render_end];
    let selected_state = render
        .find("ifself.locale==locale{painter.text(")
        .map(|offset| &render[offset..])
        .expect("selected locale gets a text cue");
    assert!(selected_state.contains("desktop.settings.option.selected"));
    assert!(selected_state.contains("label_end+8"));
}

#[test]
fn japanese_selected_state_characters_have_desktop_font_glyphs() {
    let font = compact(FONT);
    assert!(font.contains("#[path=\"font_japanese_generated.rs\"]modgenerated;"));
    for character in ["選", "択", "中"] {
        let codepoint = format!("{:#06x},", character.chars().next().unwrap() as u32);
        assert!(
            GENERATED_FONT.contains(&codepoint),
            "desktop generated font is missing U+{:04X}",
            character.chars().next().unwrap() as u32
        );
    }
}

#[test]
fn m29_keyboard_focus_reaches_all_desktop_acceptance_panels() {
    let desktop = compact(DESKTOP);
    assert!(desktop.contains("Some(DesktopFocus::Application(index))"));
    assert!(desktop.contains("self.desktop_focus==Some(DesktopFocus::Application(index))"));
    assert!(desktop.contains("self.keyboard_app_focus.iter().all(|focused|*focused)"));
    assert!(desktop.contains("NagiM29desktopkeyboardfocusPASS"));
}
