//! Fixed-capacity, deterministic keyboard focus traversal.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FocusTarget {
    id: u16,
    enabled: bool,
}

impl FocusTarget {
    pub const fn new(id: u16, enabled: bool) -> Self {
        Self { id, enabled }
    }

    pub const fn id(self) -> u16 {
        self.id
    }

    pub const fn is_enabled(self) -> bool {
        self.enabled
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusDirection {
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationBehavior {
    /// Arrow keys move the active row; Enter or Space changes selection.
    ActivateSelection,
    /// Arrow keys also select the focused item, as in a tab strip.
    SelectOnFocus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationAction {
    Focused(usize),
    Selected(usize),
}

/// Shared roving-focus contract for lists, tabs, menus, sidebars, and toolbars.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NavigationModel<const N: usize> {
    enabled: [bool; N],
    active: Option<usize>,
    selected: Option<usize>,
    behavior: NavigationBehavior,
}

impl<const N: usize> NavigationModel<N> {
    pub const fn new(
        enabled: [bool; N],
        selected: Option<usize>,
        behavior: NavigationBehavior,
    ) -> Self {
        let selected = match selected {
            Some(index) if index < N => Some(index),
            _ => None,
        };
        let active = match selected {
            Some(index) if enabled[index] => Some(index),
            _ => None,
        };
        Self {
            enabled,
            active,
            selected: active,
            behavior,
        }
    }

    pub const fn active(&self) -> Option<usize> {
        self.active
    }

    pub const fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn set_enabled(&mut self, index: usize, enabled: bool) -> bool {
        let Some(item) = self.enabled.get_mut(index) else {
            return false;
        };
        *item = enabled;
        if !enabled {
            if self.active == Some(index) {
                self.active = None;
            }
            if self.selected == Some(index) {
                self.selected = None;
            }
        }
        true
    }

    pub fn handle_key(&mut self, key: crate::interaction::KeyCode) -> Option<NavigationAction> {
        use crate::interaction::KeyCode;
        match key {
            KeyCode::ArrowDown | KeyCode::ArrowRight => self.move_active(true),
            KeyCode::ArrowUp | KeyCode::ArrowLeft => self.move_active(false),
            KeyCode::Home => self.first_enabled(),
            KeyCode::End => self.last_enabled(),
            KeyCode::Enter | KeyCode::Space => self.activate_active(),
            _ => None,
        }
    }

    fn move_active(&mut self, forward: bool) -> Option<NavigationAction> {
        if N == 0 {
            return None;
        }
        let current = self.active;
        for step in 1..=N {
            let index = match (forward, current) {
                (true, Some(current)) => (current + step) % N,
                (false, Some(current)) => (current + N - (step % N)) % N,
                (true, None) => step - 1,
                (false, None) => N - step,
            };
            if self.enabled[index] {
                return Some(self.set_active(index));
            }
        }
        self.active = None;
        None
    }

    fn first_enabled(&mut self) -> Option<NavigationAction> {
        let index = self.enabled.iter().position(|enabled| *enabled)?;
        Some(self.set_active(index))
    }

    fn last_enabled(&mut self) -> Option<NavigationAction> {
        let index = self.enabled.iter().rposition(|enabled| *enabled)?;
        Some(self.set_active(index))
    }

    fn set_active(&mut self, index: usize) -> NavigationAction {
        self.active = Some(index);
        if self.behavior == NavigationBehavior::SelectOnFocus {
            self.selected = Some(index);
            NavigationAction::Selected(index)
        } else {
            NavigationAction::Focused(index)
        }
    }

    fn activate_active(&mut self) -> Option<NavigationAction> {
        let index = self.active?;
        if !self.enabled[index] {
            return None;
        }
        self.selected = Some(index);
        Some(NavigationAction::Selected(index))
    }
}

/// Owns focus order for one view. Disabled targets are skipped and traversal wraps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FocusManager<const N: usize> {
    targets: [FocusTarget; N],
    focused: Option<u16>,
}

impl<const N: usize> FocusManager<N> {
    pub const fn new(targets: [FocusTarget; N]) -> Self {
        Self {
            targets,
            focused: None,
        }
    }

    pub const fn focused(&self) -> Option<u16> {
        self.focused
    }

    pub fn focus(&mut self, id: u16) -> bool {
        if self
            .targets
            .iter()
            .any(|target| target.id == id && target.enabled)
        {
            self.focused = Some(id);
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self) {
        self.focused = None;
    }

    pub fn set_enabled(&mut self, id: u16, enabled: bool) -> bool {
        let Some(target) = self.targets.iter_mut().find(|target| target.id == id) else {
            return false;
        };
        target.enabled = enabled;
        if !enabled && self.focused == Some(id) {
            self.focused = None;
        }
        true
    }

    pub fn advance(&mut self, direction: FocusDirection) -> Option<u16> {
        if N == 0 {
            self.focused = None;
            return None;
        }
        let current = self
            .focused
            .and_then(|id| self.targets.iter().position(|target| target.id == id));
        for step in 1..=N {
            let index = match (direction, current) {
                (FocusDirection::Forward, Some(index)) => (index + step) % N,
                (FocusDirection::Backward, Some(index)) => (index + N - (step % N)) % N,
                (FocusDirection::Forward, None) => step - 1,
                (FocusDirection::Backward, None) => N - step,
            };
            if self.targets[index].enabled {
                self.focused = Some(self.targets[index].id);
                return self.focused;
            }
        }
        self.focused = None;
        None
    }
}

/// A modal's local tab order. Callers restore the returned opener ID after
/// closing; background focus targets are never members of this scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FocusScope<const N: usize> {
    manager: FocusManager<N>,
    restore_target: Option<u16>,
    active: bool,
}

impl<const N: usize> FocusScope<N> {
    pub fn new(
        targets: [FocusTarget; N],
        restore_target: Option<u16>,
        initial_focus: Option<u16>,
    ) -> Self {
        let mut manager = FocusManager::new(targets);
        if !initial_focus.is_some_and(|id| manager.focus(id)) {
            manager.advance(FocusDirection::Forward);
        }
        Self {
            manager,
            restore_target,
            active: true,
        }
    }

    pub const fn is_active(self) -> bool {
        self.active
    }

    pub const fn focused(&self) -> Option<u16> {
        self.manager.focused()
    }

    pub fn focus(&mut self, id: u16) -> bool {
        self.active && self.manager.focus(id)
    }

    pub fn handle_tab(&mut self, shift_pressed: bool) -> Option<u16> {
        if !self.active {
            return None;
        }
        self.manager.advance(if shift_pressed {
            FocusDirection::Backward
        } else {
            FocusDirection::Forward
        })
    }

    /// Deactivate the scope and return the ID that the adapter should focus.
    pub fn close(&mut self) -> Option<u16> {
        if !self.active {
            return None;
        }
        self.active = false;
        self.manager.clear();
        self.restore_target.take()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectAction {
    Opened(Option<usize>),
    Focused(usize),
    Selected(usize),
    Cancelled,
    Dismissed,
}

/// Fixed-capacity combo-box/select behavior. Labels and option actions stay in
/// the app; this model only tracks enabled and selected option indices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectModel<const N: usize> {
    enabled: [bool; N],
    selected: Option<usize>,
    active: Option<usize>,
    open: bool,
}

impl<const N: usize> SelectModel<N> {
    pub const fn new(enabled: [bool; N], selected: Option<usize>) -> Self {
        let selected = match selected {
            Some(index) if index < N && enabled[index] => Some(index),
            _ => None,
        };
        Self {
            enabled,
            selected,
            active: selected,
            open: false,
        }
    }

    pub const fn is_open(self) -> bool {
        self.open
    }

    pub const fn selected(self) -> Option<usize> {
        self.selected
    }

    pub const fn active(self) -> Option<usize> {
        self.active
    }

    pub fn set_enabled(&mut self, index: usize, enabled: bool) -> bool {
        let Some(option) = self.enabled.get_mut(index) else {
            return false;
        };
        *option = enabled;
        if !enabled {
            if self.selected == Some(index) {
                self.selected = None;
            }
            if self.active == Some(index) {
                self.active = self.selected;
            }
        }
        true
    }

    pub fn handle_key(&mut self, key: crate::interaction::KeyCode) -> Option<SelectAction> {
        use crate::interaction::KeyCode;
        if !self.open {
            return match key {
                KeyCode::Enter | KeyCode::Space => {
                    self.open = true;
                    self.active = self.selected.or_else(|| self.first_enabled());
                    Some(SelectAction::Opened(self.active))
                }
                _ => None,
            };
        }
        match key {
            KeyCode::ArrowDown | KeyCode::ArrowRight => {
                self.move_active(true).map(SelectAction::Focused)
            }
            KeyCode::ArrowUp | KeyCode::ArrowLeft => {
                self.move_active(false).map(SelectAction::Focused)
            }
            KeyCode::Home => self.first_enabled().map(|index| {
                self.active = Some(index);
                SelectAction::Focused(index)
            }),
            KeyCode::End => self.last_enabled().map(|index| {
                self.active = Some(index);
                SelectAction::Focused(index)
            }),
            KeyCode::Enter | KeyCode::Space => self.commit_active(),
            KeyCode::Escape => {
                self.open = false;
                self.active = self.selected;
                Some(SelectAction::Cancelled)
            }
            KeyCode::Tab => {
                self.open = false;
                self.active = self.selected;
                Some(SelectAction::Dismissed)
            }
            _ => None,
        }
    }

    pub fn choose(&mut self, index: usize) -> Option<SelectAction> {
        if !self.open || !self.enabled.get(index).copied().unwrap_or(false) {
            return None;
        }
        self.active = Some(index);
        self.commit_active()
    }

    fn move_active(&mut self, forward: bool) -> Option<usize> {
        if N == 0 {
            return None;
        }
        for step in 1..=N {
            let index = match (forward, self.active) {
                (true, Some(active)) => (active + step) % N,
                (false, Some(active)) => (active + N - (step % N)) % N,
                (true, None) => step - 1,
                (false, None) => N - step,
            };
            if self.enabled[index] {
                self.active = Some(index);
                return Some(index);
            }
        }
        self.active = None;
        None
    }

    fn first_enabled(&self) -> Option<usize> {
        self.enabled.iter().position(|enabled| *enabled)
    }

    fn last_enabled(&self) -> Option<usize> {
        self.enabled.iter().rposition(|enabled| *enabled)
    }

    fn commit_active(&mut self) -> Option<SelectAction> {
        let index = self.active?;
        if !self.enabled[index] {
            return None;
        }
        self.selected = Some(index);
        self.active = Some(index);
        self.open = false;
        Some(SelectAction::Selected(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_skips_disabled_targets_and_wraps() {
        let mut focus = FocusManager::new([
            FocusTarget::new(10, true),
            FocusTarget::new(11, false),
            FocusTarget::new(12, true),
        ]);
        assert_eq!(focus.advance(FocusDirection::Forward), Some(10));
        assert_eq!(focus.advance(FocusDirection::Forward), Some(12));
        assert_eq!(focus.advance(FocusDirection::Forward), Some(10));
        assert_eq!(focus.advance(FocusDirection::Backward), Some(12));
    }

    #[test]
    fn focus_cannot_be_assigned_to_disabled_targets() {
        let mut focus = FocusManager::new([FocusTarget::new(4, false)]);
        assert!(!focus.focus(4));
        assert_eq!(focus.advance(FocusDirection::Forward), None);
        assert!(focus.set_enabled(4, true));
        assert!(focus.focus(4));
        assert_eq!(focus.focused(), Some(4));
        assert!(focus.set_enabled(4, false));
        assert_eq!(focus.focused(), None);
    }

    #[test]
    fn empty_focus_ring_is_safe() {
        let mut focus = FocusManager::<0>::new([]);
        assert_eq!(focus.advance(FocusDirection::Forward), None);
        assert_eq!(focus.advance(FocusDirection::Backward), None);
    }

    #[test]
    fn list_navigation_skips_disabled_items_and_tabs_select_on_focus() {
        use crate::interaction::KeyCode;

        let mut list = NavigationModel::new(
            [true, false, true],
            Some(0),
            NavigationBehavior::ActivateSelection,
        );
        assert_eq!(
            list.handle_key(KeyCode::ArrowDown),
            Some(NavigationAction::Focused(2))
        );
        assert_eq!(list.selected(), Some(0));
        assert_eq!(
            list.handle_key(KeyCode::Enter),
            Some(NavigationAction::Selected(2))
        );

        let mut tabs =
            NavigationModel::new([true, true], Some(0), NavigationBehavior::SelectOnFocus);
        assert_eq!(
            tabs.handle_key(KeyCode::ArrowRight),
            Some(NavigationAction::Selected(1))
        );
        assert_eq!(tabs.selected(), Some(1));
    }

    #[test]
    fn navigation_home_end_and_empty_lists_are_defined() {
        use crate::interaction::KeyCode;

        let mut navigation = NavigationModel::new(
            [false, true, false, true],
            None,
            NavigationBehavior::ActivateSelection,
        );
        assert_eq!(
            navigation.handle_key(KeyCode::Home),
            Some(NavigationAction::Focused(1))
        );
        assert_eq!(
            navigation.handle_key(KeyCode::End),
            Some(NavigationAction::Focused(3))
        );
        let mut empty = NavigationModel::<0>::new([], None, NavigationBehavior::ActivateSelection);
        assert_eq!(empty.handle_key(KeyCode::ArrowDown), None);
    }

    #[test]
    fn modal_focus_scope_contains_tab_traversal_and_restores_the_opener() {
        let mut scope = FocusScope::new(
            [
                FocusTarget::new(10, true),
                FocusTarget::new(11, false),
                FocusTarget::new(12, true),
            ],
            Some(99),
            Some(10),
        );
        assert_eq!(scope.focused(), Some(10));
        assert!(!scope.focus(99));
        assert_eq!(scope.handle_tab(false), Some(12));
        assert_eq!(scope.handle_tab(false), Some(10));
        assert_eq!(scope.handle_tab(true), Some(12));
        assert_eq!(scope.close(), Some(99));
        assert_eq!(scope.focused(), None);
        assert_eq!(scope.close(), None);
    }

    #[test]
    fn select_keyboard_navigation_skips_disabled_options_and_escape_preserves_value() {
        use crate::interaction::KeyCode;

        let mut select = SelectModel::new([true, false, true], Some(0));
        assert_eq!(
            select.handle_key(KeyCode::Enter),
            Some(SelectAction::Opened(Some(0)))
        );
        assert_eq!(
            select.handle_key(KeyCode::ArrowDown),
            Some(SelectAction::Focused(2))
        );
        assert_eq!(
            select.handle_key(KeyCode::Escape),
            Some(SelectAction::Cancelled)
        );
        assert_eq!(select.selected(), Some(0));
        assert!(!select.is_open());

        assert_eq!(
            select.handle_key(KeyCode::Space),
            Some(SelectAction::Opened(Some(0)))
        );
        assert_eq!(
            select.handle_key(KeyCode::ArrowDown),
            Some(SelectAction::Focused(2))
        );
        assert_eq!(
            select.handle_key(KeyCode::Enter),
            Some(SelectAction::Selected(2))
        );
        assert_eq!(select.selected(), Some(2));
    }

    #[test]
    fn tab_and_reverse_tab_select_open_dialog_scope_order() {
        let mut scope = FocusScope::new(
            [FocusTarget::new(20, true), FocusTarget::new(21, true)],
            Some(5),
            None,
        );
        assert_eq!(scope.handle_tab(true), Some(21));
        assert_eq!(scope.handle_tab(false), Some(20));
    }

    #[test]
    fn select_defaults_reject_disabled_values_and_allow_option_disable() {
        let mut select = SelectModel::new([true, false], Some(1));
        assert_eq!(select.selected(), None);
        assert_eq!(select.active(), None);
        assert!(select.set_enabled(1, true));
        assert_eq!(
            select.handle_key(crate::interaction::KeyCode::Enter),
            Some(SelectAction::Opened(Some(0)))
        );
        assert_eq!(select.choose(1), Some(SelectAction::Selected(1)));
        assert!(select.set_enabled(1, false));
        assert_eq!(select.selected(), None);
        assert!(!select.set_enabled(9, true));
    }
}
