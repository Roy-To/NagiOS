//! Narrow synchronous console surface shared by Desktop and first-party apps.

use crate::ui::Painter;
use libnagi::InputEvent;
use std::{io, time::Duration};

pub(crate) trait NagiConsole {
    fn present(&mut self, render: impl FnOnce(&mut Painter<'_>)) -> io::Result<()>;
    fn next_key_timeout(&mut self, timeout: Duration) -> io::Result<Option<InputEvent>>;
}
