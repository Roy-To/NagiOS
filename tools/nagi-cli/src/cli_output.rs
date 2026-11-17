//! Fallible terminal output for commands whose result must survive a closed pipe.

use std::fmt;
use std::io::{self, Write};

pub(crate) fn write_stdout(message: fmt::Arguments<'_>) -> io::Result<()> {
    io::stdout().lock().write_fmt(message)
}

pub(crate) fn write_stderr(message: fmt::Arguments<'_>) -> io::Result<()> {
    io::stderr().lock().write_fmt(message)
}
