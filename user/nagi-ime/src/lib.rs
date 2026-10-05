//! Nagi user-space Japanese input method.
//!
//! The engine turns layout-translated key presses into composition results
//! (preedit updates and commits). It has no device, display, or OS authority:
//! the client that owns trusted keyboard input feeds it keys and forwards the
//! results to the focused text field. Input language is independent of the
//! System language (see `docs/architecture/language-architecture.md`).
//!
//! Conversion candidates come from a [`CandidateSource`]. Nagi 0.1 ships the
//! [`KanaCandidates`] source (hiragana and katakana); kanji conversion needs a
//! dictionary-backed source.

#![no_std]

extern crate alloc;

mod romaji;

use alloc::string::String;
use alloc::vec::Vec;

pub use romaji::{to_katakana, RomajiComposer};

/// Upper bound on composed characters, so a stuck key cannot grow the
/// preedit without limit.
pub const MAX_PREEDIT_CHARS: usize = 64;
/// Upper bound on candidates taken from a source.
pub const MAX_CANDIDATES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputMode {
    /// Keys reach the application unchanged (English US layout).
    Direct,
    /// Letters compose hiragana.
    Hiragana,
}

/// A key press, already translated through the active keyboard layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImeKey {
    Char(char),
    Space,
    Enter,
    Backspace,
    Escape,
    /// Ctrl+Space or Zenkaku/Hankaku.
    ToggleMode,
    /// Henkan (JIS keyboards).
    ModeOn,
    /// Muhenkan (JIS keyboards).
    ModeOff,
    /// F6.
    ConvertHiragana,
    /// F7.
    ConvertKatakana,
    /// Any other key (arrows, Tab, function keys...).
    Other,
}

/// What the client must do for one key, applied in field order: end the
/// current composition with `commit`, show `preedit` (starting a new
/// composition when `preedit_started`), then deliver the original key if
/// `pass_through` is set.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImeResponse {
    /// Ends the active composition. An empty string cancels it.
    pub commit: Option<String>,
    pub preedit: Option<String>,
    pub preedit_started: bool,
    pub pass_through: bool,
    pub mode_changed: Option<InputMode>,
}

impl ImeResponse {
    fn pass_through() -> Self {
        Self {
            pass_through: true,
            ..Self::default()
        }
    }

    fn commit(text: String, pass_through: bool) -> Self {
        Self {
            commit: Some(text),
            pass_through,
            ..Self::default()
        }
    }
}

/// Produces conversion candidates for a hiragana reading. The first
/// candidate should be the reading itself.
pub trait CandidateSource {
    fn candidates(&self, reading: &str) -> Vec<String>;
}

/// Hiragana and katakana forms of the reading.
#[derive(Clone, Copy, Debug, Default)]
pub struct KanaCandidates;

impl CandidateSource for KanaCandidates {
    fn candidates(&self, reading: &str) -> Vec<String> {
        let mut candidates = Vec::with_capacity(2);
        candidates.push(String::from(reading));
        let katakana = to_katakana(reading);
        if katakana != reading {
            candidates.push(katakana);
        }
        candidates
    }
}

#[derive(Clone, Debug)]
struct Conversion {
    candidates: Vec<String>,
    selected: usize,
}

impl Conversion {
    fn selected_text(&self) -> String {
        self.candidates[self.selected].clone()
    }
}

#[derive(Debug)]
pub struct InputMethod<Source: CandidateSource> {
    mode: InputMode,
    composer: RomajiComposer,
    conversion: Option<Conversion>,
    source: Source,
}

impl<Source: CandidateSource> InputMethod<Source> {
    pub fn new(source: Source) -> Self {
        Self {
            mode: InputMode::Direct,
            composer: RomajiComposer::default(),
            conversion: None,
            source,
        }
    }

    pub fn mode(&self) -> InputMode {
        self.mode
    }

    pub fn is_composing(&self) -> bool {
        self.conversion.is_some() || !self.composer.is_empty()
    }

    /// Current preedit text, if composing.
    pub fn preedit(&self) -> Option<String> {
        if let Some(conversion) = &self.conversion {
            return Some(conversion.selected_text());
        }
        (!self.composer.is_empty()).then(|| self.composer.preedit())
    }

    /// Drop any composition without committing, for example when the text
    /// field loses focus.
    pub fn reset(&mut self) {
        self.composer.clear();
        self.conversion = None;
    }

    pub fn handle(&mut self, key: ImeKey) -> ImeResponse {
        let mode = match key {
            ImeKey::ToggleMode => Some(match self.mode {
                InputMode::Direct => InputMode::Hiragana,
                InputMode::Hiragana => InputMode::Direct,
            }),
            ImeKey::ModeOn => Some(InputMode::Hiragana),
            ImeKey::ModeOff => Some(InputMode::Direct),
            _ => None,
        };
        if let Some(mode) = mode {
            let commit = self.is_composing().then(|| self.take_commit_text());
            self.mode = mode;
            return ImeResponse {
                commit,
                mode_changed: Some(mode),
                ..ImeResponse::default()
            };
        }
        if self.mode == InputMode::Direct {
            return ImeResponse::pass_through();
        }
        if self.conversion.is_some() {
            self.handle_converting(key)
        } else {
            self.handle_composing(key)
        }
    }

