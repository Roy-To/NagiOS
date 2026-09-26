use nagi_ui::{
    palette, ButtonEvent, ButtonModel, ButtonOutcome, ColorRole, CommandId, CommandPalette,
    CommandPaletteAction, CommandResult, ComponentKind, KeyCode, MessageKey, PaletteStatus,
    ResolvedText, ThemeMode, COMPONENT_KINDS,
};

fn main() {
    let theme = palette(ThemeMode::Light);
    let mut button = ButtonModel::new(true);
    button.handle(ButtonEvent::Focus);
    let activated = button.handle(ButtonEvent::KeyDown(KeyCode::Enter));
    let query = ResolvedText::new(MessageKey::new("command.search"), "設定を開く");
    let results = [CommandResult {
        id: CommandId(1),
        label: MessageKey::new("command.open_settings"),
        description: Some(MessageKey::new("command.open_settings.description")),
        section: Some(MessageKey::new("command.system")),
        shortcut: None,
    }];
    let mut palette_model = CommandPalette::new(query);
    palette_model.update(query, PaletteStatus::Ready, &results);
    let palette_action = palette_model.handle_key(KeyCode::Enter);

    println!("Nagi UI component contract gallery");
    println!("component kinds: {}", COMPONENT_KINDS.len());
    println!(
        "button Enter: {:?}",
        activated == Some(ButtonOutcome::Activated)
    );
    println!("palette action: {:?}", palette_action);
    println!("Japanese query: {}", palette_model.query().value);
    println!(
        "semantic canvas/accent pixels: #{:08x} / #{:08x}",
        theme.color(ColorRole::Canvas).to_pixel(),
        theme.color(ColorRole::Accent).to_pixel()
    );

    assert_eq!(activated, Some(ButtonOutcome::Activated));
    assert_eq!(
        palette_action,
        Some(CommandPaletteAction::Execute(CommandId(1)))
    );
    assert!(COMPONENT_KINDS.contains(&ComponentKind::CommandPalette));
}
