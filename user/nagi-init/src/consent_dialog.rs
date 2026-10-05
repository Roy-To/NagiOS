//! OS-owned consent dialog (ADR 0060).
//!
//! The desktop shows this modal when the Supervisor has a pending
//! `ConsentRequest`. It is drawn by init, takes every key and button event
//! while open, and answers through `libnagi::consent::ConsentPrompt`. The
//! answer is recorded by the Supervisor with the signed-in user's session;
//! `Allow` and `Deny` are written to User Data so they survive a restart.
//!
//! The launched application has no route to any of this: it cannot draw the
//! dialog, read its state, or supply the answer.

use libnagi::consent::ConsentPrompt;
pub use libnagi::consent::{ConsentAnswer, ConsentChoice, PromptOutcome};
use libnagi::launch::{ConsentRequest, DecisionStoreError, GrantCheck};
use libnagi::security::Session;
use libnagi::storage::{StorageError, SyscallBlockDevice, Vfs, MAX_SMALL_FILE_SIZE};
use libnagi::InputEvent;
use nagi_localization::Locale;
use nagi_ui::{color, ColorRole, ThemeMode};

use crate::supervisor;
use crate::ui::{Painter, Rect};

type UserDataVolume = Vfs<SyscallBlockDevice>;

/// User Data file holding the persisted `Allow`/`Deny` decisions.
const DECISIONS_PATH: &[u8] = b"consent-decisions";

const THEME: ThemeMode = ThemeMode::Light;
const DIALOG: Rect = Rect::new(30, 46, 260, 108);
const TITLE_HEIGHT: i32 = 16;
const BUTTON_Y: i32 = DIALOG.y + 82;
const BUTTON_WIDTH: i32 = 80;
const BUTTON_HEIGHT: i32 = 18;
const BUTTONS: [(ConsentChoice, Rect, &str); 3] = [
    (
        ConsentChoice::Deny,
        Rect::new(DIALOG.x + 8, BUTTON_Y, BUTTON_WIDTH, BUTTON_HEIGHT),
        "consent.dialog.deny",
    ),
    (
        ConsentChoice::AllowOnce,
        Rect::new(DIALOG.x + 90, BUTTON_Y, BUTTON_WIDTH, BUTTON_HEIGHT),
        "consent.dialog.allow_once",
    ),
    (
        ConsentChoice::Allow,
        Rect::new(DIALOG.x + 172, BUTTON_Y, BUTTON_WIDTH, BUTTON_HEIGHT),
        "consent.dialog.allow",
    ),
];

pub struct ConsentDialog {
    request: ConsentRequest,
    prompt: ConsentPrompt,
}

impl ConsentDialog {
    pub const fn new(request: ConsentRequest) -> Self {
        Self {
            request,
            prompt: ConsentPrompt::new(),
        }
    }

    /// Accept input; call after the frame showing the dialog is presented.
    pub fn arm(&mut self) {
        self.prompt.arm();
    }

    pub const fn is_armed(&self) -> bool {
        self.prompt.is_armed()
    }

    pub fn render(&self, painter: &mut Painter<'_>, locale: Locale) {
        let surface = color(THEME, ColorRole::Surface).to_pixel();
        let text = color(THEME, ColorRole::TextPrimary).to_pixel();
        let accent = color(THEME, ColorRole::Accent).to_pixel();
        let border = color(THEME, ColorRole::BorderStrong).to_pixel();
        let focus = color(THEME, ColorRole::Focus).to_pixel();
        painter.fill(DIALOG, surface);
        painter.frame(DIALOG, border);
        painter.fill(
            Rect::new(DIALOG.x + 1, DIALOG.y + 1, DIALOG.width - 2, TITLE_HEIGHT),
            accent,
        );
        painter.text(
            DIALOG.x + 8,
            DIALOG.y + 5,
            nagi_localization::text(locale, "consent.dialog.title").as_bytes(),
            color(THEME, ColorRole::TextOnAccent).to_pixel(),
        );
        painter.text(
            DIALOG.x + 10,
            DIALOG.y + 24,
            nagi_localization::text(locale, "consent.dialog.app").as_bytes(),
            text,
        );
        painter.text(
            DIALOG.x + 10,
            DIALOG.y + 35,
            self.request.identifier(),
            text,
        );
        painter.text(
            DIALOG.x + 10,
            DIALOG.y + 51,
            nagi_localization::text(locale, "consent.dialog.access").as_bytes(),
            text,
        );
        painter.text(
            DIALOG.x + 10,
            DIALOG.y + 62,
            self.request.capability(),
            text,
        );
        for (choice, rect, key) in BUTTONS {
            let pressed = self.prompt.is_pressed(choice);
            painter.fill(
                rect,
                if pressed {
                    accent
                } else {
                    color(THEME, ColorRole::SurfaceRaised).to_pixel()
                },
            );
            painter.frame(
                rect,
                if self.prompt.focus() == choice {
                    focus
                } else {
                    border
                },
            );
            if self.prompt.focus() == choice {
                painter.frame(
                    Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2),
                    focus,
                );
            }
            painter.text(
                rect.x + 5,
                rect.y + 6,
                nagi_localization::text(locale, key).as_bytes(),
                text,
            );
        }
    }

    /// Route a key or button event (press or release) to the prompt. The
    /// dialog is modal: the caller must not pass these events elsewhere.
    pub fn handle_event(
        &mut self,
        event: InputEvent,
        pointer_x: i32,
        pointer_y: i32,
    ) -> PromptOutcome {
        if event.event_type != libnagi::INPUT_EVENT_KEY {
            return PromptOutcome::Ignored;
        }
        let pressed = event.value != 0;
        if event.code == libnagi::INPUT_BUTTON_PRIMARY {
            let target = BUTTONS
                .iter()
                .find(|(_, rect, _)| rect.contains(pointer_x, pointer_y))
                .map(|(choice, _, _)| *choice);
            self.prompt.handle_pointer(event.code, pressed, target)
        } else {
            self.prompt.handle_key(event.code, pressed)
        }
    }

    pub const fn request(&self) -> ConsentRequest {
        self.request
    }
}

