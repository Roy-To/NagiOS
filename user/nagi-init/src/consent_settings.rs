//! OS-owned permissions view in Settings (ADR 0065).
//!
//! Lists the signed-in owner's recorded consent decisions and lets the
//! owner withdraw one. Withdrawing records `Ask` for that application and
//! capability, so the next use prompts again, and rewrites the persisted
//! decisions immediately. The view is drawn by init; launched applications
//! cannot read or change it.

use libnagi::launch::{DecisionView, GrantDecision, MAX_CONSENT_DECISIONS};
use libnagi::security::Session;
use libnagi::storage::{SyscallBlockDevice, Vfs};
use libnagi::InputEvent;
use nagi_localization::Locale;
use nagi_ui::{color, ColorRole, ThemeMode};

use crate::supervisor;
use crate::ui::{Painter, Rect};

type UserDataVolume = Vfs<SyscallBlockDevice>;

const THEME: ThemeMode = ThemeMode::Light;
const PANEL: Rect = Rect::new(20, 30, 280, 140);
const ROW_HEIGHT: i32 = 22;
const VISIBLE_ROWS: usize = 5;

pub enum PermissionsOutcome {
    Ignored,
    Changed,
    /// A decision was withdrawn and persisted.
    Withdrawn,
    Closed,
}

pub struct PermissionsPanel {
    entries: [Option<DecisionView>; MAX_CONSENT_DECISIONS],
    count: usize,
    focus: usize,
    announced: bool,
}

impl PermissionsPanel {
    pub fn open() -> Self {
        let mut entries = [None; MAX_CONSENT_DECISIONS];
        let count = supervisor::list_decisions(&mut entries);
        Self {
            entries,
            count,
            focus: 0,
            announced: false,
        }
    }

    /// Report the view once its first frame is on screen.
    pub fn announce(&mut self) {
        if self.announced {
            return;
        }
        self.announced = true;
        libnagi::console_write(b"Nagi consent settings OPEN decisions=");
        libnagi::console_write(&[b'0' + (self.count.min(9) as u8)]);
        libnagi::console_write(b"\r\n");
    }

    pub fn render(&self, painter: &mut Painter<'_>, locale: Locale) {
        let text = color(THEME, ColorRole::TextPrimary).to_pixel();
        let border = color(THEME, ColorRole::BorderStrong).to_pixel();
        let focus = color(THEME, ColorRole::Focus).to_pixel();
        painter.fill(PANEL, color(THEME, ColorRole::Surface).to_pixel());
        painter.frame(PANEL, border);
        painter.fill(
            Rect::new(PANEL.x + 1, PANEL.y + 1, PANEL.width - 2, 16),
            color(THEME, ColorRole::Accent).to_pixel(),
        );
        painter.text(
            PANEL.x + 8,
            PANEL.y + 5,
            nagi_localization::text(locale, "settings.permissions.title").as_bytes(),
            color(THEME, ColorRole::TextOnAccent).to_pixel(),
        );
        let first = self.focus.saturating_sub(VISIBLE_ROWS - 1);
        for (row, index) in (first..self.count).take(VISIBLE_ROWS).enumerate() {
            let Some(entry) = &self.entries[index] else {
                continue;
            };
            let rect = Rect::new(
                PANEL.x + 8,
                PANEL.y + 22 + row as i32 * ROW_HEIGHT,
                PANEL.width - 16,
                ROW_HEIGHT - 2,
            );
            painter.fill(rect, color(THEME, ColorRole::SurfaceRaised).to_pixel());
            painter.frame(rect, if index == self.focus { focus } else { border });
            let mut hex = [0u8; 16];
            let name = match entry.identifier() {
                Some(identifier) => identifier,
                None => {
                    for (digit, shift) in hex.iter_mut().zip((0..16).rev()) {
                        *digit =
                            b"0123456789abcdef"[((entry.app_id.0 >> (shift * 4)) & 0xf) as usize];
                    }
                    &hex
                }
            };
            // The application name, then capability and decision.
            painter.text(rect.x + 4, rect.y + 2, &name[..name.len().min(36)], text);
            let end = painter.text(
                rect.x + 4,
                rect.y + 11,
                &entry.capability()[..entry.capability().len().min(24)],
                text,
            );
            let decision = match entry.decision {
                GrantDecision::Allow => "consent.dialog.allow",
                GrantDecision::Deny => "consent.dialog.deny",
                GrantDecision::AllowOnce(_) | GrantDecision::Ask => "consent.dialog.allow_once",
            };
            painter.text(
                end + 8,
                rect.y + 11,
                nagi_localization::text(locale, decision).as_bytes(),
                text,
            );
        }
    }

    /// Keys while the panel is open: Up/Down/Tab move, Enter or Space
    /// withdraws the focused decision, Escape closes.
    pub fn handle_event(
        &mut self,
        event: InputEvent,
        user: Option<&Session>,
        volume: &mut UserDataVolume,
    ) -> PermissionsOutcome {
        if event.event_type != libnagi::INPUT_EVENT_KEY || event.value == 0 {
            return PermissionsOutcome::Ignored;
        }
        match event.code {
            libnagi::INPUT_KEY_ESCAPE => PermissionsOutcome::Closed,
            libnagi::INPUT_KEY_DOWN | libnagi::INPUT_KEY_TAB if self.count > 0 => {
                self.focus = (self.focus + 1) % self.count;
                PermissionsOutcome::Changed
            }
            libnagi::INPUT_KEY_UP if self.count > 0 => {
                self.focus = (self.focus + self.count - 1) % self.count;
                PermissionsOutcome::Changed
            }
            libnagi::INPUT_KEY_ENTER | libnagi::INPUT_KEY_SPACE => self.withdraw(user, volume),
            _ => PermissionsOutcome::Ignored,
        }
    }

    fn withdraw(
        &mut self,
        user: Option<&Session>,
        volume: &mut UserDataVolume,
    ) -> PermissionsOutcome {
        let (Some(user), Some(entry)) = (user, self.entries.get(self.focus).copied().flatten())
        else {
            return PermissionsOutcome::Ignored;
        };
        let recorded = supervisor::record_user_decision(
            user,
            entry.app_id,
            entry.capability(),
            GrantDecision::Ask,
        )
        .is_ok();
        if !recorded || !crate::consent_dialog::persist_decisions(volume) {
            libnagi::console_write(b"Nagi consent settings withdraw FAIL\r\n");
            return PermissionsOutcome::Changed;
        }
        libnagi::console_write(b"Nagi consent decision withdrawn PASS capability=");
        libnagi::console_write(entry.capability());
        libnagi::console_write(b"\r\n");
        self.count = supervisor::list_decisions(&mut self.entries);
        self.focus = self.focus.min(self.count.saturating_sub(1));
        PermissionsOutcome::Withdrawn
    }
}
