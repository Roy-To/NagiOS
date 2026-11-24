//! Reusable Files surface adapter that keeps provider results, selection, and
//! canvas rendering on one [`FilesApp`] view state.

#[cfg(unix)]
use crate::SandboxProvider;
use crate::{
    draw_three_pane,
    localization::{self, Locale},
    Actor, CancellationToken, CapabilityAuthorizer, EntryKind, FileEntry, FileName, FilesApp,
    FilesCanvas, FilesCanvasRect, FilesError, FilesErrorKind, FilesService, FilesViewState,
    FilesystemProvider, Location, OperationResult, TrashEntry,
};
use nagi_ui::{ColorRole, TypeRole};
#[cfg(unix)]
use std::fs::File;

/// A Files UI component backed by the service and authorization implementation
/// supplied by its caller. It does not create grants or choose a provider.
pub struct FilesComponent<P, A> {
    app: FilesApp,
    service: FilesService<P, A>,
    last_feedback_error: Option<FilesErrorKind>,
}
impl<P, A> FilesComponent<P, A>
where
    P: FilesystemProvider,
    A: CapabilityAuthorizer,
{
    /// Creates the component with a caller-configured service and initial
    /// location. Call [`refresh`](Self::refresh) to load the first listing.
    pub fn new(service: FilesService<P, A>, initial_location: Location) -> Self {
        Self {
            app: FilesApp::new(initial_location),
            service,
            last_feedback_error: None,
        }
    }

    /// Returns the view state used for both interaction and drawing.
    pub fn state(&self) -> &FilesViewState {
        &self.app.state
    }

    pub fn refresh(&mut self) -> Result<(), FilesError> {
        match self.app.refresh(&self.service) {
            Ok(()) => {
                self.last_feedback_error = None;
                Ok(())
            }
            Err(error) => {
                self.last_feedback_error = Some(error.kind);
                Err(error)
            }
        }
    }

    pub fn navigate(&mut self, location: Location) -> Result<(), FilesError> {
        self.last_feedback_error = None;
        self.app.navigate(&self.service, location)
    }

    pub fn navigate_parent(&mut self) -> Result<bool, FilesError> {
        self.last_feedback_error = None;
        self.app.navigate_parent(&self.service)
    }

    /// Creates a folder through FilesService. If creation succeeds but the
    /// following listing fails, the creation result is still returned and the
    /// refresh error remains visible so the caller can refetch without retrying.
    pub fn create_folder(
        &mut self,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.last_feedback_error = None;
        let (entry, operation) =
            match self
                .service
                .create_folder(&self.app.state.location, name, actor)
            {
                Ok(result) => result,
                Err(error) => {
                    self.last_feedback_error = Some(error.kind);
                    return Err(error);
                }
            };
        // Creation is already applied. Keep a follow-up list failure visible
        // on the view without reporting the mutation itself as failed.
        let _ = self.refresh();
        Ok((entry, operation))
    }

    /// Renames the focused resource through FilesService. If the provider
    /// mutation succeeds but the following listing refresh fails, the applied
    /// operation is still returned as successful and the refresh error remains
    /// visible on the screen. The caller can refresh again without repeating
    /// the rename.
    pub fn rename_selected(
        &mut self,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.last_feedback_error = None;
        let source = self
            .focused_selected_entry()?
            .ok_or_else(|| FilesError::new(crate::FilesErrorKind::NotFound))?;
        let (renamed, operation) = match self.service.rename(&source, name, actor) {
            Ok(result) => result,
            Err(error) => {
                self.last_feedback_error = Some(error.kind);
                return Err(error);
            }
        };
        if self.refresh().is_ok()
            && self
                .app
                .state
                .entries
                .iter()
                .any(|entry| entry.id == renamed.id)
        {
            self.app.state.select(renamed.id, false)?;
        }
        Ok((renamed, operation))
    }

    pub fn save_selected(
        &mut self,
        contents: &[u8],
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.last_feedback_error = None;
        let source = self
            .focused_selected_entry()?
            .ok_or_else(|| FilesError::new(crate::FilesErrorKind::NotFound))?;
        let (saved, operation) = match self.service.write_file(&source, contents, actor) {
            Ok(result) => result,
            Err(error) => {
                self.last_feedback_error = Some(error.kind);
                return Err(error);
            }
        };
        if let Some(entry) = self
            .app
            .state
            .entries
            .iter_mut()
            .find(|entry| entry.id == source.id)
        {
            *entry = saved.clone();
        }
        if self.app.state.selected.remove(&source.id) {
            self.app.state.selected.insert(saved.id);
        }
        if self.app.state.focused == Some(source.id) {
            self.app.state.focused = Some(saved.id);
        }
        Ok((saved, operation))
    }

    /// Copies the focused selected item through `FilesService`.
    ///
    /// When the destination is the visible folder, the copied entry is added
    /// to the same view state and becomes the focused Inspector item. A copy
    /// to another folder leaves the current selection intact.
    pub fn copy_selected(
        &mut self,
        destination: &Location,
        name: &FileName,
        actor: Actor,
        cancellation: &CancellationToken,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.last_feedback_error = None;
        let source = match self.focused_selected_entry().and_then(|source| {
            source.ok_or_else(|| FilesError::new(crate::FilesErrorKind::NotFound))
        }) {
            Ok(source) => source,
            Err(error) => {
                self.last_feedback_error = Some(error.kind);
                return Err(error);
            }
        };
        let (copied, operation) =
            match self
                .service
                .copy(&source, destination, name, actor, cancellation)
            {
                Ok(result) => result,
                Err(error) => {
                    self.last_feedback_error = Some(error.kind);
                    return Err(error);
                }
            };

        if destination == &self.app.state.location {
            upsert_entry(&mut self.app.state.entries, copied.clone());
            self.app.state.selected.insert(copied.id);
            self.app.state.focused = Some(copied.id);
        }
        Ok((copied, operation))
    }

    /// Moves the focused selected item through `FilesService`.
    ///
    /// Moving out of the visible folder removes the entry and its selection
    /// from that folder. Moving within it replaces the row and keeps the
    /// moved item focused so the Inspector follows the returned service entry.
    pub fn move_selected(
        &mut self,
        destination: &Location,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.last_feedback_error = None;
        let source = match self.focused_selected_entry().and_then(|source| {
            source.ok_or_else(|| FilesError::new(crate::FilesErrorKind::NotFound))
        }) {
            Ok(source) => source,
            Err(error) => {
                self.last_feedback_error = Some(error.kind);
                return Err(error);
            }
        };
        let (moved, operation) = match self.service.move_item(&source, destination, name, actor) {
            Ok(result) => result,
            Err(error) => {
                self.last_feedback_error = Some(error.kind);
                return Err(error);
            }
        };

        if source.location == self.app.state.location {
            self.app.state.entries.retain(|entry| entry.id != source.id);
            if destination == &self.app.state.location {
                upsert_entry(&mut self.app.state.entries, moved.clone());
                if self.app.state.selected.remove(&source.id) {
                    self.app.state.selected.insert(moved.id);
                }
                if self.app.state.focused == Some(source.id) {
                    self.app.state.focused = Some(moved.id);
                }
            } else {
                self.app.state.selected.remove(&source.id);
                if self.app.state.focused == Some(source.id) {
                    self.app.state.focused = self
                        .app
                        .state
                        .selected
                        .iter()
                        .copied()
                        .find(|id| self.app.state.entries.iter().any(|entry| entry.id == *id));
                }
            }
        }
        Ok((moved, operation))
    }

    /// Moves the focused selected item to the provider's trash and refreshes
    /// the visible listing so the removed resource and its selection disappear.
    /// A refresh failure after successful trash stays visible on the screen,
    /// while the applied trash operation is returned as successful so the
    /// caller can refetch without trying the mutation again.
    pub fn trash_selected(
        &mut self,
        actor: Actor,
    ) -> Result<(TrashEntry, OperationResult), FilesError> {
        self.last_feedback_error = None;
        let source = match self.focused_selected_entry().and_then(|source| {
            source.ok_or_else(|| FilesError::new(crate::FilesErrorKind::NotFound))
        }) {
            Ok(source) => source,
            Err(error) => {
                self.last_feedback_error = Some(error.kind);
                return Err(error);
            }
        };
        let (trashed, operation) = match self.service.trash(&source, actor) {
            Ok(result) => result,
            Err(error) => {
                self.last_feedback_error = Some(error.kind);
                return Err(error);
            }
        };
        let _ = self.refresh();
        Ok((trashed, operation))
    }

    /// Lists restorable provider trash entries through FilesService's
    /// Enumerate authorization. The screen can use the returned resource IDs
    /// to offer a restore action without exposing permanent deletion here.
    pub fn list_trash(&self) -> Result<Vec<TrashEntry>, FilesError> {
        self.service.list_trash()
    }

    /// Restores one item through FilesService's Restore authorization. If the
    /// item returns to the visible location, refreshes and selects it so the
    /// row and Inspector follow the provider result. A refresh failure after a
    /// successful restore stays visible on the screen while the restore result
    /// is still returned as successful; the caller can refresh again without
    /// restoring the item a second time.
    pub fn restore_trash_item(
        &mut self,
        trash_id: crate::ResourceId,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        let result = self.service.restore(trash_id, actor);
        self.complete_restore(result)
    }

    pub fn restore_trash_item_as(
        &mut self,
        trash_id: crate::ResourceId,
        parent: &Location,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        let result = self.service.restore_as(trash_id, parent, name, actor);
        self.complete_restore(result)
    }

    fn complete_restore(
        &mut self,
        result: Result<(FileEntry, OperationResult), FilesError>,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.last_feedback_error = None;
        let (restored, operation) = match result {
            Ok(result) => result,
            Err(error) => {
                self.last_feedback_error = Some(error.kind);
                return Err(error);
            }
        };
        if restored.location == self.app.state.location {
            if self.refresh().is_ok()
                && self
                    .app
                    .state
                    .entries
                    .iter()
                    .any(|entry| entry.id == restored.id)
            {
                self.app.state.select(restored.id, false)?;
            }
        }
        Ok((restored, operation))
    }

    pub fn select(&mut self, id: crate::ResourceId, additive: bool) -> Result<(), FilesError> {
        self.last_feedback_error = None;
        self.app.state.select(id, additive)
    }

    pub fn clear_selection(&mut self) {
        self.last_feedback_error = None;
        self.app.state.clear_selection();
    }

    /// Opens the focused selected item through `FilesService`, returning the
    /// service's bounded contents and operation record. With no selection it
    /// returns `Ok(None)`.
    pub fn open_selected(
        &mut self,
        actor: Actor,
    ) -> Result<Option<(Vec<u8>, OperationResult)>, FilesError> {
        let Some(source) = self.focused_selected_entry()? else {
            return Ok(None);
        };
        match self.service.open_file(&source, actor) {
            Ok(opened) => {
                self.last_feedback_error = None;
                Ok(Some(opened))
            }
            Err(error) => {
                self.last_feedback_error = Some(error.kind);
                Err(error)
            }
        }
    }

    /// Draws the current service-backed view through the existing Files canvas
    /// contract. The layout is derived from the same state returned by
    /// [`state`](Self::state).
    pub fn draw(
        &self,
        canvas: &mut impl FilesCanvas,
        bounds: FilesCanvasRect,
        empty_message: &str,
        locale: Locale,
    ) {
        let status_height = if self.last_feedback_error.is_some() && bounds.height >= 24 {
            22
        } else {
            0
        };
        let content_bounds = FilesCanvasRect::new(
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height - status_height,
        );
        draw_three_pane(
            canvas,
            content_bounds,
            &self.app.state.layout(),
            empty_message,
            locale,
        );
        if let Some(error) = self.last_feedback_error.filter(|_| status_height > 0) {
            let message = localization::error_message(locale, &FilesError::new(error));
            canvas.text(
                FilesCanvasRect::new(
                    bounds.x + 5,
                    bounds.y + bounds.height - 20,
                    (bounds.width - 10).max(0),
                    18,
                ),
                &message,
                TypeRole::Caption,
                ColorRole::TextPrimary,
            );
        }
    }

    fn focused_selected_entry(&self) -> Result<Option<FileEntry>, FilesError> {
        let Some(id) = self
            .app
            .state
            .focused
            .filter(|id| self.app.state.selected.contains(id))
        else {
            return Ok(None);
        };
        self.app
            .state
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .map(Some)
            .ok_or_else(|| FilesError::new(crate::FilesErrorKind::NotFound))
    }
}

