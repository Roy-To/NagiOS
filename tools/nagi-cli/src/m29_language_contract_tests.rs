const DESKTOP: &str = include_str!("../../../user/nagi-init/src/desktop.rs");
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

    let persist = desktop
        .find("if!persist_locale(volume,nagi_localization::Locale::JaJp)")
        .expect("Japanese selection must persist before display state changes");
    let select = desktop[persist..]
        .find("self.locale=nagi_localization::Locale::JaJp")
        .map(|offset| persist + offset)
        .expect("Japanese locale is applied to the Desktop");
    assert!(persist < select);

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
