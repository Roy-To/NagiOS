//! Translate Linux evdev input into Calendar form and Agenda actions.

use nagi_abi::{InputEvent, INPUT_EVENT_KEY};
use nagi_ui::interaction::KeyCode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CalendarKey {
    Navigation(KeyCode),
    Backspace,
    Delete,
    Text(char),
}

#[derive(Default)]
pub(crate) struct CalendarKeys {
    left_shift: bool,
    right_shift: bool,
}

impl CalendarKeys {
    pub(crate) fn translate(&mut self, event: InputEvent) -> Option<CalendarKey> {
        if event.event_type != INPUT_EVENT_KEY {
            return None;
        }
        let shift = match event.code {
            42 => Some(&mut self.left_shift),
            54 => Some(&mut self.right_shift),
            _ => None,
        };
        if let Some(shift) = shift {
            match event.value {
                0 => *shift = false,
                1 => *shift = true,
                _ => {}
            }
            return None;
        }
        if event.value != 1 || self.left_shift || self.right_shift {
            return None;
        }
        use CalendarKey::{Backspace, Delete, Navigation, Text};
        Some(match event.code {
            1 => Navigation(KeyCode::Escape),
            14 => Backspace,
            15 => Navigation(KeyCode::Tab),
            28 | 96 => Navigation(KeyCode::Enter),
            103 => Navigation(KeyCode::ArrowUp),
            105 => Navigation(KeyCode::ArrowLeft),
            106 => Navigation(KeyCode::ArrowRight),
            108 => Navigation(KeyCode::ArrowDown),
            111 => Delete,
            2..=10 => Text(char::from(b'1' + (event.code - 2) as u8)),
            11 => Text('0'),
            12 => Text('-'),
            57 => Text(' '),
            16..=25 => Text(char::from(b"qwertyuiop"[(event.code - 16) as usize])),
            30..=38 => Text(char::from(b"asdfghjkl"[(event.code - 30) as usize])),
            44..=50 => Text(char::from(b"zxcvbnm"[(event.code - 44) as usize])),
            _ => return None,
        })
    }
}
