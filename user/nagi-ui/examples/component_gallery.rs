use nagi_ui::{
    palette, scaled_typography, ApplicationShellSpec, ButtonEvent, ButtonModel, ButtonOutcome,
    ColorRole, CommandId, CommandPalette, CommandPaletteAction, CommandResult, ComponentKind,
    FeedbackState, KeyCode, MessageKey, PaletteStatus, ResolvedText, SelectAction, SelectModel,
    ShellRegionSpec, TextScale, ThemeMode, TypeRole, COMPONENT_KINDS,
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
    let mut select = SelectModel::new([true, false, true], Some(0));
    select.handle_key(KeyCode::Enter);
    select.handle_key(KeyCode::ArrowDown);
    let selection = select.handle_key(KeyCode::Enter);
    let mut shell = ApplicationShellSpec::new(
        MessageKey::new("settings.title"),
        ShellRegionSpec::new(MessageKey::new("settings.content")),
    );
    shell.toolbar = Some(ShellRegionSpec::new(MessageKey::new("settings.actions")));
    shell.content_state = FeedbackState::Ready;
    let scaled_body = scaled_typography(TypeRole::Body, TextScale::Large);

    println!("Nagi UI component contract gallery");
    println!("component kinds: {}", COMPONENT_KINDS.len());
    println!(
        "button Enter: {:?}",
        activated == Some(ButtonOutcome::Activated)
    );
    println!("palette action: {:?}", palette_action);
    println!("select action: {:?}", selection);
    println!("shell regions: {}", shell.visible_region_count());
    println!("large body size: {}px", scaled_body.size_px);
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
    assert_eq!(selection, Some(SelectAction::Selected(2)));
    assert_eq!(shell.visible_region_count(), 3);
    assert!(COMPONENT_KINDS.contains(&ComponentKind::CommandPalette));
}
