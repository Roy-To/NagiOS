//! Accessibility metadata contract for renderer and input adapters.

use crate::interaction::ComponentKind;
use crate::text::MessageKey;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibleRole {
    Button,
    TextField,
    SearchBox,
    Checkbox,
    RadioButton,
    Switch,
    List,
    ListItem,
    Navigation,
    Toolbar,
    Tab,
    Menu,
    MenuItem,
    Dialog,
    Group,
    ProgressIndicator,
    Status,
    Tooltip,
    Separator,
    ScrollArea,
    CommandPalette,
    Command,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckedState {
    NotApplicable,
    Unchecked,
    Checked,
    Mixed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibleState {
    pub disabled: bool,
    pub selected: bool,
    pub checked: CheckedState,
    pub expanded: Option<bool>,
    pub busy: bool,
    pub invalid: bool,
    pub focused: bool,
}

impl AccessibleState {
    pub const fn new() -> Self {
        Self {
            disabled: false,
            selected: false,
            checked: CheckedState::NotApplicable,
            expanded: None,
            busy: false,
            invalid: false,
            focused: false,
        }
    }
}

impl Default for AccessibleState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardOperation {
    None,
    ActivateEnterOrSpace,
    EditText,
    NavigateAndActivate,
    RovingFocus,
    DialogDefaultCancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibleNode {
    pub role: AccessibleRole,
    pub name: MessageKey,
    pub description: Option<MessageKey>,
    pub state: AccessibleState,
    pub focusable: bool,
    pub keyboard: KeyboardOperation,
}

pub const fn role_for_component(kind: ComponentKind) -> AccessibleRole {
    match kind {
        ComponentKind::Button | ComponentKind::IconButton => AccessibleRole::Button,
        ComponentKind::TextField => AccessibleRole::TextField,
        ComponentKind::SearchField => AccessibleRole::SearchBox,
        ComponentKind::Checkbox => AccessibleRole::Checkbox,
        ComponentKind::Radio => AccessibleRole::RadioButton,
        ComponentKind::Switch => AccessibleRole::Switch,
        ComponentKind::List => AccessibleRole::List,
        ComponentKind::ListItem => AccessibleRole::ListItem,
        ComponentKind::Sidebar => AccessibleRole::Navigation,
        ComponentKind::Toolbar => AccessibleRole::Toolbar,
        ComponentKind::TabSegment => AccessibleRole::Tab,
        ComponentKind::Menu | ComponentKind::ContextMenu => AccessibleRole::Menu,
        ComponentKind::MenuItem => AccessibleRole::MenuItem,
        ComponentKind::Dialog | ComponentKind::Sheet | ComponentKind::Popover => {
            AccessibleRole::Dialog
        }
        ComponentKind::WindowContentFrame
        | ComponentKind::SettingsRow
        | ComponentKind::SettingsPage
        | ComponentKind::SectionHeader => AccessibleRole::Group,
        ComponentKind::ProgressIndicator => AccessibleRole::ProgressIndicator,
        ComponentKind::EmptyState | ComponentKind::StatusBadge => AccessibleRole::Status,
        ComponentKind::Tooltip => AccessibleRole::Tooltip,
        ComponentKind::Divider => AccessibleRole::Separator,
        ComponentKind::ScrollContainer => AccessibleRole::ScrollArea,
        ComponentKind::CommandPalette => AccessibleRole::CommandPalette,
        ComponentKind::CommandResultRow | ComponentKind::ShortcutHint => AccessibleRole::Command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessibility_metadata_keeps_role_name_state_and_keyboard_contract() {
        let mut state = AccessibleState::new();
        state.checked = CheckedState::Checked;
        state.focused = true;
        let node = AccessibleNode {
            role: role_for_component(ComponentKind::Switch),
            name: MessageKey::new("settings.wifi.enabled"),
            description: Some(MessageKey::new("settings.wifi.description")),
            state,
            focusable: true,
            keyboard: KeyboardOperation::ActivateEnterOrSpace,
        };
        assert_eq!(node.role, AccessibleRole::Switch);
        assert_eq!(node.state.checked, CheckedState::Checked);
        assert!(node.focusable);
        assert_eq!(node.keyboard, KeyboardOperation::ActivateEnterOrSpace);
    }

    #[test]
    fn command_palette_and_settings_components_have_semantic_roles() {
        assert_eq!(
            role_for_component(ComponentKind::CommandPalette),
            AccessibleRole::CommandPalette
        );
        assert_eq!(
            role_for_component(ComponentKind::SettingsRow),
            AccessibleRole::Group
        );
        assert_eq!(
            role_for_component(ComponentKind::MenuItem),
            AccessibleRole::MenuItem
        );
    }
}
