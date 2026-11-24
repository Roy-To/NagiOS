use super::*;

const INITIAL: Preferences = Preferences {
    locale: Locale::EnUs,
    theme: ThemeMode::Light,
};

fn press(code: u16) -> InputEvent {
    InputEvent {
        event_type: INPUT_EVENT_KEY,
        code,
        value: 1,
    }
}

#[test]
fn keyboard_cycles_and_only_requests_a_change() {
    let mut panel = SettingsPanel::new(INITIAL);
    for _ in ROWS {
        assert_eq!(panel.on_key(press(INPUT_KEY_TAB)), SettingsAction::Redraw);
    }
    assert_eq!(
        panel.on_key(press(INPUT_KEY_ENTER)),
        SettingsAction::Save(INITIAL)
    );
    panel.on_key(press(INPUT_KEY_DOWN));
    let japanese = Preferences {
        locale: Locale::JaJp,
        ..INITIAL
    };
    assert_eq!(
        panel.on_key(press(INPUT_KEY_SPACE)),
        SettingsAction::Save(japanese)
    );
    panel.on_key(press(INPUT_KEY_DOWN));
    assert_eq!(
        panel.on_key(press(INPUT_KEY_ENTER)),
        SettingsAction::Save(INITIAL)
    );
    panel.show_preferences(japanese);
    panel.on_key(press(INPUT_KEY_DOWN));
    assert_eq!(
        panel.on_key(press(INPUT_KEY_ENTER)),
        SettingsAction::Save(Preferences {
            theme: ThemeMode::Dark,
            ..japanese
        })
    );
    assert_eq!(panel.on_key(press(INPUT_KEY_ESCAPE)), SettingsAction::Close);
}

#[test]
fn reload_required_blocks_preference_writes() {
    let mut panel = SettingsPanel::new(INITIAL);
    panel.show_save_error(SettingsStatus::ReloadRequired);
    assert_eq!(
        panel.on_key(press(INPUT_KEY_ENTER)),
        SettingsAction::Ignored
    );
    panel.on_key(press(INPUT_KEY_UP));
    panel.on_key(press(INPUT_KEY_UP));
    assert_eq!(panel.on_key(press(INPUT_KEY_ENTER)), SettingsAction::Reload);
    panel.show_preferences(INITIAL);
    panel.on_key(press(INPUT_KEY_DOWN));
    panel.on_key(press(INPUT_KEY_DOWN));
    assert_eq!(
        panel.on_key(press(INPUT_KEY_ENTER)),
        SettingsAction::Save(INITIAL)
    );
}
