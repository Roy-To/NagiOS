//! E-owned consumer for the canonical A Notes profile-root API.
//!
//! The production factory keeps its real C1 authority inputs. The separate
//! fixture test exercises the normal bridge flow with an explicitly volatile
//! in-memory store; it does not claim authenticated or durable behavior.

mod consumer {
    use nagi_abi::InputEvent;
    use notes_event_bridge_candidate::{NotesEventBridge, ScopedNotesEventBridge};
    use notes_files_codec_candidate::{AppError, NoteDocument, ObjectId};

    #[cfg(target_os = "linux")]
    use nagi_identity_session::{
        AuthenticatedInteractiveCaller, ProfileRootAuthorizer, ResolvedIdentity,
    };
    #[cfg(target_os = "linux")]
    use notes_files_codec_candidate::{ProfileRootNotesInputError, ProfileVolumeNotesBundle};
    #[cfg(target_os = "linux")]
    use std::thread::Scope;
    #[cfg(target_os = "linux")]
    use std::time::Duration;

    /// Consume A's post-login profile-volume bundle with the live C1 identity,
    /// C1-issued caller from the C2-authenticated login path, and trusted root
    /// authorizer supplied by the normal launch path. A's factory validates
    /// their binding and holds the single C1 root-effect lease for each Notes
    /// operation. This adapter does not issue the bundle, select a volume,
    /// mint a caller, or choose an ID.
    #[cfg(target_os = "linux")]
    pub fn construct_notes_bridge<C, A>(
        bundle: ProfileVolumeNotesBundle,
        identity: ResolvedIdentity,
        caller: C,
        authorizer: A,
        io_budget: Duration,
    ) -> Result<NotesEventBridge, ProfileRootNotesInputError>
    where
        C: AuthenticatedInteractiveCaller + Send + 'static,
        A: ProfileRootAuthorizer + Send + 'static,
    {
        NotesEventBridge::new_with_linux_profile_root(
            bundle, identity, caller, authorizer, io_budget,
        )
    }

    /// Build the same production bridge while retaining a borrowed C1 caller
    /// only for the surrounding Session scope. Call this inside
    /// `std::thread::scope` so the bridge and any scoped Notes workers are
    /// dropped before the issued Session can end.
    #[cfg(target_os = "linux")]
    pub fn construct_notes_bridge_scoped<'scope, 'env, C, A>(
        scope: &'scope Scope<'scope, 'env>,
        bundle: ProfileVolumeNotesBundle,
        identity: ResolvedIdentity,
        caller: C,
        authorizer: A,
        io_budget: Duration,
    ) -> Result<ScopedNotesEventBridge<'scope>, ProfileRootNotesInputError>
    where
        'env: 'scope,
        C: AuthenticatedInteractiveCaller + Send + 'scope,
        A: ProfileRootAuthorizer + Send + 'scope,
    {
        ScopedNotesEventBridge::new_with_linux_profile_root_scoped(
            scope, bundle, identity, caller, authorizer, io_budget,
        )
    }

    /// Pass existing UI save and close events through the canonical bridge,
    /// then reopen the same persisted note and return all three snapshots.
    /// The caller supplies its actual input events; no events are fabricated here.
    pub fn save_close_reopen(
        bridge: &mut ScopedNotesEventBridge<'_>,
        save_event: InputEvent,
        close_event: InputEvent,
        note_id: ObjectId,
    ) -> Result<(Option<NoteDocument>, Option<NoteDocument>, NoteDocument), AppError> {
        let saved = bridge.handle_event(save_event)?;
        let closed = bridge.handle_event(close_event)?;
        let reopened = bridge.open_note(note_id)?;
        Ok((saved, closed, reopened))
    }
}

#[cfg(target_os = "linux")]
pub use consumer::construct_notes_bridge;
#[cfg(target_os = "linux")]
pub use consumer::construct_notes_bridge_scoped;
pub use consumer::save_close_reopen;
