//! Files consumer that binds each service operation to a live C1 Session.
//!
//! The caller supplies the existing Files provider and capability authorizer.
//! This adapter never creates a capability grant. The C1 lease is an
//! admission boundary only; `FilesService` still asks the supplied
//! `CapabilityAuthorizer` for each Files right inside the admitted effect.
//! The supplied `Actor` is forwarded for Files operation records only; it
//! never creates or strengthens a capability.

use std::marker::PhantomData;

use nagi_files::localization::Locale;
use nagi_files::{
    Actor, CancellationToken, CapabilityAuthorizer, FileEntry, FileName, FilesCanvas,
    FilesCanvasRect, FilesComponent, FilesError, FilesService, FilesViewState, FilesystemProvider,
    Location, OperationResult, TrashEntry,
};
#[cfg(all(feature = "c1-issued-session", target_os = "linux"))]
use nagi_files::{
    BorrowedFilesRootScope, C1SessionBoundFilesRootScope, OwnerScopedSandboxProvider,
};
#[cfg(all(feature = "c1-issued-session", target_os = "linux"))]
use nagi_identity_session::{
    C1InitialInteractiveSessionError, C1IssuedInteractiveSession, Clock, GuestProfileCleaner,
    IdentityAuthorizer, IdentityEventSink, IdentityIdSource, IdentityService,
    IdentitySnapshotStore, InteractiveSessionError,
};
use nagi_identity_session::{ResolutionError, SessionLeaseRevalidationHandle};
#[cfg(all(feature = "c1-issued-session", target_os = "linux"))]
use nagi_profile_volume_notes_owner::{FilesRootScope, ProfileFilesRoot};

/// Failure from either the canonical C1 live-Session gate or FilesService.
#[derive(Debug)]
pub enum FilesSessionEffectError {
    Session(ResolutionError),
    Files(FilesError),
}

/// Failure while resolving a real C1 issued caller into this Files consumer.
#[cfg(all(feature = "c1-issued-session", target_os = "linux"))]
#[derive(Debug)]
pub enum FilesSessionConsumerBuildError {
    Provider(FilesError),
    IssuedSession(C1InitialInteractiveSessionError),
    Revalidation(InteractiveSessionError),
}

/// Owner-root wrapper used only by the issued-session screen constructor.
///
/// The constructed provider retains the matching C1 revalidation handle and
/// gates each complete FilesService operation before this scope callback.
#[cfg(all(feature = "c1-issued-session", target_os = "linux"))]
#[doc(hidden)]
pub struct C1IssuedOwnerRootScope<'root> {
    root: &'root ProfileFilesRoot,
    revalidation: SessionLeaseRevalidationHandle,
}

#[cfg(all(feature = "c1-issued-session", target_os = "linux"))]
impl FilesRootScope for C1IssuedOwnerRootScope<'_> {
    fn with_files_root<R>(
        &self,
        effect: impl for<'view> FnOnce(nagi_profile_volume_notes_owner::FilesRootView<'view>) -> R,
    ) -> Result<R, nagi_profile_volume_notes_owner::ProfileVolumeNotesOwnerError> {
        self.root.with_files_root(effect)
    }
}

#[cfg(all(feature = "c1-issued-session", target_os = "linux"))]
impl C1SessionBoundFilesRootScope for C1IssuedOwnerRootScope<'_> {
    fn c1_session_revalidation_handle(&self) -> &SessionLeaseRevalidationHandle {
        &self.revalidation
    }
}

/// A narrow Files consumer that retains its provider, capability policy, and
/// C1 revalidation handle and exposes only effects admitted by C1.
///
/// On Linux, construct this through [`Self::from_issued_session`] with the
/// actual C1-issued caller and its matching `IdentityService`. The consumer's
/// lifetime borrows that issued caller, while the opaque revalidation handle
/// is produced internally from its checked context. The caller must still bind
/// this consumer to the authenticated connection that supplied the C1
/// context; C1 identifiers alone do not authenticate a transport peer.
///
/// `authorizer` remains the sole Files capability decision source; Session
/// liveness does not grant Read, Enumerate, Create, or Move rights. Its
/// synchronous policy callback runs inside C1 admission and must not re-enter
/// C1 lifecycle or recovery APIs.
pub struct C1FilesSessionConsumer<'issued, P, A> {
    session: SessionLeaseRevalidationHandle,
    files: FilesService<P, A>,
    _issued_session: PhantomData<&'issued ()>,
}

