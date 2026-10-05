//! Command Palette presentation state; command execution remains app-owned.

use crate::interaction::KeyCode;
use crate::text::{MessageKey, ResolvedText};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandId(pub u16);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShortcutHint {
    pub keys: &'static [KeyCode],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandResult {
    pub id: CommandId,
    pub label: MessageKey,
    pub description: Option<MessageKey>,
    pub section: Option<MessageKey>,
    pub shortcut: Option<ShortcutHint>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaletteStatus {
    Loading,
    Ready,
    Empty,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPaletteAction {
    Execute(CommandId),
    Dismiss,
}

/// Query and result text are already localized before reaching this view model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandPalette<'a> {
    query: ResolvedText<'a>,
    results: &'a [CommandResult],
    status: PaletteStatus,
    selected: Option<usize>,
}

impl<'a> CommandPalette<'a> {
    pub const fn new(query: ResolvedText<'a>) -> Self {
        Self {
            query,
            results: &[],
            status: PaletteStatus::Loading,
            selected: None,
        }
    }

    pub const fn query(self) -> ResolvedText<'a> {
        self.query
    }

    pub const fn status(self) -> PaletteStatus {
        self.status
    }

    pub const fn selected_index(self) -> Option<usize> {
        self.selected
    }

    pub fn selected_result(&self) -> Option<&CommandResult> {
        self.selected.and_then(|index| self.results.get(index))
    }

    pub fn update(
        &mut self,
        query: ResolvedText<'a>,
        status: PaletteStatus,
        results: &'a [CommandResult],
    ) {
        self.query = query;
        self.results = results;
        self.status = if status == PaletteStatus::Ready && results.is_empty() {
            PaletteStatus::Empty
        } else {
            status
        };
        self.selected = if self.status == PaletteStatus::Ready {
            Some(0)
        } else {
            None
        };
    }

    pub fn handle_key(&mut self, key: KeyCode) -> Option<CommandPaletteAction> {
        match key {
            KeyCode::Escape => Some(CommandPaletteAction::Dismiss),
            KeyCode::ArrowDown if self.status == PaletteStatus::Ready => {
                self.move_selection(true);
                None
            }
            KeyCode::ArrowUp if self.status == PaletteStatus::Ready => {
                self.move_selection(false);
                None
            }
            KeyCode::Enter if self.status == PaletteStatus::Ready => self
                .selected_result()
                .map(|result| CommandPaletteAction::Execute(result.id)),
            _ => None,
        }
    }

    fn move_selection(&mut self, forward: bool) {
        let count = self.results.len();
        if count == 0 {
            self.selected = None;
            return;
        }
        let current = self.selected.unwrap_or(0);
        self.selected = Some(if forward {
            (current + 1) % count
        } else if current == 0 {
            count - 1
        } else {
            current - 1
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMANDS: [CommandResult; 2] = [
        CommandResult {
            id: CommandId(20),
            label: MessageKey::new("command.open_settings"),
            description: Some(MessageKey::new("command.open_settings.description")),
            section: Some(MessageKey::new("command.system")),
            shortcut: Some(ShortcutHint {
                keys: &[KeyCode::Enter],
            }),
        },
        CommandResult {
            id: CommandId(21),
            label: MessageKey::new("command.open_files"),
            description: None,
            section: Some(MessageKey::new("command.apps")),
            shortcut: None,
        },
    ];

    #[test]
    fn palette_has_query_sections_shortcuts_selection_and_execution_affordance() {
        let query = ResolvedText::new(MessageKey::new("command.search"), "設定");
        let mut palette = CommandPalette::new(query);
        palette.update(query, PaletteStatus::Ready, &COMMANDS);
        assert_eq!(palette.status(), PaletteStatus::Ready);
        assert_eq!(palette.query().value, "設定");
        assert_eq!(
            palette.selected_result().unwrap().section,
            Some(MessageKey::new("command.system"))
        );
        assert_eq!(palette.handle_key(KeyCode::ArrowDown), None);
        assert_eq!(palette.selected_index(), Some(1));
        assert_eq!(
            palette.handle_key(KeyCode::Enter),
            Some(CommandPaletteAction::Execute(CommandId(21)))
        );
    }

    #[test]
    fn loading_empty_and_error_states_cannot_execute() {
        let query = ResolvedText::new(MessageKey::new("command.search"), "nagi");
        let mut palette = CommandPalette::new(query);
        for status in [PaletteStatus::Loading, PaletteStatus::Error] {
            palette.update(query, status, &COMMANDS);
            assert_eq!(palette.handle_key(KeyCode::Enter), None);
            assert_eq!(palette.selected_index(), None);
        }
        palette.update(query, PaletteStatus::Ready, &[]);
        assert_eq!(palette.status(), PaletteStatus::Empty);
        assert_eq!(palette.handle_key(KeyCode::Enter), None);
        assert_eq!(
            palette.handle_key(KeyCode::Escape),
            Some(CommandPaletteAction::Dismiss)
        );
    }

    #[test]
    fn palette_selection_wraps_both_directions() {
        let query = ResolvedText::new(MessageKey::new("command.search"), "");
        let mut palette = CommandPalette::new(query);
        palette.update(query, PaletteStatus::Ready, &COMMANDS);
        palette.handle_key(KeyCode::ArrowUp);
        assert_eq!(palette.selected_index(), Some(1));
        palette.handle_key(KeyCode::ArrowDown);
        assert_eq!(palette.selected_index(), Some(0));
    }
}
