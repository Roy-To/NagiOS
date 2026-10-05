//! Albert's trusted file picker for page file inputs.
//!
//! The picker is first-party chrome, never page content. It lists regular
//! files from the user's Documents folder, and only the file the user
//! explicitly chooses is handed to Servo; per the spec, that selection is the
//! user's consent for the page to read that file.

use nagi_localization::{text, Locale};

/// Folder the picker offers files from.
pub const PICKER_DIRECTORY: &str = "/Documents";
/// Upper bound on listed entries.
pub const MAX_PICKER_ENTRIES: usize = 12;
/// Upper bound on a displayed file name.
pub const MAX_PICKER_NAME_BYTES: usize = 48;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerEntry {
    pub name: String,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerKey {
    Up,
    Down,
    Enter,
    Escape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PickerOutcome {
    Pending,
    /// Absolute guest path of the chosen file.
    Chosen(String),
    Canceled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePickerState {
    entries: Vec<PickerEntry>,
    selected: usize,
}

/// Whether `name` satisfies the page's `accept` filter. Servo passes file
/// extensions (with or without a leading dot); an empty filter accepts all.
pub fn accepts(name: &str, filters: &[String]) -> bool {
    if filters.is_empty() {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    filters.iter().any(|filter| {
        let extension = filter.trim().trim_start_matches('.').to_ascii_lowercase();
        extension == "*" || (!extension.is_empty() && lower.ends_with(&format!(".{extension}")))
    })
}

fn displayable(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_PICKER_NAME_BYTES
        && !name.starts_with('.')
        && !name.contains('/')
        && !name.chars().any(char::is_control)
}

impl FilePickerState {
    /// Build the picker from directory entries, keeping displayable names
    /// that pass the filter, sorted by name and bounded.
    pub fn new(candidates: Vec<PickerEntry>, filters: &[String]) -> Self {
        let mut entries: Vec<PickerEntry> = candidates
            .into_iter()
            .filter(|entry| displayable(&entry.name) && accepts(&entry.name, filters))
            .collect();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        entries.truncate(MAX_PICKER_ENTRIES);
        Self {
            entries,
            selected: 0,
        }
    }

    pub fn entries(&self) -> &[PickerEntry] {
        &self.entries
    }

    pub fn selected(&self) -> Option<usize> {
        (!self.entries.is_empty()).then_some(self.selected)
    }

    pub fn handle_key(&mut self, key: PickerKey) -> PickerOutcome {
        match key {
            PickerKey::Up => {
                self.selected = self.selected.saturating_sub(1);
                PickerOutcome::Pending
            }
            PickerKey::Down => {
                if self.selected + 1 < self.entries.len() {
                    self.selected += 1;
                }
                PickerOutcome::Pending
            }
            PickerKey::Enter => self.choose(self.selected),
            PickerKey::Escape => PickerOutcome::Canceled,
        }
    }

    /// Choose the entry at `index` (a click on its row).
    pub fn choose(&mut self, index: usize) -> PickerOutcome {
        match self.entries.get(index) {
            Some(entry) => {
                self.selected = index;
                PickerOutcome::Chosen(format!("{PICKER_DIRECTORY}/{}", entry.name))
            }
            None => PickerOutcome::Pending,
        }
    }
}

/// Picker geometry inside a `width` x `height` surface: the card rectangle
/// and the height of each row. Shared by rendering and hit testing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PickerLayout {
    pub card_x: u32,
    pub card_y: u32,
    pub card_width: u32,
    pub card_height: u32,
    pub rows_y: u32,
}

pub const PICKER_ROW_HEIGHT: u32 = 11;

impl PickerLayout {
    pub fn new(width: u32, height: u32) -> Option<Self> {
        let card_width = width.checked_sub(32)?.min(280);
        let card_height = height.checked_sub(24)?.min(170);
        if card_width < 120 || card_height < 60 {
            return None;
        }
        let card_x = (width - card_width) / 2;
        let card_y = (height - card_height) / 2;
        Some(Self {
            card_x,
            card_y,
            card_width,
            card_height,
            rows_y: card_y + 22,
        })
    }

    /// The row index under a click, if any.
    pub fn row_at(&self, x: u32, y: u32) -> Option<usize> {
        if x < self.card_x || x >= self.card_x + self.card_width || y < self.rows_y {
            return None;
        }
        let row = ((y - self.rows_y) / PICKER_ROW_HEIGHT) as usize;
        (y < self.card_y + self.card_height).then_some(row)
    }
}

pub fn picker_title(locale: Locale) -> &'static str {
    text(locale, "albert.picker.title")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> PickerEntry {
        PickerEntry {
            name: name.to_owned(),
            size: 1,
        }
    }

    #[test]
    fn filters_sorts_and_bounds_entries() {
        let picker = FilePickerState::new(
            vec![
                entry("b.txt"),
                entry("a.TXT"),
                entry("photo.png"),
                entry(".hidden.txt"),
                entry("bad\nname.txt"),
            ],
            &[".txt".to_owned()],
        );
        let names: Vec<_> = picker.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.TXT", "b.txt"]);

        let many = (0..30)
            .map(|index| entry(&format!("f{index:02}.txt")))
            .collect();
        assert_eq!(
            FilePickerState::new(many, &[]).entries().len(),
            MAX_PICKER_ENTRIES
        );
    }

    #[test]
    fn keyboard_moves_selection_and_chooses_an_absolute_documents_path() {
        let mut picker = FilePickerState::new(vec![entry("a.txt"), entry("b.txt")], &[]);
        assert_eq!(picker.handle_key(PickerKey::Up), PickerOutcome::Pending);
        assert_eq!(picker.selected(), Some(0));
        picker.handle_key(PickerKey::Down);
        picker.handle_key(PickerKey::Down);
        assert_eq!(picker.selected(), Some(1));
        assert_eq!(
            picker.handle_key(PickerKey::Enter),
            PickerOutcome::Chosen("/Documents/b.txt".to_owned())
        );
        assert_eq!(
            picker.handle_key(PickerKey::Escape),
            PickerOutcome::Canceled
        );
    }

    #[test]
    fn an_empty_picker_cannot_choose_anything() {
        let mut picker = FilePickerState::new(Vec::new(), &[]);
        assert_eq!(picker.selected(), None);
        assert_eq!(picker.handle_key(PickerKey::Enter), PickerOutcome::Pending);
        assert_eq!(picker.choose(3), PickerOutcome::Pending);
    }

    #[test]
    fn accept_filters_match_extensions() {
        assert!(accepts("x.txt", &[]));
        assert!(accepts("x.TXT", &["txt".to_owned()]));
        assert!(accepts("x.bin", &["*".to_owned()]));
        assert!(!accepts("x.txt.png", &[".txt".to_owned()]));
        assert!(!accepts("txt", &[".txt".to_owned()]));
    }

    #[test]
    fn rows_hit_test_inside_the_card() {
        let layout = PickerLayout::new(320, 200).unwrap();
        assert_eq!(layout.row_at(layout.card_x + 5, layout.rows_y), Some(0));
        assert_eq!(
            layout.row_at(layout.card_x + 5, layout.rows_y + PICKER_ROW_HEIGHT + 1),
            Some(1)
        );
        assert_eq!(layout.row_at(0, layout.rows_y), None);
        assert_eq!(layout.row_at(layout.card_x + 5, layout.card_y), None);
        assert!(PickerLayout::new(100, 50).is_none());
    }

    #[test]
    fn picker_title_is_localized() {
        assert_eq!(picker_title(Locale::EnUs), "Choose a file");
        assert_eq!(picker_title(Locale::JaJp), "ファイル選択");
    }
}
