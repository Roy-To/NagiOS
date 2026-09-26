//! Backend-independent component types and small deterministic state models.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentKind {
    Button,
    IconButton,
    TextField,
    SearchField,
    Checkbox,
    Radio,
    Switch,
    List,
    ListItem,
    Sidebar,
    Toolbar,
    TabSegment,
    Menu,
    MenuItem,
    ContextMenu,
    Dialog,
    Sheet,
    Popover,
    WindowContentFrame,
    SettingsRow,
    SettingsPage,
    ProgressIndicator,
    EmptyState,
    StatusBadge,
    Tooltip,
    Divider,
    ScrollContainer,
    CommandPalette,
    CommandResultRow,
    ShortcutHint,
    SectionHeader,
}

pub const COMPONENT_KINDS: &[ComponentKind] = &[
    ComponentKind::Button,
    ComponentKind::IconButton,
    ComponentKind::TextField,
    ComponentKind::SearchField,
    ComponentKind::Checkbox,
    ComponentKind::Radio,
    ComponentKind::Switch,
    ComponentKind::List,
    ComponentKind::ListItem,
    ComponentKind::Sidebar,
    ComponentKind::Toolbar,
    ComponentKind::TabSegment,
    ComponentKind::Menu,
    ComponentKind::MenuItem,
    ComponentKind::ContextMenu,
    ComponentKind::Dialog,
    ComponentKind::Sheet,
    ComponentKind::Popover,
    ComponentKind::WindowContentFrame,
    ComponentKind::SettingsRow,
    ComponentKind::SettingsPage,
    ComponentKind::ProgressIndicator,
    ComponentKind::EmptyState,
    ComponentKind::StatusBadge,
    ComponentKind::Tooltip,
    ComponentKind::Divider,
    ComponentKind::ScrollContainer,
    ComponentKind::CommandPalette,
    ComponentKind::CommandResultRow,
    ComponentKind::ShortcutHint,
    ComponentKind::SectionHeader,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentFamily {
    Action,
    Input,
    Selection,
    Navigation,
    Surface,
    Feedback,
    Content,
    Command,
}

pub const fn component_family(kind: ComponentKind) -> ComponentFamily {
    match kind {
        ComponentKind::Button | ComponentKind::IconButton => ComponentFamily::Action,
        ComponentKind::TextField | ComponentKind::SearchField => ComponentFamily::Input,
        ComponentKind::Checkbox | ComponentKind::Radio | ComponentKind::Switch => {
            ComponentFamily::Selection
        }
        ComponentKind::List
        | ComponentKind::ListItem
        | ComponentKind::Sidebar
        | ComponentKind::Toolbar
        | ComponentKind::TabSegment
        | ComponentKind::Menu
        | ComponentKind::MenuItem
        | ComponentKind::ContextMenu => ComponentFamily::Navigation,
        ComponentKind::Dialog
        | ComponentKind::Sheet
        | ComponentKind::Popover
        | ComponentKind::WindowContentFrame
        | ComponentKind::ScrollContainer => ComponentFamily::Surface,
        ComponentKind::ProgressIndicator
        | ComponentKind::EmptyState
        | ComponentKind::StatusBadge
        | ComponentKind::Tooltip
        | ComponentKind::Divider => ComponentFamily::Feedback,
        ComponentKind::SettingsRow | ComponentKind::SettingsPage | ComponentKind::SectionHeader => {
            ComponentFamily::Content
        }
        ComponentKind::CommandPalette
        | ComponentKind::CommandResultRow
        | ComponentKind::ShortcutHint => ComponentFamily::Command,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionState {
    Idle,
    Hovered,
    Pressed,
    Focused,
    Selected,
    Invalid,
    Busy,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyCode {
    Enter,
    Space,
    Escape,
    Tab,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonEvent {
    PointerEnter,
    PointerLeave,
    PointerDown,
    PointerUp { inside: bool },
    Focus,
    Blur,
    KeyDown(KeyCode),
    KeyUp(KeyCode),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonOutcome {
    Activated,
}

/// State contract for Button and IconButton. The app performs the returned action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ButtonModel {
    enabled: bool,
    busy: bool,
    hovered: bool,
    focused: bool,
    pointer_armed: bool,
    space_armed: bool,
    enter_armed: bool,
}

impl ButtonModel {
    pub const fn new(enabled: bool) -> Self {
        Self {
            enabled,
            busy: false,
            hovered: false,
            focused: false,
            pointer_armed: false,
            space_armed: false,
            enter_armed: false,
        }
    }

    pub const fn is_enabled(self) -> bool {
        self.enabled && !self.busy
    }

    pub const fn is_focused(self) -> bool {
        self.focused
    }

    pub const fn is_hovered(self) -> bool {
        self.hovered
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !self.is_enabled() {
            self.clear_arms();
        }
    }

    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
        if busy {
            self.clear_arms();
        }
    }

    pub fn handle(&mut self, event: ButtonEvent) -> Option<ButtonOutcome> {
        match event {
            ButtonEvent::PointerEnter => self.hovered = true,
            ButtonEvent::PointerLeave => self.hovered = false,
            ButtonEvent::Focus => self.focused = true,
            ButtonEvent::Blur => {
                self.focused = false;
                self.space_armed = false;
                self.enter_armed = false;
            }
            ButtonEvent::PointerDown => {
                self.pointer_armed = self.is_enabled();
            }
            ButtonEvent::PointerUp { inside } => {
                let activate = self.pointer_armed && inside && self.is_enabled();
                self.pointer_armed = false;
                if activate {
                    return Some(ButtonOutcome::Activated);
                }
            }
            ButtonEvent::KeyDown(KeyCode::Enter) => {
                if self.is_enabled() && self.focused && !self.enter_armed {
                    self.enter_armed = true;
                    return Some(ButtonOutcome::Activated);
                }
            }
            ButtonEvent::KeyUp(KeyCode::Enter) => self.enter_armed = false,
            ButtonEvent::KeyDown(KeyCode::Space) => {
                if self.is_enabled() && self.focused {
                    self.space_armed = true;
                }
            }
            ButtonEvent::KeyUp(KeyCode::Space) => {
                let activate = self.space_armed && self.is_enabled();
                self.space_armed = false;
                if activate {
                    return Some(ButtonOutcome::Activated);
                }
            }
            ButtonEvent::KeyDown(_) | ButtonEvent::KeyUp(_) => {}
        }
        None
    }

    pub const fn visual_state(self) -> InteractionState {
        if !self.enabled {
            InteractionState::Disabled
        } else if self.busy {
            InteractionState::Busy
        } else if self.pointer_armed || self.space_armed {
            InteractionState::Pressed
        } else if self.focused {
            InteractionState::Focused
        } else if self.hovered {
            InteractionState::Hovered
        } else {
            InteractionState::Idle
        }
    }

    fn clear_arms(&mut self) {
        self.pointer_armed = false;
        self.space_armed = false;
        self.enter_armed = false;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToggleKind {
    Checkbox,
    Radio,
    Switch,
}

/// State contract shared by Checkbox, Radio, and Switch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToggleModel {
    kind: ToggleKind,
    selected: bool,
    enabled: bool,
    focused: bool,
    space_armed: bool,
    enter_armed: bool,
}

impl ToggleModel {
    pub const fn new(kind: ToggleKind, selected: bool, enabled: bool) -> Self {
        Self {
            kind,
            selected,
            enabled,
            focused: false,
            space_armed: false,
            enter_armed: false,
        }
    }

    pub const fn is_selected(self) -> bool {
        self.selected
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.focused = false;
            self.space_armed = false;
            self.enter_armed = false;
        }
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused && self.enabled;
        if !self.focused {
            self.space_armed = false;
            self.enter_armed = false;
        }
    }

    /// Apply one semantic activation. Radio activation selects; it never clears itself.
    pub fn activate(&mut self) -> Option<bool> {
        if !self.enabled {
            return None;
        }
        let next = match self.kind {
            ToggleKind::Radio => true,
            ToggleKind::Checkbox | ToggleKind::Switch => !self.selected,
        };
        if next == self.selected {
            return None;
        }
        self.selected = next;
        Some(next)
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Option<bool> {
        if !self.enabled || !self.focused {
            return None;
        }
        match key {
            KeyCode::Enter if !self.enter_armed => {
                self.enter_armed = true;
                self.activate()
            }
            KeyCode::Space => {
                self.space_armed = true;
                None
            }
            _ => None,
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Option<bool> {
        match key {
            KeyCode::Enter => {
                self.enter_armed = false;
                None
            }
            KeyCode::Space => {
                let activate = self.space_armed && self.enabled && self.focused;
                self.space_armed = false;
                if activate {
                    self.activate()
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextFieldAction {
    Submit,
    Cancel,
}

/// Editing storage and Unicode cursor movement stay with the injected text-input owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextFieldModel {
    enabled: bool,
    read_only: bool,
    focused: bool,
}

impl TextFieldModel {
    pub const fn new(enabled: bool, read_only: bool) -> Self {
        Self {
            enabled,
            read_only,
            focused: false,
        }
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused && self.enabled;
    }

    pub fn handle_key_down(&self, key: KeyCode) -> Option<TextFieldAction> {
        if !self.enabled || !self.focused {
            return None;
        }
        match key {
            KeyCode::Enter => Some(TextFieldAction::Submit),
            KeyCode::Escape => Some(TextFieldAction::Cancel),
            _ => None,
        }
    }

    pub const fn accepts_edits(self) -> bool {
        self.enabled && self.focused && !self.read_only
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_and_busy_buttons_never_activate() {
        let mut disabled = ButtonModel::new(false);
        assert_eq!(disabled.handle(ButtonEvent::KeyDown(KeyCode::Enter)), None);
        disabled.handle(ButtonEvent::PointerDown);
        assert_eq!(
            disabled.handle(ButtonEvent::PointerUp { inside: true }),
            None
        );

        let mut busy = ButtonModel::new(true);
        busy.set_busy(true);
        assert_eq!(busy.visual_state(), InteractionState::Busy);
        assert_eq!(busy.handle(ButtonEvent::KeyDown(KeyCode::Enter)), None);
    }

    #[test]
    fn button_activation_is_defined_for_pointer_enter_and_space() {
        let mut button = ButtonModel::new(true);
        button.handle(ButtonEvent::PointerDown);
        assert_eq!(
            button.handle(ButtonEvent::PointerUp { inside: false }),
            None
        );
        button.handle(ButtonEvent::PointerDown);
        assert_eq!(
            button.handle(ButtonEvent::PointerUp { inside: true }),
            Some(ButtonOutcome::Activated)
        );
        assert_eq!(button.handle(ButtonEvent::KeyDown(KeyCode::Space)), None);
        button.handle(ButtonEvent::Focus);
        button.handle(ButtonEvent::KeyDown(KeyCode::Space));
        assert_eq!(button.handle(ButtonEvent::KeyDown(KeyCode::Space)), None);
        assert_eq!(
            button.handle(ButtonEvent::KeyUp(KeyCode::Space)),
            Some(ButtonOutcome::Activated)
        );
    }

    #[test]
    fn enter_key_repeat_does_not_repeat_activation() {
        let mut button = ButtonModel::new(true);
        assert_eq!(button.handle(ButtonEvent::KeyDown(KeyCode::Enter)), None);
        button.handle(ButtonEvent::Focus);
        assert_eq!(
            button.handle(ButtonEvent::KeyDown(KeyCode::Enter)),
            Some(ButtonOutcome::Activated)
        );
        assert_eq!(button.handle(ButtonEvent::KeyDown(KeyCode::Enter)), None);
        button.handle(ButtonEvent::KeyUp(KeyCode::Enter));
        assert_eq!(
            button.handle(ButtonEvent::KeyDown(KeyCode::Enter)),
            Some(ButtonOutcome::Activated)
        );
    }

    #[test]
    fn toggles_keep_checkbox_switch_and_radio_semantics_distinct() {
        let mut checkbox = ToggleModel::new(ToggleKind::Checkbox, false, true);
        assert_eq!(checkbox.activate(), Some(true));
        assert_eq!(checkbox.activate(), Some(false));

        let mut radio = ToggleModel::new(ToggleKind::Radio, true, true);
        assert_eq!(radio.activate(), None);

        let mut disabled_switch = ToggleModel::new(ToggleKind::Switch, false, false);
        assert_eq!(disabled_switch.activate(), None);
        assert_eq!(disabled_switch.handle_key_down(KeyCode::Enter), None);
    }

    #[test]
    fn toggle_space_activates_on_key_release_and_enter_does_not_repeat() {
        let mut switch = ToggleModel::new(ToggleKind::Switch, false, true);
        assert_eq!(switch.handle_key_down(KeyCode::Space), None);
        switch.set_focused(true);
        assert_eq!(switch.handle_key_down(KeyCode::Space), None);
        assert_eq!(switch.handle_key_up(KeyCode::Space), Some(true));
        assert_eq!(switch.handle_key_down(KeyCode::Enter), Some(false));
        assert_eq!(switch.handle_key_down(KeyCode::Enter), None);
        switch.handle_key_up(KeyCode::Enter);
        assert_eq!(switch.handle_key_down(KeyCode::Enter), Some(true));
    }

    #[test]
    fn text_field_key_actions_require_focus_and_enabled_state() {
        let mut field = TextFieldModel::new(true, false);
        assert_eq!(field.handle_key_down(KeyCode::Enter), None);
        field.set_focused(true);
        assert_eq!(
            field.handle_key_down(KeyCode::Enter),
            Some(TextFieldAction::Submit)
        );
        assert!(field.accepts_edits());
        let readonly = TextFieldModel::new(true, true);
        assert!(!readonly.accepts_edits());
    }

    #[test]
    fn every_required_visual_primitive_has_a_public_component_kind() {
        assert!(COMPONENT_KINDS.contains(&ComponentKind::Button));
        assert!(COMPONENT_KINDS.contains(&ComponentKind::IconButton));
        assert!(COMPONENT_KINDS.contains(&ComponentKind::SearchField));
        assert!(COMPONENT_KINDS.contains(&ComponentKind::Sidebar));
        assert!(COMPONENT_KINDS.contains(&ComponentKind::MenuItem));
        assert!(COMPONENT_KINDS.contains(&ComponentKind::CommandPalette));
        assert_eq!(
            component_family(ComponentKind::SettingsRow),
            ComponentFamily::Content
        );
    }
}