impl<'issued, P, A> C1FilesSessionConsumer<'issued, P, A>
where
    P: FilesystemProvider,
    A: CapabilityAuthorizer,
{
    /// Resolve the actual C1-issued caller through its matching identity
    /// service, then retain the resulting live-session handle with the
    /// caller-supplied Files provider and capability authorizer.
    ///
    /// The factory accepts no public IDs or prebuilt handle. The shared borrow
    /// keeps `issued_session` alive for this consumer's lifetime; each effect
    /// still revalidates through C1. Build on Linux with this crate's
    /// `c1-issued-session` feature; it enables the canonical C1
    /// `linux-interactive-account` feature through Cargo.
    #[cfg(all(feature = "c1-issued-session", target_os = "linux"))]
    pub fn from_issued_session<S, I, C, IA, G, E>(
        provider: P,
        authorizer: A,
        issued_session: &'issued C1IssuedInteractiveSession,
        identity: &mut IdentityService<S, I, C, IA, G, E>,
    ) -> Result<Self, FilesSessionConsumerBuildError>
    where
        S: IdentitySnapshotStore,
        I: IdentityIdSource,
        C: Clock,
        IA: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        let context = issued_session
            .checked_context(identity)
            .map_err(FilesSessionConsumerBuildError::IssuedSession)?;
        let session = context
            .revalidation_handle()
            .map_err(FilesSessionConsumerBuildError::Revalidation)?;
        Ok(Self {
            session,
            files: FilesService::new(provider, authorizer),
            _issued_session: PhantomData,
        })
    }

    /// Enumerates a location after entering the C1 live-Session effect gate.
    pub fn list(&mut self, location: &Location) -> Result<Vec<FileEntry>, FilesSessionEffectError> {
        self.with_live_files_effect(|files| files.list(location))
    }

    /// Reads a selected file through the recorded Files open operation after
    /// entering the C1 live-Session effect gate.
    pub fn read(
        &mut self,
        source: &FileEntry,
        actor: Actor,
    ) -> Result<(Vec<u8>, OperationResult), FilesSessionEffectError> {
        self.with_live_files_effect(|files| files.open_file(source, actor))
    }

    /// Renames a selected resource through the existing FilesService policy
    /// and operation record while the C1 live-Session gate is held.
    pub fn rename(
        &mut self,
        source: &FileEntry,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesSessionEffectError> {
        self.with_live_files_effect(|files| files.rename(source, name, actor))
    }

    /// Saves edited contents through the existing FilesService Write policy
    /// and operation record while the C1 live-Session gate is held.
    pub fn save(
        &mut self,
        source: &FileEntry,
        contents: &[u8],
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesSessionEffectError> {
        self.with_live_files_effect(|files| files.write_file(source, contents, actor))
    }

    /// Copies through the existing FilesService and caller cancellation token
    /// while the C1 live-Session effect gate remains held.
    pub fn copy(
        &mut self,
        source: &FileEntry,
        destination: &Location,
        name: &FileName,
        actor: Actor,
        cancellation: &CancellationToken,
    ) -> Result<(FileEntry, OperationResult), FilesSessionEffectError> {
        self.with_live_files_effect(|files| {
            files.copy(source, destination, name, actor, cancellation)
        })
    }

    /// Moves through the existing FilesService while the C1 live-Session
    /// effect gate remains held.
    pub fn move_item(
        &mut self,
        source: &FileEntry,
        destination: &Location,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesSessionEffectError> {
        self.with_live_files_effect(|files| files.move_item(source, destination, name, actor))
    }

    /// Transfers this already-issued consumer into the existing stateful Files
    /// component so UI selection, rename, save, trash, restore, reopen, and
    /// drawing all use the same provider, authorizer, and service instance.
    pub fn into_screen(self, initial_location: Location) -> C1FilesScreenConsumer<'issued, P, A> {
        C1FilesScreenConsumer {
            session: Some(self.session),
            component: FilesComponent::new(self.files, initial_location),
            _issued_session: PhantomData,
        }
    }

    fn with_live_files_effect<R>(
        &mut self,
        effect: impl FnOnce(&mut FilesService<P, A>) -> Result<R, FilesError>,
    ) -> Result<R, FilesSessionEffectError> {
        let session = &self.session;
        let files = &mut self.files;
        session
            .with_live_session_effect(|| effect(files))
            .map_err(FilesSessionEffectError::Session)?
            .map_err(FilesSessionEffectError::Files)
    }
}

#[cfg(all(feature = "c1-issued-session", target_os = "linux"))]
impl<'issued, 'root, A>
    C1FilesScreenConsumer<'issued, OwnerScopedSandboxProvider<C1IssuedOwnerRootScope<'root>>, A>
where
    A: CapabilityAuthorizer,
{
    /// Builds the normal Files screen from the owner-issued root capability
    /// and matching live C1 caller. The provider stores only the owner scope
    /// and an opaque handle produced by that checked caller; it enters one C1
    /// effect per complete FilesService operation before borrowing one
    /// callback-scoped root view. Files rights remain with the supplied
    /// `CapabilityAuthorizer`.
    pub fn from_issued_owner_root<S, I, C, IA, G, E>(
        files_root: &'root ProfileFilesRoot,
        authorizer: A,
        issued_session: &'issued C1IssuedInteractiveSession,
        identity: &mut IdentityService<S, I, C, IA, G, E>,
        initial_location: Location,
    ) -> Result<Self, FilesSessionConsumerBuildError>
    where
        S: IdentitySnapshotStore,
        I: IdentityIdSource,
        C: Clock,
        IA: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        let context = issued_session
            .checked_context(identity)
            .map_err(FilesSessionConsumerBuildError::IssuedSession)?;
        let revalidation = context
            .revalidation_handle()
            .map_err(FilesSessionConsumerBuildError::Revalidation)?;
        let provider = OwnerScopedSandboxProvider::new(C1IssuedOwnerRootScope {
            root: files_root,
            revalidation,
        });
        Ok(C1FilesScreenConsumer {
            session: None,
            component: FilesComponent::new(
                FilesService::new(provider, authorizer),
                initial_location,
            ),
            _issued_session: PhantomData,
        })
    }
}

