//! Development Files keyboard mapping. Key codes are Linux evdev values.

use libnagi::{InputEvent, INPUT_EVENT_KEY};
use nagi_files::{EntryKind, FileName, FilesViewState, Location, ResourceId, TrashEntry};

const ESC: u16 = 1;
const BACKSPACE: u16 = 14;
const ENTER: u16 = 28;
const KEYPAD_ENTER: u16 = 96;
const UP: u16 = 103;
const DOWN: u16 = 108;
const DELETE: u16 = 111;
const N: u16 = 49;
const R: u16 = 19;
const F2: u16 = 60;
const T: u16 = 20;
const NAME_LIMIT: usize = 255;

#[derive(Debug, Eq, PartialEq)]
pub(super) enum FilesKeyAction {
    Ignored,
    Redraw,
    Select(ResourceId),
    OpenFolder(Location),
    OpenFile,
    RejectSymlink,
    Parent,
    CreateFolder(FileName),
    Rename(FileName),
    Trash,
    ShowTrash,
    Restore(ResourceId),
    RestoreAs(ResourceId, Location, FileName),
    Close,
}

#[derive(Debug)]
enum Purpose {
    Create,
    Rename,
    RestoreAs(ResourceId, Location),
}

#[derive(Debug)]
struct NameEdit {
    purpose: Purpose,
    text: String,
    invalid: bool,
}

#[derive(Default, Debug)]
pub(super) struct FilesKeys {
    in_trash: bool,
    trash_row: usize,
    left_shift: bool,
    right_shift: bool,
    name: Option<NameEdit>,
}

impl FilesKeys {
    pub(super) fn in_trash(&self) -> bool {
        self.in_trash
    }

    pub(super) fn name_input(&self) -> Option<(&str, bool)> {
        self.name
            .as_ref()
            .map(|edit| (edit.text.as_str(), edit.invalid))
    }

    pub(super) fn begin_restore_as(&mut self, id: ResourceId, parent: Location, name: &FileName) {
        self.edit_name(Purpose::RestoreAs(id, parent), name.as_str());
    }

    pub(super) fn on_event(
        &mut self,
        event: InputEvent,
        state: &FilesViewState,
        trash: &[TrashEntry],
    ) -> FilesKeyAction {
        use FilesKeyAction::*;
        if event.event_type != INPUT_EVENT_KEY {
            return Ignored;
        }
        match event.code {
            42 => {
                self.left_shift = event.value != 0;
                return Ignored;
            }
            54 => {
                self.right_shift = event.value != 0;
                return Ignored;
            }
            _ => {}
        }
        let shift = self.left_shift || self.right_shift;
        let repeatable = matches!(event.code, UP | DOWN)
            || (self.name.is_some()
                && (event.code == BACKSPACE || name_char(event.code, shift).is_some()));
        if event.value != 1 && !(event.value == 2 && repeatable) {
            return Ignored;
        }
        if self.name.is_some() {
            return self.edit_key(event.code, shift);
        }
        if self.in_trash {
            return match event.code {
                ESC | BACKSPACE | T => {
                    self.in_trash = false;
                    Redraw
                }
                UP | DOWN if !trash.is_empty() => {
                    self.trash_row = move_row(
                        Some(self.trash_row.min(trash.len() - 1)),
                        trash.len(),
                        event.code == DOWN,
                    );
                    Redraw
                }
                ENTER | KEYPAD_ENTER => self.trash_selection(trash).map_or(Ignored, Restore),
                _ => Ignored,
            };
        }

        let focused = state
            .focused
            .and_then(|id| state.entries.iter().position(|entry| entry.id == id));
        let current = focused.and_then(|row| state.entries.get(row));
        match event.code {
            ESC => Close,
            BACKSPACE if !state.location.is_root() => Parent,
            UP | DOWN if !state.entries.is_empty() => {
                let index = move_row(focused, state.entries.len(), event.code == DOWN);
                Select(state.entries[index].id)
            }
            ENTER | KEYPAD_ENTER => match current {
                Some(entry) if entry.kind == EntryKind::Folder => {
                    OpenFolder(entry.child_location())
                }
                Some(entry) if entry.kind == EntryKind::Symlink => RejectSymlink,
                Some(_) => OpenFile,
                None => Ignored,
            },
            N => {
                self.edit_name(Purpose::Create, "");
                Redraw
            }
            R | F2 => match current {
                Some(entry) => {
                    self.edit_name(Purpose::Rename, entry.name.as_str());
                    Redraw
                }
                None => Ignored,
            },
            DELETE if current.is_some() => Trash,
            T => {
                self.in_trash = true;
                self.trash_row = 0;
                ShowTrash
            }
            _ => Ignored,
        }
    }

    pub(super) fn trash_selection(&self, entries: &[TrashEntry]) -> Option<ResourceId> {
        entries
            .get(self.trash_row.min(entries.len().saturating_sub(1)))
            .map(|entry| entry.id)
    }

    fn edit_name(&mut self, purpose: Purpose, text: &str) {
        self.name = Some(NameEdit {
            purpose,
            text: text.to_owned(),
            invalid: false,
        });
    }