    fn take_commit_text(&mut self) -> String {
        if let Some(conversion) = self.conversion.take() {
            self.composer.clear();
            return conversion.selected_text();
        }
        self.composer.finish()
    }

    fn with_preedit(&self, mut response: ImeResponse, started: bool) -> ImeResponse {
        response.preedit = self.preedit();
        response.preedit_started = started;
        response
    }

    fn handle_composing(&mut self, key: ImeKey) -> ImeResponse {
        let composing = !self.composer.is_empty();
        match key {
            ImeKey::Char(character) => {
                if self.composer.char_count() >= MAX_PREEDIT_CHARS {
                    return ImeResponse::default();
                }
                if self.composer.push(character) {
                    self.with_preedit(ImeResponse::default(), !composing)
                } else if composing {
                    ImeResponse::commit(self.composer.finish(), true)
                } else {
                    ImeResponse::pass_through()
                }
            }
            _ if !composing => ImeResponse::pass_through(),
            ImeKey::Space => {
                let reading = self.composer.finish();
                let candidates = bounded(self.source.candidates(&reading), &reading);
                // The first Space moves past the reading to the first
                // alternative, as Japanese IMEs do.
                let selected = usize::from(candidates.len() > 1);
                self.conversion = Some(Conversion {
                    candidates,
                    selected,
                });
                self.with_preedit(ImeResponse::default(), false)
            }
            ImeKey::ConvertHiragana | ImeKey::ConvertKatakana => {
                let reading = self.composer.finish();
                let mut conversion = Conversion {
                    candidates: bounded(self.source.candidates(&reading), &reading),
                    selected: 0,
                };
                let target = if key == ImeKey::ConvertKatakana {
                    to_katakana(&reading)
                } else {
                    reading
                };
                select_or_append(&mut conversion, target);
                self.conversion = Some(conversion);
                self.with_preedit(ImeResponse::default(), false)
            }
            ImeKey::Enter => ImeResponse::commit(self.composer.finish(), false),
            ImeKey::Backspace => {
                self.composer.backspace();
                if self.composer.is_empty() {
                    ImeResponse::commit(String::new(), false)
                } else {
                    self.with_preedit(ImeResponse::default(), false)
                }
            }
            ImeKey::Escape => {
                self.composer.clear();
                ImeResponse::commit(String::new(), false)
            }
            ImeKey::Other => ImeResponse::commit(self.composer.finish(), true),
            ImeKey::ToggleMode | ImeKey::ModeOn | ImeKey::ModeOff => ImeResponse::default(),
        }
    }

    fn handle_converting(&mut self, key: ImeKey) -> ImeResponse {
        let Some(conversion) = self.conversion.as_mut() else {
            return ImeResponse::pass_through();
        };
        match key {
            ImeKey::Space => {
                conversion.selected = (conversion.selected + 1) % conversion.candidates.len();
                self.with_preedit(ImeResponse::default(), false)
            }
            ImeKey::Escape | ImeKey::Backspace => {
                // Return to the editable reading.
                let reading = conversion.candidates[0].clone();
                self.conversion = None;
                self.composer.set_reading(&reading);
                self.with_preedit(ImeResponse::default(), false)
            }
            ImeKey::ConvertHiragana => {
                let reading = conversion.candidates[0].clone();
                select_or_append(conversion, reading);
                self.with_preedit(ImeResponse::default(), false)
            }
            ImeKey::ConvertKatakana => {
                let katakana = to_katakana(&conversion.candidates[0]);
                select_or_append(conversion, katakana);
                self.with_preedit(ImeResponse::default(), false)
            }
            ImeKey::Enter => ImeResponse::commit(self.take_commit_text(), false),
            ImeKey::Char(_) => {
                // Typing continues: commit the selection, then compose anew.
                let committed = self.take_commit_text();
                let mut next = self.handle_composing(key);
                next.commit = Some(committed);
                next
            }
            ImeKey::Other => ImeResponse::commit(self.take_commit_text(), true),
            ImeKey::ToggleMode | ImeKey::ModeOn | ImeKey::ModeOff => ImeResponse::default(),
        }
    }
}

fn bounded(mut candidates: Vec<String>, reading: &str) -> Vec<String> {
    candidates.truncate(MAX_CANDIDATES);
    if candidates.is_empty() {
        candidates.push(String::from(reading));
    }
    candidates
}