/// Record `answer` for `request` with `user` and persist the decisions that
/// outlive the session. Returns the resulting grant check.
pub fn resolve(
    volume: &mut UserDataVolume,
    user: &Session,
    request: &ConsentRequest,
    answer: ConsentAnswer,
) -> Option<GrantCheck> {
    supervisor::resolve_consent(user, request, answer.decision(request.app_session_id)).ok()?;
    if matches!(
        answer,
        ConsentAnswer::Chosen(ConsentChoice::Allow | ConsentChoice::Deny)
    ) && !persist_decisions(volume)
    {
        return None;
    }
    Some(supervisor::check_grant(
        request.app_id,
        request.app_session_id,
        request.capability(),
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoredDecisions {
    None,
    Restored(usize),
    /// The file was unreadable or failed validation; nothing was applied.
    Invalid,
}

/// Apply persisted decisions for `user` from User Data.
pub fn restore_decisions(volume: &mut UserDataVolume, user: &Session) -> RestoredDecisions {
    let handle = match volume.open_path(DECISIONS_PATH) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) => return RestoredDecisions::None,
        Err(_) => return RestoredDecisions::Invalid,
    };
    let mut contents = [0; MAX_SMALL_FILE_SIZE];
    let Ok(length) = volume.read(handle, &mut contents) else {
        return RestoredDecisions::Invalid;
    };
    match supervisor::restore_decisions(user, &contents[..length]) {
        Ok(count) => RestoredDecisions::Restored(count),
        Err(DecisionStoreError::Malformed | DecisionStoreError::Launch(_)) => {
            RestoredDecisions::Invalid
        }
    }
}

fn persist_decisions(volume: &mut UserDataVolume) -> bool {
    let mut encoded = [0; libnagi::launch::MAX_ENCODED_DECISIONS];
    let length = supervisor::encode_decisions(&mut encoded);
    let handle = match volume.open_path(DECISIONS_PATH) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) => match volume.create_path(DECISIONS_PATH) {
            Ok(handle) => handle,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    volume
        .write(handle, &encoded[..length])
        .and_then(|()| volume.flush())
        .is_ok()
}

/// The ADR 0060 acceptance: a signed application whose manifest requests a
/// capability is launched; the desktop must ask the user before the grant
/// is effective, and a persisted answer must hold after a restart.
#[cfg(feature = "consent-dialog-acceptance")]
pub mod acceptance {
    use super::*;
    use libnagi::launch::LaunchPlacement;
    use nagi_model::{AppId, AppSessionId, NodeId};

    static FAULTING_APP_PACKAGE: &[u8] = crate::acceptance_package!("faulting-app");
    const FAULTING_APP: AppId = AppId::from_identifier(b"org.nagi.acceptance.faulting-app");
    /// Requested by the faulting app's signed manifest (ADR 0051).
    pub const CAPABILITY: &[u8] = b"acceptance.consent-probe";
    const PLACEMENT: LaunchPlacement = LaunchPlacement {
        app_session_id: AppSessionId(0x4e41_4749_0053_0001),
        node_id: NodeId(0x4e41_4749_0053_0002),
        workspace_id: None,
    };

    pub enum Start {
        /// The grant needs the user's answer; show the dialog.
        Prompt(ConsentDialog),
        /// A persisted decision already answers it.
        Decided(GrantCheck),
        Failed(&'static [u8]),
    }

    /// Restore persisted decisions, launch the signed application, and ask
    /// the Supervisor whether its requested capability needs a prompt.
    pub fn start(volume: &mut UserDataVolume, user: &Session) -> (RestoredDecisions, Start) {
        let restored = restore_decisions(volume, user);
        // The launched process waits for a fault command that never comes;
        // it stays live for the rest of the acceptance boot.
        let launched = match supervisor::launch(FAULTING_APP_PACKAGE, FAULTING_APP, PLACEMENT) {
            Ok(launched) => launched,
            Err(_) => return (restored, Start::Failed(b"signed launch")),
        };
        let record = launched.record;
        core::mem::forget(launched);
        let start =
            match supervisor::request_consent(record.app_id, record.app_session_id, CAPABILITY) {
                Ok(request) if supervisor::next_consent_request() == Some(request) => {
                    Start::Prompt(ConsentDialog::new(request))
                }
                Ok(_) => Start::Failed(b"prompt queue order"),
                Err(check @ (GrantCheck::Granted | GrantCheck::Denied)) => Start::Decided(check),
                Err(_) => Start::Failed(b"grant state"),
            };
        (restored, start)
    }
}