    fn edit_key(&mut self, code: u16, shift: bool) -> FilesKeyAction {
        use FilesKeyAction::*;
        let Some(edit) = self.name.as_mut() else {
            return Ignored;
        };
        match code {
            ESC => {
                self.name = None;
                Redraw
            }
            ENTER | KEYPAD_ENTER => {
                let Ok(name) = FileName::parse(&edit.text) else {
                    edit.invalid = true;
                    return Redraw;
                };
                match self.name.take().map(|edit| edit.purpose) {
                    Some(Purpose::Create) => CreateFolder(name),
                    Some(Purpose::Rename) => Rename(name),
                    Some(Purpose::RestoreAs(id, parent)) => RestoreAs(id, parent, name),
                    None => Ignored,
                }
            }
            BACKSPACE => {
                edit.text.pop();
                edit.invalid = false;
                Redraw
            }
            _ => match name_char(code, shift) {
                Some(character) if edit.text.len() < NAME_LIMIT => {
                    edit.text.push(character);
                    edit.invalid = false;
                    Redraw
                }
                _ => Ignored,
            },
        }
    }
}

fn move_row(current: Option<usize>, len: usize, down: bool) -> usize {
    match current {
        None => 0,
        Some(row) if down => (row + 1).min(len - 1),
        Some(row) => row.saturating_sub(1),
    }
}

fn name_char(code: u16, shift: bool) -> Option<char> {
    let (keys, index) = match code {
        2..=11 => (
            if shift { b"!@#$%^&*()" } else { b"1234567890" }.as_slice(),
            code - 2,
        ),
        16..=25 => (b"qwertyuiop".as_slice(), code - 16),
        30..=38 => (b"asdfghjkl".as_slice(), code - 30),
        44..=50 => (b"zxcvbnm".as_slice(), code - 44),
        12 => return Some(if shift { '_' } else { '-' }),
        43 => return Some('\\'),
        52 => return Some('.'),
        53 => return Some('/'),
        57 => return Some(' '),
        _ => return None,
    };
    let character = char::from(keys[usize::from(index)]);
    Some(if shift {
        character.to_ascii_uppercase()
    } else {
        character
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nagi_files::{EntryAvailability, FileEntry};

    fn event(code: u16, value: i32) -> InputEvent {
        InputEvent {
            event_type: INPUT_EVENT_KEY,
            code,
            value,
        }
    }

    fn state() -> FilesViewState {
        let mut state = FilesViewState::new(Location::root());
        for (id, name, kind) in [
            (1, "folder", EntryKind::Folder),
            (2, "file", EntryKind::File),
            (3, "link", EntryKind::Symlink),
        ] {
            state.entries.push(FileEntry {
                id: ResourceId(id),
                name: FileName::parse(name).unwrap(),
                location: Location::root(),
                kind,
                size_bytes: None,
                created_at: None,
                modified_at: None,
                tags: Vec::new(),
                availability: EntryAvailability::Available,
            });
        }
        state.focused = Some(ResourceId(1));
        state
    }

    #[test]
    fn browse_select_open_and_reject_symlinks() {
        let mut keys = FilesKeys::default();
        let mut state = state();
        assert_eq!(
            keys.on_event(event(103, 1), &state, &[]),
            FilesKeyAction::Select(ResourceId(1))
        );
        assert_eq!(
            keys.on_event(event(108, 1), &state, &[]),
            FilesKeyAction::Select(ResourceId(2))
        );
        state.focused = Some(ResourceId(2));
        assert_eq!(
            keys.on_event(event(28, 1), &state, &[]),
            FilesKeyAction::OpenFile
        );
        state.focused = Some(ResourceId(3));
        assert_eq!(
            keys.on_event(event(96, 1), &state, &[]),
            FilesKeyAction::RejectSymlink
        );
        state.focused = Some(ResourceId(1));
        assert_eq!(
            keys.on_event(event(28, 1), &state, &[]),
            FilesKeyAction::OpenFolder(Location::parse("folder").unwrap())
        );
    }

    #[test]
    fn name_entry_uses_filename_validation_and_requires_key_down() {
        let mut keys = FilesKeys::default();
        let state = FilesViewState::new(Location::root());
        assert_eq!(
            keys.on_event(event(N, 0), &state, &[]),
            FilesKeyAction::Ignored
        );
        assert_eq!(
            keys.on_event(event(N, 1), &state, &[]),
            FilesKeyAction::Redraw
        );
        keys.on_event(event(30, 1), &state, &[]); // a
        keys.on_event(event(53, 1), &state, &[]); // slash
        assert_eq!(
            keys.on_event(event(28, 1), &state, &[]),
            FilesKeyAction::Redraw
        );
        assert_eq!(keys.name_input().map(|(_, invalid)| invalid), Some(true));
        assert_eq!(
            keys.on_event(event(1, 1), &state, &[]),
            FilesKeyAction::Redraw
        );
        assert_eq!(keys.name_input(), None);
    }
}
