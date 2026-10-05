//! Trusted consent prompt (ADR 0060).
//!
//! The OS-owned dialog that turns a `GrantCheck::ConsentRequired` into a
//! user decision (spec §23). The prompt is a pure input state machine; the
//! desktop renders it and feeds it input, and the Supervisor records the
//! answer with the signed-in user's session.
//!
//! - **Fresh input only.** The prompt ignores every event until the desktop
//!   has presented it (`arm`). After that, a choice is made only by a press
//!   *and* its release that both arrive while armed, so a key or button held
//!   down before the dialog appeared cannot answer it.
//! - **Safe default.** Focus starts on `Deny`, so an unintended Enter denies.
//! - **Dismissal is not consent.** Escape dismisses the prompt without
//!   recording a decision; the capability stays `ConsentRequired`.
//!
//! The module is allocation-free and has no kernel dependency, so its policy
//! is host-testable.

use crate::launch::GrantDecision;
use crate::{
    INPUT_BUTTON_PRIMARY, INPUT_KEY_ENTER, INPUT_KEY_ESCAPE, INPUT_KEY_SPACE, INPUT_KEY_TAB,
};
use nagi_model::AppSessionId;

/// The dialog's buttons, in left-to-right order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsentChoice {
    Deny,
    AllowOnce,
    Allow,
}

impl ConsentChoice {
    pub const ALL: [Self; 3] = [Self::Deny, Self::AllowOnce, Self::Allow];

    const fn next(self) -> Self {
        match self {
            Self::Deny => Self::AllowOnce,
            Self::AllowOnce => Self::Allow,
            Self::Allow => Self::Deny,
        }
    }
}

/// How the user answered the prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsentAnswer {
    Chosen(ConsentChoice),
    /// Escape: no decision is recorded.
    Dismissed,
}

