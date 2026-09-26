use crate::{CancellationToken, FileEntry, FileName, FilesError, Location, ResourceId, TrashEntry};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderAvailability {
    Available,
    Unavailable(String),
}

/// Public adapter boundary for Nagi storage, a test backend, or a host preview.
/// Implementations must treat `Location` values as provider-relative names.
pub trait FilesystemProvider {
    fn availability(&self) -> ProviderAvailability;
    fn list(&self, location: &Location) -> Result<Vec<FileEntry>, FilesError>;
    fn metadata(&self, location: &Location) -> Result<FileEntry, FilesError>;
    fn verify_resource(
        &self,
        id: ResourceId,
        location: &Location,
    ) -> Result<FileEntry, FilesError> {
        let entry = self.metadata(location)?;
        if entry.id != id {
            return Err(FilesError::new(crate::FilesErrorKind::Conflict).at(location.clone()));
        }
        Ok(entry)
    }
    /// Read at most `max_bytes`; providers must detect one byte beyond the
    /// limit and return `FileTooLarge` without buffering the complete file.
    fn read_file(&self, location: &Location, max_bytes: usize) -> Result<Vec<u8>, FilesError>;
    fn set_tags(&mut self, location: &Location, tags: &[String]) -> Result<FileEntry, FilesError> {
        let _ = tags;
        Err(FilesError::new(crate::FilesErrorKind::ProviderUnavailable).at(location.clone()))
    }
    fn create_folder(
        &mut self,
        parent: &Location,
        name: &FileName,
    ) -> Result<FileEntry, FilesError>;
    fn rename(&mut self, source: &Location, name: &FileName) -> Result<FileEntry, FilesError>;
    fn copy(
        &mut self,
        source: &Location,
        destination: &Location,
        name: &FileName,
        cancellation: &CancellationToken,
    ) -> Result<FileEntry, FilesError>;
    fn move_item(
        &mut self,
        source: &Location,
        destination: &Location,
        name: &FileName,
    ) -> Result<FileEntry, FilesError>;
    fn trash(&mut self, source: &Location) -> Result<TrashEntry, FilesError>;
    fn list_trash(&self) -> Result<Vec<TrashEntry>, FilesError>;
    fn restore(&mut self, id: ResourceId) -> Result<FileEntry, FilesError>;
    fn permanently_delete(&mut self, id: ResourceId) -> Result<(), FilesError>;
}