#[cfg(all(feature = "c1-issued-session", target_os = "linux"))]
impl<'root, S, A>
    C1FilesScreenConsumer<'root, OwnerScopedSandboxProvider<BorrowedFilesRootScope<'root, S>>, A>
where
    S: C1SessionBoundFilesRootScope,
    A: CapabilityAuthorizer,
{
    /// Builds the screen from one value that binds an owner root scope to the
    /// opaque live-session handle issued for that same root by the canonical
    /// C1 attachment path.
    ///
    /// The provider obtains the handle from this exact scope value and enters
    /// C1's real `with_live_session_effect` before each FilesService operation
    /// reaches the owner callback. A separate root and handle cannot be paired
    /// at this constructor. Files rights remain with `authorizer`.
    ///
    /// ```compile_fail
    /// use nagi_files::{CapabilityAuthorizer, Location};
    /// use nagi_files_c1_session_consumer::C1FilesScreenConsumer;
    /// use nagi_profile_volume_notes_owner::ProfileFilesRoot;
    ///
    /// fn owner_scope_alone_is_not_a_c1_gate<A: CapabilityAuthorizer>(
    ///     root: &ProfileFilesRoot,
    ///     authorizer: A,
    /// ) {
    ///     let _ = C1FilesScreenConsumer::from_owner_root_scope(
    ///         root,
    ///         authorizer,
    ///         Location::root(),
    ///     );
    /// }
    /// ```
    pub fn from_owner_root_scope(
        root_scope: &'root S,
        authorizer: A,
        initial_location: Location,
    ) -> Self {
        let provider = OwnerScopedSandboxProvider::new(BorrowedFilesRootScope::new(root_scope));
        Self {
            session: None,
            component: FilesComponent::new(
                FilesService::new(provider, authorizer),
                initial_location,
            ),
            _issued_session: PhantomData,
        }
    }
}

/// C1-gated adapter for the existing stateful Files screen component.
///
/// Filesystem operations are admitted by its live-Session handle or by the
/// C1-gated owner provider constructed with an opaque handle. Selection and
/// painting only touch in-memory view state and remain direct calls to
/// [`FilesComponent`].
pub struct C1FilesScreenConsumer<'issued, P, A> {
    session: Option<SessionLeaseRevalidationHandle>,
    component: FilesComponent<P, A>,
    _issued_session: PhantomData<&'issued ()>,
}

