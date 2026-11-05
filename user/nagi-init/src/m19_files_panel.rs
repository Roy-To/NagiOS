//! Keyboard/mouse state for the ordinary Files management panel.

use crate::desktop::files::{Entry, Error};
use alloc::{string::String, vec::Vec};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Operation {
    Create(String),
    Rename(Entry, String),
    Trash(Entry),
    Restore(Entry),
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Focus {
    View,
    List,
    New,
    Rename,
    Trash,
    Name,
    Apply,
    Cancel,
}

pub(super) struct Panel {
    pub open: bool,
    pub trash_view: bool,
    pub entries: Vec<Entry>,
    pub selected: usize,
    pub focus: Focus,
    pub editing: bool,
    rename: Option<Entry>,
    pub name: String,
    pub status: &'static str,
    pub refresh: bool,
    pub pending: Option<Operation>,
}

impl Panel {
    pub const fn new() -> Self {
        Self {
            open: false,
            trash_view: false,
            entries: Vec::new(),
            selected: 0,
            focus: Focus::List,
            editing: false,
            rename: None,
            name: String::new(),
            status: "",
            refresh: false,
            pending: None,
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.refresh = true;
        self.status = "";
        self.focus = Focus::List;
    }

    pub fn replace_entries(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    pub fn activate(&mut self, focus: Focus) {
        if self.pending.is_some() {
            return;
        }
        self.focus = focus;
        match focus {
            Focus::View => {
                self.trash_view = !self.trash_view;
                self.selected = 0;
                self.status = "";
                self.refresh = true;
            }
            Focus::New if !self.trash_view => {
                self.editing = true;
                self.rename = None;
                self.name.clear();
                self.focus = Focus::Name;
                self.status = "";
            }
            Focus::Rename if !self.trash_view => {
                if let Some(entry) = self.entries.get(self.selected).cloned() {
                    self.editing = true;
                    self.name = entry.name.clone();
                    self.rename = Some(entry);
                    self.focus = Focus::Name;
                    self.status = "";
                }
            }
            Focus::Trash => {
                if let Some(entry) = self.entries.get(self.selected).cloned() {
                    self.pending = Some(if self.trash_view {
                        Operation::Restore(entry)
                    } else {
                        Operation::Trash(entry)
                    });
                }
            }
            Focus::Name | Focus::Apply if self.editing => {
                if !crate::desktop::files::valid_name(self.name.as_bytes()) {
                    self.status = "desktop.files.operation.invalid_name";
                    return;
                }
                self.pending = Some(match self.rename.clone() {
                    Some(entry) => Operation::Rename(entry, self.name.clone()),
                    None => Operation::Create(self.name.clone()),
                });
            }
            Focus::Cancel => self.cancel_edit(),
            _ => {}
        }
    }

    fn cancel_edit(&mut self) {
        self.editing = false;
        self.rename = None;
        self.name.clear();
        self.focus = Focus::List;
        self.status = "";
    }

    pub fn key(&mut self, code: u16) {
        if self.pending.is_some() {
            return;
        }
        match code {
            libnagi::INPUT_KEY_ESCAPE => {
                if self.editing {
                    self.cancel_edit();
                } else {
                    self.open = false;
                }
            }
            libnagi::INPUT_KEY_TAB => {
                self.focus = if self.editing {
                    match self.focus {
                        Focus::Name => Focus::Apply,
                        Focus::Apply => Focus::Cancel,
                        _ => Focus::Name,
                    }
                } else if self.trash_view {
                    match self.focus {
                        Focus::View => Focus::List,
                        Focus::List => Focus::Trash,
                        _ => Focus::View,
                    }
                } else {
                    match self.focus {
                        Focus::View => Focus::List,
                        Focus::List => Focus::New,
                        Focus::New => Focus::Rename,
                        Focus::Rename => Focus::Trash,
                        _ => Focus::View,
                    }
                };
            }
            libnagi::INPUT_KEY_ENTER => self.activate(self.focus),
            libnagi::INPUT_KEY_UP if !self.editing => {
                self.selected = self.selected.saturating_sub(1)
            }
            libnagi::INPUT_KEY_DOWN if !self.editing => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1))
            }
            libnagi::login::INPUT_KEY_BACKSPACE if self.focus == Focus::Name => {
                self.name.pop();
            }
            _ if self.focus == Focus::Name => {
                if let Some(byte) = libnagi::login::key_char(code) {
                    if self.name.len() < libnagi::storage::MAX_NAME_LENGTH {
                        self.name.push(char::from(byte));
                    }
                }
            }
            _ => {}
        }
    }

    pub fn completed(&mut self, result: Result<(), Error>) {
        self.refresh = true; // Also refresh after uncertain flush or stale selection.
        if result.is_ok() {
            self.cancel_edit();
        } else if !self.editing {
            self.focus = Focus::List;
        }
        self.status = match result {
            Ok(()) => "desktop.files.operation.saved",
            Err(Error::Conflict) => "desktop.files.operation.conflict",
            Err(Error::InvalidName) => "desktop.files.operation.invalid_name",
            Err(Error::StaleSelection) => "desktop.files.operation.stale",
            Err(Error::Capacity) => "desktop.files.operation.capacity",
            Err(Error::CorruptJournal) => "desktop.files.operation.journal",
            Err(Error::DurabilityUnknown) => "desktop.files.operation.uncertain",
            Err(Error::Storage) => "desktop.files.operation.failed",
        };
    }
}