#[cfg(unix)]
impl<A> FilesComponent<SandboxProvider, A>
where
    A: CapabilityAuthorizer,
{
    /// Builds the existing Files view from a caller-issued User/Profile Files
    /// root FD and the caller's authorizer. This consumes the FD through
    /// `SandboxProvider` and creates no grant or session; callers remain
    /// responsible for the live C1 effect gate and authorizer lifecycle.
    pub fn from_authorized_root(
        root: File,
        authorizer: A,
        initial_location: Location,
    ) -> Result<Self, FilesError> {
        let provider = SandboxProvider::from_authorized_root(root)?;
        Ok(Self::new(
            FilesService::new(provider, authorizer),
            initial_location,
        ))
    }
}

fn upsert_entry(entries: &mut Vec<FileEntry>, entry: FileEntry) {
    if let Some(existing) = entries.iter_mut().find(|existing| existing.id == entry.id) {
        *existing = entry;
    } else {
        entries.push(entry);
    }
    entries.sort_by(|left, right| {
        (left.kind != EntryKind::Folder)
            .cmp(&(right.kind != EntryKind::Folder))
            .then_with(|| {
                left.name
                    .as_str()
                    .to_lowercase()
                    .cmp(&right.name.as_str().to_lowercase())
            })
    });
}
