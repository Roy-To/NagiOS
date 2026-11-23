use super::*;
use crate::{EntryAvailability, FileEntry, FileName, FilesViewState, Location, ResourceId};

#[derive(Clone, Debug, Eq, PartialEq)]
enum DrawCommand {
    Fill(FilesCanvasRect, ColorRole),
    Frame(FilesCanvasRect, ColorRole),
    Text(FilesCanvasRect, String, TypeRole, ColorRole),
}

#[derive(Default)]
struct RecordingCanvas {
    commands: Vec<DrawCommand>,
}

impl FilesCanvas for RecordingCanvas {
    fn fill_rect(&mut self, bounds: FilesCanvasRect, color: ColorRole) {
        self.commands.push(DrawCommand::Fill(bounds, color));
    }

    fn frame_rect(&mut self, bounds: FilesCanvasRect, color: ColorRole) {
        self.commands.push(DrawCommand::Frame(bounds, color));
    }

    fn text(
        &mut self,
        bounds: FilesCanvasRect,
        value: &str,
        typography: TypeRole,
        color: ColorRole,
    ) {
        self.commands.push(DrawCommand::Text(
            bounds,
            value.to_owned(),
            typography,
            color,
        ));
    }
}

#[test]
fn draws_selected_layout_and_inspector_through_canvas() {
    let id = ResourceId(41);
    let entry = FileEntry {
        id,
        name: FileName::parse("quarterly-plan.md").unwrap(),
        location: Location::root(),
        kind: EntryKind::File,
        size_bytes: Some(42),
        created_at: None,
        modified_at: None,
        tags: Vec::new(),
        availability: EntryAvailability::Available,
    };
    let mut state = FilesViewState::new(Location::root());
    state.entries.push(entry);
    state.select(id, false).unwrap();
    let layout = state.layout();

    let mut canvas = RecordingCanvas::default();
    draw_three_pane(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 320, 200),
        &layout,
        localization::text(Locale::EnUs, "files.empty").unwrap(),
        Locale::EnUs,
    );

    assert!(canvas.commands.iter().any(|command| matches!(
        command,
        DrawCommand::Text(_, value, _, _) if value == "Resources"
    )));
    assert!(canvas.commands.iter().any(|command| matches!(
        command,
        DrawCommand::Text(_, value, _, _) if value.contains("quarterly-plan.md")
    )));
    assert!(canvas.commands.iter().any(|command| matches!(
        command,
        DrawCommand::Text(_, value, _, _) if value == "Metadata"
    )));
    assert!(canvas.commands.iter().any(|command| matches!(
        command,
        DrawCommand::Text(_, value, _, _) if value == "Kind: File"
    )));
    assert!(canvas.commands.iter().any(|command| matches!(
        command,
        DrawCommand::Text(_, value, _, _) if value == "Size"
    )));
    assert!(canvas.commands.iter().any(|command| matches!(
        command,
        DrawCommand::Text(_, value, _, _) if value == "42 B"
    )));
    assert!(canvas.commands.iter().any(|command| matches!(
        command,
        DrawCommand::Text(_, value, _, _) if value == "Home"
    )));
    assert!(canvas
        .commands
        .iter()
        .any(|command| matches!(command, DrawCommand::Fill(_, ColorRole::Selection))));
    assert!(canvas
        .commands
        .iter()
        .any(|command| matches!(command, DrawCommand::Frame(_, ColorRole::Focus))));
}
