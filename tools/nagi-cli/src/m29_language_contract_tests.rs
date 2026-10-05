const DESKTOP: &str = include_str!("../../../user/nagi-init/src/desktop.rs");
const FONT: &str = include_str!("../../../user/nagi-init/src/font.rs");
const INIT: &str = include_str!("../../../user/nagi-init/src/main.rs");
const COMMANDS: &str = include_str!("commands.rs");

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
    assert!(init.contains("desktop::run(display_capability,input_capability,volume);"));
}

#[test]
fn m29_acceptance_reboots_the_same_user_data_volume_and_checks_the_restored_locale() {
    let commands = compact(COMMANDS);
    let m29_start = commands.find("fnexecute_m29(").expect("M29 command");
    let acceptance_start = commands[m29_start..]
        .find("fnexecute_desktop_acceptance(")
        .map(|offset| m29_start + offset)
        .expect("shared Desktop acceptance runner");
    let m29 = &commands[m29_start..acceptance_start];
    assert!(
        m29.contains("restart_marker:Some(\"NagiM29settingspreferencerestoredPASSlocale=ja-JP\")")
    );
    assert!(m29.contains("restart_log_name:Some(\"m29-settings-persistent-restart.log\")"));
    assert!(m29.contains("unique_run_artifacts:true"));

    let restart = &commands[acceptance_start..];
    assert!(restart.contains("run_qemu(&restart_config)"));
    assert!(restart.contains("persistent_disk:&persistent_disk"));
    assert!(restart.contains("[\"NagiM10desktopREADY\",restart_marker]"));
    assert!(commands.contains("\"NagiM29settingslocalepersistedPASSlocale=ja-JP\""));
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

    let commands = compact(COMMANDS);
    let events_start = commands
        .find("constM29_SETTINGS_EVENTS:")
        .expect("M29 input sequence");
    let events_end = commands[events_start..]
        .find("];constM10_DESKTOP_REQUIRED_MARKERS")
        .map(|offset| events_start + offset)
        .expect("M29 input sequence end");
    let events = &commands[events_start..events_end];
    for event in [
        "\"data\":\"tab\"",
        "\"data\":\"ret\"",
        "\"data\":\"down\"",
        "\"data\":\"up\"",
        "\"data\":\"esc\"",
        "\"data\":\"spc\"",
    ] {
        assert!(events.contains(event), "M29 sequence must send {event}");
    }
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
    for character in ["選", "択", "中"] {
        assert!(
            FONT.contains(character),
            "desktop font is missing {character}"
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

    let commands = compact(COMMANDS);
    assert!(commands.contains("constM29_DESKTOP_FOCUS_EVENTS:[&str;11]"));
    assert!(commands.contains("letmutevents=M29_DESKTOP_FOCUS_EVENTS.to_vec();events.extend_from_slice(&M10_DESKTOP_EVENTS);"));
    assert!(commands.contains("\"NagiM29desktopkeyboardfocusPASS\""));
}