impl<'issued, P, A> C1FilesScreenConsumer<'issued, P, A>
where
    P: FilesystemProvider,
    A: CapabilityAuthorizer,
{
    /// Returns the same view model used for selection and drawing.
    pub fn state(&self) -> &FilesViewState {
        self.component.state()
    }

    /// Loads the visible listing inside the C1 live-Session effect gate.
    pub fn refresh(&mut self) -> Result<(), FilesSessionEffectError> {
        self.with_live_component_effect(FilesComponent::refresh)
    }

    /// Changes location and loads it inside the C1 live-Session effect gate.
    pub fn navigate(&mut self, location: Location) -> Result<(), FilesSessionEffectError> {
        self.with_live_component_effect(|component| component.navigate(location))
    }

    /// Creates a folder through the normal Files component inside the C1
    /// effect gate. The component refreshes the visible listing on success.
    pub fn create_folder(
        &mut self,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesSessionEffectError> {
        self.with_live_component_effect(|component| component.create_folder(name, actor))
    }

    /// Selection is an in-memory UI change and does not perform a Files effect.
    pub fn select(&mut self, id: nagi_files::ResourceId, additive: bool) -> Result<(), FilesError> {
        self.component.select(id, additive)
    }

    /// Routes the screen's focused rename through its existing selection and
    /// state-update logic, under the C1 live-Session gate.
    pub fn rename_selected(
        &mut self,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesSessionEffectError> {
        self.with_live_component_effect(|component| component.rename_selected(name, actor))
    }

    /// Routes the editor's selected-file save through the existing FilesService
    /// authorizer and view-state update, under the C1 live-Session gate.
    pub fn save_selected(
        &mut self,
        contents: &[u8],
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesSessionEffectError> {
        self.with_live_component_effect(|component| component.save_selected(contents, actor))
    }

    /// Moves the screen's focused selection through FilesService inside the
    /// C1 live-Session gate, preserving the component's normal view-state update.
    pub fn move_selected(
        &mut self,
        destination: &Location,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesSessionEffectError> {
        self.with_live_component_effect(|component| {
            component.move_selected(destination, name, actor)
        })
    }

    /// Reopens the focused screen selection through FilesService inside C1's
    /// live-Session gate.
    pub fn open_selected(
        &mut self,
        actor: Actor,
    ) -> Result<Option<(Vec<u8>, OperationResult)>, FilesSessionEffectError> {
        self.with_live_component_effect(|component| component.open_selected(actor))
    }

    /// Moves the focused screen selection to trash under the live-Session gate.
    pub fn trash_selected(
        &mut self,
        actor: Actor,
    ) -> Result<(TrashEntry, OperationResult), FilesSessionEffectError> {
        self.with_live_component_effect(|component| component.trash_selected(actor))
    }

    /// Lists restorable items through FilesService authorization inside the
    /// live-Session gate.
    pub fn list_trash(&mut self) -> Result<Vec<TrashEntry>, FilesSessionEffectError> {
        self.with_live_component_effect(|component| component.list_trash())
    }

    /// Restores one trash-list item under the live-Session gate, refreshing
    /// and selecting it in the visible screen when it returns to this folder.
    pub fn restore_trash_item(
        &mut self,
        trash_id: nagi_files::ResourceId,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesSessionEffectError> {
        self.with_live_component_effect(|component| component.restore_trash_item(trash_id, actor))
    }

    pub fn restore_trash_item_as(
        &mut self,
        trash_id: nagi_files::ResourceId,
        parent: &Location,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesSessionEffectError> {
        self.with_live_component_effect(|component| {
            component.restore_trash_item_as(trash_id, parent, name, actor)
        })
    }

    /// Paints the existing view model without performing a filesystem effect.
    pub fn draw(
        &self,
        canvas: &mut impl FilesCanvas,
        bounds: FilesCanvasRect,
        empty_message: &str,
        locale: Locale,
    ) {
        self.component.draw(canvas, bounds, empty_message, locale);
    }

    fn with_live_component_effect<R>(
        &mut self,
        effect: impl FnOnce(&mut FilesComponent<P, A>) -> Result<R, FilesError>,
    ) -> Result<R, FilesSessionEffectError> {
        let session = &self.session;
        let component = &mut self.component;
        match session {
            Some(session) => session
                .with_live_session_effect(|| effect(component))
                .map_err(FilesSessionEffectError::Session)?
                .map_err(FilesSessionEffectError::Files),
            None => effect(component).map_err(FilesSessionEffectError::Files),
        }
    }
}
