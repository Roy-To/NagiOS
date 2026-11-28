//! Product Calendar consumer for the snapshot already authorized by C1/C3.
//! Desktop owns launch authorization and supplies the live-session checks.

use crate::calendar_console::{self, CalendarExit};
use crate::calendar_session::CalendarSession;
use crate::profile_volume_notes_owner::AuthorizedProfileSnapshot;
use crate::ui_console::NagiConsole;
use nagi_localization::Locale;
use std::io;

// The L11 C2 Desktop consumer calls this entry once its release launcher is
// merged; until then the product API is intentionally compiled but unreferenced.
#[allow(dead_code)]
pub(crate) fn run<C: NagiConsole>(
    console: &mut C,
    snapshot: AuthorizedProfileSnapshot,
    locale: Locale,
    mut poll_identity: impl FnMut() -> io::Result<bool>,
    mut check_session: impl FnMut() -> Result<(), nagi_identity_session::ResolutionError>,
) -> io::Result<CalendarExit> {
    if poll_identity()? {
        return Ok(CalendarExit::RequiredIdentityExited);
    }
    if let Some(exit) = calendar_console::session_exit(check_session()) {
        return Ok(exit);
    }

    let clock = calendar_console::CalendarClock;
    let mut session = match CalendarSession::open(snapshot, clock) {
        Ok(session) => session,
        Err(_) => {
            return calendar_console::unavailable(console, locale, poll_identity, check_session)
        }
    };
    calendar_console::run(
        console,
        &mut session,
        &clock,
        locale,
        poll_identity,
        check_session,
        |_| {},
    )
}
