//! Dialog default, cancel, and Escape behavior contract.

use crate::interaction::KeyCode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionId(pub u16);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogAction {
    Default(ActionId),
    Cancel(ActionId),
    Dismissed,
}

/// A dialog reports a response to its owner; it never performs the associated action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DialogModel {
    open: bool,
    default_action: Option<ActionId>,
    cancel_action: Option<ActionId>,
}

impl DialogModel {
    pub const fn new(default_action: Option<ActionId>, cancel_action: Option<ActionId>) -> Self {
        Self {
            open: true,
            default_action,
            cancel_action,
        }
    }

    pub const fn is_open(self) -> bool {
        self.open
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Option<DialogAction> {
        if !self.open {
            return None;
        }
        match key {
            KeyCode::Enter => {
                let action = self.default_action.map(DialogAction::Default)?;
                self.open = false;
                Some(action)
            }
            KeyCode::Escape => {
                self.open = false;
                Some(match self.cancel_action {
                    Some(action) => DialogAction::Cancel(action),
                    None => DialogAction::Dismissed,
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_uses_the_declared_default_action_once() {
        let mut dialog = DialogModel::new(Some(ActionId(3)), Some(ActionId(4)));
        assert_eq!(
            dialog.handle_key_down(KeyCode::Enter),
            Some(DialogAction::Default(ActionId(3)))
        );
        assert!(!dialog.is_open());
        assert_eq!(dialog.handle_key_down(KeyCode::Enter), None);
    }

    #[test]
    fn escape_uses_cancel_or_dismisses_when_no_cancel_action_exists() {
        let mut cancellable = DialogModel::new(Some(ActionId(3)), Some(ActionId(4)));
        assert_eq!(
            cancellable.handle_key_down(KeyCode::Escape),
            Some(DialogAction::Cancel(ActionId(4)))
        );
        let mut dismissible = DialogModel::new(None, None);
        assert_eq!(
            dismissible.handle_key_down(KeyCode::Escape),
            Some(DialogAction::Dismissed)
        );
        assert_eq!(dismissible.handle_key_down(KeyCode::Escape), None);
    }
}