impl ConsentAnswer {
    /// The registry decision for this answer in `session`, if any.
    pub const fn decision(self, session: AppSessionId) -> Option<GrantDecision> {
        match self {
            Self::Chosen(ConsentChoice::Deny) => Some(GrantDecision::Deny),
            Self::Chosen(ConsentChoice::AllowOnce) => Some(GrantDecision::AllowOnce(session)),
            Self::Chosen(ConsentChoice::Allow) => Some(GrantDecision::Allow),
            Self::Dismissed => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptOutcome {
    /// The event did not affect the prompt.
    Ignored,
    /// Focus or pressed state changed; redraw.
    Changed,
    Answered(ConsentAnswer),
}

/// An activation that has been pressed but not yet released.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pressed {
    Key(u16),
    Pointer(ConsentChoice),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConsentPrompt {
    focus: ConsentChoice,
    armed: bool,
    pressed: Option<Pressed>,
    answered: bool,
}

impl Default for ConsentPrompt {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsentPrompt {
    pub const fn new() -> Self {
        Self {
            focus: ConsentChoice::Deny,
            armed: false,
            pressed: None,
            answered: false,
        }
    }

    pub const fn focus(&self) -> ConsentChoice {
        self.focus
    }

    pub const fn is_armed(&self) -> bool {
        self.armed
    }

    /// Whether `choice` is currently held down by the pointer.
    pub fn is_pressed(&self, choice: ConsentChoice) -> bool {
        self.pressed == Some(Pressed::Pointer(choice))
    }

    /// Accept input from now on. The desktop calls this after the dialog
    /// frame has been presented on screen.
    pub fn arm(&mut self) {
        self.armed = true;
    }

    /// A key press (`pressed`) or release.
    pub fn handle_key(&mut self, code: u16, pressed: bool) -> PromptOutcome {
        if !self.armed || self.answered {
            return PromptOutcome::Ignored;
        }
        match (code, pressed) {
            (INPUT_KEY_TAB, true) => {
                self.focus = self.focus.next();
                self.pressed = None;
                PromptOutcome::Changed
            }
            (INPUT_KEY_ESCAPE, true) => self.answer(ConsentAnswer::Dismissed),
            (INPUT_KEY_ENTER | INPUT_KEY_SPACE, true) => {
                self.pressed = Some(Pressed::Key(code));
                PromptOutcome::Changed
            }
            (INPUT_KEY_ENTER | INPUT_KEY_SPACE, false) => {
                if self.pressed == Some(Pressed::Key(code)) {
                    self.answer(ConsentAnswer::Chosen(self.focus))
                } else {
                    PromptOutcome::Ignored
                }
            }
            _ => PromptOutcome::Ignored,
        }
    }

    /// A primary-button press or release over `target` (the button under
    /// the pointer, if any). A choice needs press and release on the same
    /// button.
    pub fn handle_pointer(
        &mut self,
        code: u16,
        pressed: bool,
        target: Option<ConsentChoice>,
    ) -> PromptOutcome {
        if !self.armed || self.answered || code != INPUT_BUTTON_PRIMARY {
            return PromptOutcome::Ignored;
        }
        if pressed {
            self.pressed = target.map(Pressed::Pointer);
            if let Some(choice) = target {
                self.focus = choice;
            }
            return PromptOutcome::Changed;
        }
        match (self.pressed.take(), target) {
            (Some(Pressed::Pointer(held)), Some(released)) if held == released => {
                self.answer(ConsentAnswer::Chosen(released))
            }
            (Some(_), _) => PromptOutcome::Changed,
            (None, _) => PromptOutcome::Ignored,
        }
    }

    fn answer(&mut self, answer: ConsentAnswer) -> PromptOutcome {
        self.answered = true;
        self.pressed = None;
        PromptOutcome::Answered(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn armed() -> ConsentPrompt {
        let mut prompt = ConsentPrompt::new();
        prompt.arm();
        prompt
    }

    fn key(prompt: &mut ConsentPrompt, code: u16) -> PromptOutcome {
        prompt.handle_key(code, true);
        prompt.handle_key(code, false)
    }

    #[test]
    fn input_before_the_dialog_is_presented_is_ignored() {
        let mut prompt = ConsentPrompt::new();
        assert_eq!(
            prompt.handle_key(INPUT_KEY_ENTER, true),
            PromptOutcome::Ignored
        );
        assert_eq!(
            prompt.handle_pointer(INPUT_BUTTON_PRIMARY, true, Some(ConsentChoice::Allow)),
            PromptOutcome::Ignored
        );
        prompt.arm();
        // Releases of presses that started before arming answer nothing.
        assert_eq!(
            prompt.handle_key(INPUT_KEY_ENTER, false),
            PromptOutcome::Ignored
        );
        assert_eq!(
            prompt.handle_pointer(INPUT_BUTTON_PRIMARY, false, Some(ConsentChoice::Allow)),
            PromptOutcome::Ignored
        );
        assert_eq!(prompt.focus(), ConsentChoice::Deny);
    }

    #[test]
    fn focus_starts_on_deny_and_enter_denies() {
        let mut prompt = armed();
        assert_eq!(
            key(&mut prompt, INPUT_KEY_ENTER),
            PromptOutcome::Answered(ConsentAnswer::Chosen(ConsentChoice::Deny))
        );
    }

    #[test]
    fn tab_cycles_choices_and_space_activates() {
        let mut prompt = armed();
        assert_eq!(key(&mut prompt, INPUT_KEY_TAB), PromptOutcome::Ignored);
        assert_eq!(prompt.focus(), ConsentChoice::AllowOnce);
        key(&mut prompt, INPUT_KEY_TAB);
        assert_eq!(prompt.focus(), ConsentChoice::Allow);
        key(&mut prompt, INPUT_KEY_TAB);
        assert_eq!(prompt.focus(), ConsentChoice::Deny);
        key(&mut prompt, INPUT_KEY_TAB);
        assert_eq!(
            key(&mut prompt, INPUT_KEY_SPACE),
            PromptOutcome::Answered(ConsentAnswer::Chosen(ConsentChoice::AllowOnce))
        );
    }

    #[test]
    fn activation_needs_the_release_of_the_same_key() {
        let mut prompt = armed();
        prompt.handle_key(INPUT_KEY_SPACE, true);
        assert_eq!(
            prompt.handle_key(INPUT_KEY_ENTER, false),
            PromptOutcome::Ignored
        );
        // Moving focus abandons the pending press.
        prompt.handle_key(INPUT_KEY_TAB, true);
        assert_eq!(
            prompt.handle_key(INPUT_KEY_SPACE, false),
            PromptOutcome::Ignored
        );
    }

    #[test]
    fn pointer_needs_press_and_release_on_the_same_button() {
        let mut prompt = armed();
        prompt.handle_pointer(INPUT_BUTTON_PRIMARY, true, Some(ConsentChoice::Allow));
        assert!(prompt.is_pressed(ConsentChoice::Allow));
        assert_eq!(
            prompt.handle_pointer(INPUT_BUTTON_PRIMARY, false, Some(ConsentChoice::Deny)),
            PromptOutcome::Changed
        );
        // A press outside every button arms nothing.
        prompt.handle_pointer(INPUT_BUTTON_PRIMARY, true, None);
        assert_eq!(
            prompt.handle_pointer(INPUT_BUTTON_PRIMARY, false, Some(ConsentChoice::Allow)),
            PromptOutcome::Ignored
        );
        prompt.handle_pointer(INPUT_BUTTON_PRIMARY, true, Some(ConsentChoice::Allow));
        assert_eq!(
            prompt.handle_pointer(INPUT_BUTTON_PRIMARY, false, Some(ConsentChoice::Allow)),
            PromptOutcome::Answered(ConsentAnswer::Chosen(ConsentChoice::Allow))
        );
    }

    #[test]
    fn escape_dismisses_without_a_decision_and_answers_once() {
        let mut prompt = armed();
        let PromptOutcome::Answered(answer) = prompt.handle_key(INPUT_KEY_ESCAPE, true) else {
            panic!("escape did not dismiss");
        };
        assert_eq!(answer, ConsentAnswer::Dismissed);
        assert_eq!(answer.decision(AppSessionId(7)), None);
        assert_eq!(key(&mut prompt, INPUT_KEY_ENTER), PromptOutcome::Ignored);
    }

    #[test]
    fn answers_map_to_registry_decisions() {
        let session = AppSessionId(9);
        assert_eq!(
            ConsentAnswer::Chosen(ConsentChoice::Deny).decision(session),
            Some(GrantDecision::Deny)
        );
        assert_eq!(
            ConsentAnswer::Chosen(ConsentChoice::AllowOnce).decision(session),
            Some(GrantDecision::AllowOnce(session))
        );
        assert_eq!(
            ConsentAnswer::Chosen(ConsentChoice::Allow).decision(session),
            Some(GrantDecision::Allow)
        );
    }
}
