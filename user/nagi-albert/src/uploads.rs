//! User-selected, read-only file upload handoff; browser content receives no path access.

use crate::tabs::TabId;

pub const MAX_UPLOAD_FILES: usize = 32;
pub const MAX_UPLOAD_SESSIONS: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct UploadRequestId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct UploadTicket(pub u64);

/// Opaque object identity returned by the trusted file picker adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ReadOnlyFileHandle(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadSelectionRequest {
    pub id: UploadRequestId,
    pub tab_id: TabId,
    pub origin: String,
    pub accept: Vec<String>,
    pub max_files: usize,
    pub multiple: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedUploadFile {
    pub handle: ReadOnlyFileHandle,
    pub display_name: String,
    pub byte_length: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadPhase {
    AwaitingSelection,
    Selected,
    Submitted,
    Canceled,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadSession {
    pub request: UploadSelectionRequest,
    pub selected: Vec<SelectedUploadFile>,
    pub phase: UploadPhase,
    pub ticket: Option<UploadTicket>,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UploadError {
    CapacityReached,
    InvalidRequest,
    TooManyFiles,
    SelectionDenied,
    RuntimeRejected,
    RequestNotFound,
    InvalidPhase,
    IdsExhausted,
}

pub trait UploadRuntime {
    fn choose_files(
        &mut self,
        request: &UploadSelectionRequest,
    ) -> Result<Vec<SelectedUploadFile>, UploadError>;

    fn submit(
        &mut self,
        origin: &str,
        files: &[SelectedUploadFile],
    ) -> Result<UploadTicket, UploadError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadsState {
    sessions: Vec<UploadSession>,
    next_id: u64,
}

impl UploadsState {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            next_id: 1,
        }
    }

    pub fn sessions(&self) -> &[UploadSession] {
        &self.sessions
    }

    pub fn request_selection(
        &mut self,
        runtime: &mut impl UploadRuntime,
        tab_id: TabId,
        origin: &str,
        accept: Vec<String>,
        multiple: bool,
    ) -> Result<UploadRequestId, UploadError> {
        if self.sessions.len() >= MAX_UPLOAD_SESSIONS {
            return Err(UploadError::CapacityReached);
        }
        let normalized_origin =
            crate::permissions::parse_origin(origin).map_err(|_| UploadError::InvalidRequest)?;
        if normalized_origin != origin || accept.len() > 32 {
            return Err(UploadError::InvalidRequest);
        }
        let id = UploadRequestId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(UploadError::IdsExhausted)?;
        let request = UploadSelectionRequest {
            id,
            tab_id,
            origin: origin.to_owned(),
            accept,
            max_files: if multiple { MAX_UPLOAD_FILES } else { 1 },
            multiple,
        };
        self.sessions.push(UploadSession {
            request: request.clone(),
            selected: Vec::new(),
            phase: UploadPhase::AwaitingSelection,
            ticket: None,
            failure: None,
        });
        let session = self.sessions.last_mut().expect("session was just inserted");
        match runtime.choose_files(&request) {
            Ok(files) if files.len() <= request.max_files => {
                session.selected = files;
                session.phase = UploadPhase::Selected;
            }
            Ok(_) => {
                session.phase = UploadPhase::Failed;
                session.failure = Some("file picker exceeded requested selection limit".to_owned());
                return Err(UploadError::TooManyFiles);
            }
            Err(error) => {
                session.phase = if error == UploadError::SelectionDenied {
                    UploadPhase::Canceled
                } else {
                    UploadPhase::Failed
                };
                session.failure = Some(format!("{error:?}"));
                return Err(error);
            }
        }
        Ok(id)
    }

    pub fn submit(
        &mut self,
        runtime: &mut impl UploadRuntime,
        id: UploadRequestId,
    ) -> Result<UploadTicket, UploadError> {
        let session = self
            .sessions
            .iter_mut()
            .find(|session| session.request.id == id)
            .ok_or(UploadError::RequestNotFound)?;
        if session.phase != UploadPhase::Selected {
            return Err(UploadError::InvalidPhase);
        }
        let ticket = runtime.submit(&session.request.origin, &session.selected)?;
        session.ticket = Some(ticket);
        session.phase = UploadPhase::Submitted;
        Ok(ticket)
    }

    pub fn cancel(&mut self, id: UploadRequestId) -> Result<(), UploadError> {
        let session = self
            .sessions
            .iter_mut()
            .find(|session| session.request.id == id)
            .ok_or(UploadError::RequestNotFound)?;
        if !matches!(
            session.phase,
            UploadPhase::AwaitingSelection | UploadPhase::Selected
        ) {
            return Err(UploadError::InvalidPhase);
        }
        session.selected.clear();
        session.phase = UploadPhase::Canceled;
        Ok(())
    }
}

impl Default for UploadsState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestRuntime;

    impl UploadRuntime for TestRuntime {
        fn choose_files(
            &mut self,
            request: &UploadSelectionRequest,
        ) -> Result<Vec<SelectedUploadFile>, UploadError> {
            assert!(request.max_files > 0);
            Ok(vec![SelectedUploadFile {
                handle: ReadOnlyFileHandle(77),
                display_name: "user-selected.txt".to_owned(),
                byte_length: 12,
            }])
        }

        fn submit(
            &mut self,
            origin: &str,
            files: &[SelectedUploadFile],
        ) -> Result<UploadTicket, UploadError> {
            assert_eq!(origin, "https://example.test");
            assert_eq!(files[0].handle, ReadOnlyFileHandle(77));
            Ok(UploadTicket(9))
        }
    }

    #[test]
    fn upload_requires_a_picker_response_then_uses_a_typed_handle_handoff() {
        let mut runtime = TestRuntime;
        let mut uploads = UploadsState::new();
        let id = uploads
            .request_selection(
                &mut runtime,
                TabId(3),
                "https://example.test",
                vec![".txt".to_owned()],
                false,
            )
            .unwrap();
        assert_eq!(uploads.sessions()[0].phase, UploadPhase::Selected);
        let ticket = uploads.submit(&mut runtime, id).unwrap();
        assert_eq!(ticket, UploadTicket(9));
        assert_eq!(uploads.sessions()[0].phase, UploadPhase::Submitted);
    }

    #[test]
    fn invalid_origin_cannot_trigger_a_file_picker_request() {
        let mut runtime = TestRuntime;
        let mut uploads = UploadsState::new();
        assert_eq!(
            uploads.request_selection(
                &mut runtime,
                TabId(3),
                "file:///etc/passwd",
                Vec::new(),
                true
            ),
            Err(UploadError::InvalidRequest)
        );
        assert!(uploads.sessions().is_empty());
    }
}