fn select_or_append(conversion: &mut Conversion, text: String) {
    if let Some(index) = conversion
        .candidates
        .iter()
        .position(|candidate| *candidate == text)
    {
        conversion.selected = index;
    } else if conversion.candidates.len() < MAX_CANDIDATES {
        conversion.candidates.push(text);
        conversion.selected = conversion.candidates.len() - 1;
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    fn ime() -> InputMethod<KanaCandidates> {
        let mut ime = InputMethod::new(KanaCandidates);
        ime.handle(ImeKey::ToggleMode);
        ime
    }

    fn type_text(ime: &mut InputMethod<KanaCandidates>, text: &str) -> Vec<ImeResponse> {
        text.chars().map(|c| ime.handle(ImeKey::Char(c))).collect()
    }

    #[test]
    fn direct_mode_passes_every_key_through() {
        let mut ime = InputMethod::new(KanaCandidates);
        assert_eq!(ime.mode(), InputMode::Direct);
        assert!(ime.handle(ImeKey::Char('a')).pass_through);
        assert!(ime.handle(ImeKey::Enter).pass_through);
        assert!(!ime.is_composing());
    }

    #[test]
    fn typing_starts_updates_and_commits_a_composition() {
        let mut ime = ime();
        let responses = type_text(&mut ime, "nihongo");
        assert!(responses[0].preedit_started);
        assert_eq!(responses[0].preedit.as_deref(), Some("n"));
        assert!(responses[1..].iter().all(|r| !r.preedit_started));
        assert_eq!(responses[6].preedit.as_deref(), Some("にほんご"));
        let commit = ime.handle(ImeKey::Enter);
        assert_eq!(commit.commit.as_deref(), Some("にほんご"));
        assert!(!commit.pass_through);
        assert!(!ime.is_composing());
    }

    #[test]
    fn keys_outside_a_composition_pass_through_in_hiragana_mode() {
        let mut ime = ime();
        assert!(ime.handle(ImeKey::Enter).pass_through);
        assert!(ime.handle(ImeKey::Backspace).pass_through);
        assert!(ime.handle(ImeKey::Space).pass_through);
        assert!(ime.handle(ImeKey::Char('1')).pass_through);
    }

    #[test]
    fn space_cycles_kana_candidates_and_escape_returns_to_reading() {
        let mut ime = ime();
        type_text(&mut ime, "nagi");
        assert_eq!(ime.handle(ImeKey::Space).preedit.as_deref(), Some("ナギ"));
        assert_eq!(ime.handle(ImeKey::Space).preedit.as_deref(), Some("なぎ"));
        ime.handle(ImeKey::Space);
        assert_eq!(ime.handle(ImeKey::Escape).preedit.as_deref(), Some("なぎ"));
        // The reading is editable again.
        assert_eq!(ime.handle(ImeKey::Backspace).preedit.as_deref(), Some("な"));
    }

    #[test]
    fn function_keys_select_hiragana_or_katakana() {
        let mut ime = ime();
        type_text(&mut ime, "ra-men");
        assert_eq!(
            ime.handle(ImeKey::ConvertKatakana).preedit.as_deref(),
            Some("ラーメン")
        );
        assert_eq!(
            ime.handle(ImeKey::ConvertHiragana).preedit.as_deref(),
            Some("らーめん")
        );
        assert_eq!(
            ime.handle(ImeKey::Enter).commit.as_deref(),
            Some("らーめん")
        );
    }

    #[test]
    fn typing_during_conversion_commits_and_starts_a_new_composition() {
        let mut ime = ime();
        type_text(&mut ime, "ka");
        ime.handle(ImeKey::Space);
        let response = ime.handle(ImeKey::Char('n'));
        assert_eq!(response.commit.as_deref(), Some("カ"));
        assert_eq!(response.preedit.as_deref(), Some("n"));
        assert!(response.preedit_started);
        assert!(!response.pass_through);
    }

    #[test]
    fn backspace_and_escape_cancel_with_an_empty_commit() {
        let mut ime = ime();
        type_text(&mut ime, "a");
        assert_eq!(ime.handle(ImeKey::Backspace).commit.as_deref(), Some(""));
        type_text(&mut ime, "ka");
        assert_eq!(ime.handle(ImeKey::Escape).commit.as_deref(), Some(""));
        assert!(!ime.is_composing());
    }

    #[test]
    fn other_keys_commit_then_pass_through() {
        let mut ime = ime();
        type_text(&mut ime, "kan");
        let response = ime.handle(ImeKey::Other);
        assert_eq!(response.commit.as_deref(), Some("かん"));
        assert!(response.pass_through);
        type_text(&mut ime, "a");
        let response = ime.handle(ImeKey::Char('1'));
        assert_eq!(response.commit.as_deref(), Some("あ"));
        assert!(response.pass_through);
    }

    #[test]
    fn mode_changes_commit_pending_text() {
        let mut ime = ime();
        type_text(&mut ime, "nihon");
        let response = ime.handle(ImeKey::ModeOff);
        assert_eq!(response.mode_changed, Some(InputMode::Direct));
        assert_eq!(response.commit.as_deref(), Some("にほん"));
        assert!(ime.handle(ImeKey::Char('a')).pass_through);
        assert_eq!(
            ime.handle(ImeKey::ModeOn).mode_changed,
            Some(InputMode::Hiragana)
        );
    }

    #[test]
    fn preedit_is_bounded() {
        let mut ime = ime();
        for _ in 0..(MAX_PREEDIT_CHARS + 10) {
            ime.handle(ImeKey::Char('a'));
        }
        assert_eq!(ime.preedit().unwrap().chars().count(), MAX_PREEDIT_CHARS);
    }
}
