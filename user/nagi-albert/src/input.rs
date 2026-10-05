//! Bounded pointer hit testing and evdev key translation for Albert chrome.

use crate::ui::{BrowserChromeAction, BrowserChromeView};

pub const CHROME_HEIGHT: u32 = 48;

/// Map a primary-button press to the visible browser chrome control.
pub fn chrome_action_at(
    view: &BrowserChromeView,
    width: u32,
    x: u32,
    y: u32,
) -> Option<BrowserChromeAction> {
    if x >= width || y >= CHROME_HEIGHT {
        return None;
    }

    if y < 20 {
        if x >= width.saturating_sub(20) {
            return Some(BrowserChromeAction::NewTab);
        }
        let active_index = view
            .tabs
            .iter()
            .position(|tab| tab.active)
            .unwrap_or_default();
        let visible_start = active_index.saturating_sub(2);
        let visible_end = visible_start.saturating_add(3).min(view.tabs.len());
        let tab_area_width = width.saturating_sub(56);
        let tab_width = tab_area_width
            .saturating_sub(8)
            .div_euclid(3)
            .clamp(40, 170);
        return view
            .tabs
            .iter()
            .skip(visible_start)
            .take(visible_end.saturating_sub(visible_start))
            .enumerate()
            .find_map(|(slot, tab)| {
                let left = 4 + slot as u32 * (tab_width + 3);
                (x >= left && x < left.saturating_add(tab_width))
                    .then_some(BrowserChromeAction::SelectTab(tab.id))
            });
    }

    if (23..43).contains(&y) {
        return match x {
            0..26 => Some(BrowserChromeAction::Back),
            26..51 => Some(BrowserChromeAction::Forward),
            51..77 => Some(BrowserChromeAction::Reload),
            79.. => Some(BrowserChromeAction::FocusAddressBar),
            _ => None,
        };
    }

    None
}

/// Translate the small US-layout evdev subset needed to enter web addresses.
/// The OS input service remains the source of events; this does not read a
/// host keyboard or grant the browser device authority.
pub fn evdev_character(code: u16, shifted: bool) -> Option<char> {
    let (plain, shifted_value) = match code {
        2..=11 => {
            const DIGITS: [char; 10] = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'];
            const SHIFTED: [char; 10] = ['!', '@', '#', '$', '%', '^', '&', '*', '(', ')'];
            let index = usize::from(code - 2);
            (DIGITS[index], SHIFTED[index])
        }
        12 => ('-', '_'),
        13 => ('=', '+'),
        16..=25 => {
            const TOP: [char; 10] = ['q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p'];
            (
                TOP[usize::from(code - 16)],
                TOP[usize::from(code - 16)].to_ascii_uppercase(),
            )
        }
        26 => ('[', '{'),
        27 => (']', '}'),
        30..=38 => {
            const HOME: [char; 9] = ['a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l'];
            let value = HOME[usize::from(code - 30)];
            (value, value.to_ascii_uppercase())
        }
        39 => (';', ':'),
        40 => ('\'', '"'),
        41 => ('`', '~'),
        43 => ('\\', '|'),
        44..=50 => {
            const BOTTOM: [char; 7] = ['z', 'x', 'c', 'v', 'b', 'n', 'm'];
            let value = BOTTOM[usize::from(code - 44)];
            (value, value.to_ascii_uppercase())
        }
        51 => (',', '<'),
        52 => ('.', '>'),
        53 => ('/', '?'),
        57 => (' ', ' '),
        _ => return None,
    };
    Some(if shifted { shifted_value } else { plain })
}

pub const fn is_shift_key(code: u16) -> bool {
    matches!(code, 42 | 54)
}

pub const fn is_backspace_key(code: u16) -> bool {
    code == 14
}

pub const fn is_enter_key(code: u16) -> bool {
    code == 28
}

pub const fn is_escape_key(code: u16) -> bool {
    code == 1
}

pub const fn is_control_key(code: u16) -> bool {
    matches!(code, 29 | 97)
}

/// Clipboard shortcut named by a key press while Control is held.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardShortcut {
    Copy,
    Cut,
    Paste,
}

/// Classify evdev `code` pressed with Control held (US layout).
pub const fn clipboard_shortcut(code: u16, control: bool) -> Option<ClipboardShortcut> {
    if !control {
        return None;
    }
    match code {
        46 => Some(ClipboardShortcut::Copy),
        45 => Some(ClipboardShortcut::Cut),
        47 => Some(ClipboardShortcut::Paste),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        chrome_action_at, clipboard_shortcut, evdev_character, is_control_key, BrowserChromeAction,
        ClipboardShortcut, CHROME_HEIGHT,
    };

    #[test]
    fn clipboard_shortcuts_require_control() {
        assert!(is_control_key(29) && is_control_key(97));
        assert!(!is_control_key(42));
        assert_eq!(clipboard_shortcut(46, true), Some(ClipboardShortcut::Copy));
        assert_eq!(clipboard_shortcut(45, true), Some(ClipboardShortcut::Cut));
        assert_eq!(clipboard_shortcut(47, true), Some(ClipboardShortcut::Paste));
        assert_eq!(clipboard_shortcut(47, false), None);
        assert_eq!(clipboard_shortcut(30, true), None);
    }
    use crate::browser_state::BrowserState;
    use crate::tabs::TabId;
    use crate::ui::{self, BrowserChromeOutcome};

    #[test]
    fn hit_testing_tracks_toolbar_controls_and_excludes_page_content() {
        let state = BrowserState::new();
        let view = ui::view(&state);
        assert_eq!(
            chrome_action_at(&view, 320, 12, 31),
            Some(BrowserChromeAction::Back)
        );
        assert_eq!(
            chrome_action_at(&view, 320, 100, 30),
            Some(BrowserChromeAction::FocusAddressBar)
        );
        assert_eq!(
            chrome_action_at(&view, 320, 305, 10),
            Some(BrowserChromeAction::NewTab)
        );
        assert_eq!(chrome_action_at(&view, 320, 100, CHROME_HEIGHT), None);
    }

    #[test]
    fn entering_an_address_through_chrome_dispatches_a_typed_navigation() {
        let mut state = BrowserState::new();
        let address_action = chrome_action_at(&ui::view(&state), 320, 100, 30).unwrap();
        assert_eq!(
            ui::dispatch(&mut state, address_action, 0),
            Ok(BrowserChromeOutcome::Changed)
        );

        for code in [18, 45, 30, 50, 25, 38, 18, 52, 46, 24, 50] {
            let character = evdev_character(code, false).unwrap();
            assert_eq!(
                ui::dispatch(
                    &mut state,
                    BrowserChromeAction::InsertAddressText(character.to_string()),
                    1,
                ),
                Ok(BrowserChromeOutcome::Changed)
            );
        }

        assert_eq!(state.address_bar().text(), "example.com");
        let outcome = ui::dispatch(&mut state, BrowserChromeAction::SubmitAddress, 2).unwrap();
        let BrowserChromeOutcome::Navigate(request) = outcome else {
            panic!("address submission did not produce a navigation request");
        };
        assert_eq!(request.tab_id, TabId(1));
        assert_eq!(request.url, "https://example.com/");
    }

    #[test]
    fn evdev_translation_respects_shift_and_named_browser_keys() {
        assert_eq!(evdev_character(18, false), Some('e'));
        assert_eq!(evdev_character(18, true), Some('E'));
        assert_eq!(evdev_character(52, true), Some('>'));
        assert_eq!(evdev_character(0, false), None);
    }
}
