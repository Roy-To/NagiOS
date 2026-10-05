//! Browser download presentation state and a typed destination/runtime handoff.

use crate::history::bounded_text;

pub const MAX_DOWNLOADS: usize = 256;
pub const MAX_DOWNLOAD_FILENAME_BYTES: usize = 255;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DownloadId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DownloadTicket(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadPhase {
    Pending,
    Active,
    Completed,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadRequest {
    pub id: DownloadId,
    pub source_url: String,
    pub suggested_filename: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadItem {
    pub id: DownloadId,
    pub source_url: String,
    pub display_filename: String,
    pub phase: DownloadPhase,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub failure: Option<String>,
    pub(crate) ticket: Option<DownloadTicket>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DownloadError {
    InvalidSource,
    CapacityReached,
    ItemNotFound,
    TicketNotFound,
    RuntimeRejected,
    IdsExhausted,
}

pub trait DownloadRuntime {
    fn start(&mut self, request: &DownloadRequest) -> Result<DownloadTicket, DownloadError>;
    fn cancel(&mut self, ticket: DownloadTicket) -> Result<(), DownloadError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadsState {
    items: Vec<DownloadItem>,
    next_id: u64,
}

impl DownloadsState {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            next_id: 1,
        }
    }

    pub fn items(&self) -> &[DownloadItem] {
        &self.items
    }

    pub fn request(
        &mut self,
        runtime: &mut impl DownloadRuntime,
        source_url: &str,
        suggested_filename: &str,
    ) -> Result<DownloadId, DownloadError> {
        let source_url = crate::address_bar::normalize_address(source_url)
            .map_err(|_| DownloadError::InvalidSource)?;
        if !source_url.starts_with("http://") && !source_url.starts_with("https://") {
            return Err(DownloadError::InvalidSource);
        }
        if self.items.len() >= MAX_DOWNLOADS {
            return Err(DownloadError::CapacityReached);
        }
        let id = DownloadId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(DownloadError::IdsExhausted)?;
        let filename = safe_filename(suggested_filename);
        let request = DownloadRequest {
            id,
            source_url,
            suggested_filename: filename.clone(),
        };
        self.items.push(DownloadItem {
            id,
            source_url: request.source_url.clone(),
            display_filename: filename,
            phase: DownloadPhase::Pending,
            received_bytes: 0,
            total_bytes: None,
            failure: None,
            ticket: None,
        });
        let item = self.items.last_mut().expect("item was just inserted");
        match runtime.start(&request) {
            Ok(ticket) => {
                item.ticket = Some(ticket);
                item.phase = DownloadPhase::Active;
            }
            Err(_) => {
                item.phase = DownloadPhase::Failed;
                item.failure = Some("download runtime rejected request".to_owned());
            }
        }
        Ok(id)
    }

    pub fn report_progress(
        &mut self,
        ticket: DownloadTicket,
        received_bytes: u64,
        total_bytes: Option<u64>,
    ) -> Result<(), DownloadError> {
        let item = self.by_ticket_mut(ticket)?;
        if item.phase != DownloadPhase::Active {
            return Err(DownloadError::TicketNotFound);
        }
        item.received_bytes = received_bytes;
        item.total_bytes = total_bytes;
        Ok(())
    }

    pub fn complete(&mut self, ticket: DownloadTicket) -> Result<(), DownloadError> {
        let item = self.by_ticket_mut(ticket)?;
        if item.phase != DownloadPhase::Active {
            return Err(DownloadError::TicketNotFound);
        }
        item.phase = DownloadPhase::Completed;
        Ok(())
    }

    pub fn fail(&mut self, ticket: DownloadTicket, message: &str) -> Result<(), DownloadError> {
        let item = self.by_ticket_mut(ticket)?;
        if item.phase != DownloadPhase::Active {
            return Err(DownloadError::TicketNotFound);
        }
        item.phase = DownloadPhase::Failed;
        item.failure = Some(bounded_text(message, 512));
        Ok(())
    }

    pub fn cancel(
        &mut self,
        runtime: &mut impl DownloadRuntime,
        id: DownloadId,
    ) -> Result<(), DownloadError> {
        let item = self
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or(DownloadError::ItemNotFound)?;
        match item.phase {
            DownloadPhase::Pending => {
                item.phase = DownloadPhase::Canceled;
                Ok(())
            }
            DownloadPhase::Active => {
                let ticket = item.ticket.ok_or(DownloadError::TicketNotFound)?;
                runtime.cancel(ticket)?;
                item.phase = DownloadPhase::Canceled;
                Ok(())
            }
            DownloadPhase::Completed | DownloadPhase::Failed | DownloadPhase::Canceled => {
                Err(DownloadError::ItemNotFound)
            }
        }
    }

    fn by_ticket_mut(
        &mut self,
        ticket: DownloadTicket,
    ) -> Result<&mut DownloadItem, DownloadError> {
        self.items
            .iter_mut()
            .find(|item| item.ticket == Some(ticket))
            .ok_or(DownloadError::TicketNotFound)
    }
}

impl Default for DownloadsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Strip path syntax and control characters; the file service selects a destination.
/// Folder downloads are saved to.
pub const DOWNLOAD_DIRECTORY: &str = "/Downloads";

/// Choose a file name for a download in a folder whose entries are limited
/// to `max_name_bytes`: sanitize the page's suggestion, keep its extension
/// when shortening, and add ` (n)` while `exists` reports a collision.
/// Returns `None` when no free name is found.
pub fn download_file_name(
    suggested: &str,
    max_name_bytes: usize,
    exists: impl Fn(&str) -> bool,
) -> Option<String> {
    let safe = safe_filename(suggested);
    let (stem, extension) = match safe.rfind('.') {
        Some(dot) if dot > 0 && safe.len() - dot <= 8 => (&safe[..dot], &safe[dot..]),
        _ => (safe.as_str(), ""),
    };
    let fit = |stem: &str, suffix: &str| -> Option<String> {
        let budget = max_name_bytes.checked_sub(suffix.len() + extension.len())?;
        let mut end = stem.len().min(budget);
        while !stem.is_char_boundary(end) {
            end -= 1;
        }
        let stem = stem[..end].trim_end();
        (!stem.is_empty()).then(|| format!("{stem}{suffix}{extension}"))
    };
    (0..10).find_map(|attempt| {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!(" ({attempt})")
        };
        fit(stem, &suffix).filter(|name| !exists(name))
    })
}

pub fn safe_filename(suggested: &str) -> String {
    let leaf = suggested.rsplit(['/', '\\']).next().unwrap_or_default();
    let safe: String = leaf
        .chars()
        .filter(|character| !character.is_control() && *character != ':')
        .collect();
    let safe = safe.trim().trim_matches('.');
    if safe.is_empty() {
        "download".to_owned()
    } else {
        bounded_text(safe, MAX_DOWNLOAD_FILENAME_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestRuntime {
        canceled: Vec<DownloadTicket>,
    }

    impl DownloadRuntime for TestRuntime {
        fn start(&mut self, request: &DownloadRequest) -> Result<DownloadTicket, DownloadError> {
            Ok(DownloadTicket(request.id.0 + 100))
        }

        fn cancel(&mut self, ticket: DownloadTicket) -> Result<(), DownloadError> {
            self.canceled.push(ticket);
            Ok(())
        }
    }

    #[test]
    fn lifecycle_is_driven_by_typed_runtime_tickets() {
        let mut runtime = TestRuntime {
            canceled: Vec::new(),
        };
        let mut downloads = DownloadsState::new();
        let id = downloads
            .request(&mut runtime, "https://example.test/file", "../report.txt")
            .unwrap();
        assert_eq!(downloads.items()[0].display_filename, "report.txt");
        assert_eq!(downloads.items()[0].phase, DownloadPhase::Active);
        let ticket = downloads.items()[0].ticket.unwrap();
        downloads.report_progress(ticket, 8, Some(16)).unwrap();
        assert_eq!(downloads.items()[0].received_bytes, 8);
        downloads.complete(ticket).unwrap();
        assert_eq!(downloads.items()[0].phase, DownloadPhase::Completed);
        assert_eq!(
            downloads.cancel(&mut runtime, id),
            Err(DownloadError::ItemNotFound)
        );
    }

    #[test]
    fn cancellation_is_sent_to_the_runtime_and_paths_are_not_retained() {
        let mut runtime = TestRuntime {
            canceled: Vec::new(),
        };
        let mut downloads = DownloadsState::new();
        let id = downloads
            .request(
                &mut runtime,
                "https://example.test/file",
                "C:\\tmp\\file.bin",
            )
            .unwrap();
        downloads.cancel(&mut runtime, id).unwrap();
        assert_eq!(runtime.canceled, vec![DownloadTicket(101)]);
        assert_eq!(downloads.items()[0].phase, DownloadPhase::Canceled);
        assert_eq!(downloads.items()[0].display_filename, "file.bin");
    }

    #[test]
    fn download_names_fit_the_folder_and_avoid_collisions() {
        use super::download_file_name;
        assert_eq!(
            download_file_name("../../etc/report.txt", 32, |_| false).as_deref(),
            Some("report.txt")
        );
        let long = "a-very-long-download-name-from-the-page.pdf";
        let name = download_file_name(long, 32, |_| false).unwrap();
        assert!(name.len() <= 32 && name.ends_with(".pdf"), "{name}");
        assert_eq!(
            download_file_name("x.txt", 32, |name| name == "x.txt").as_deref(),
            Some("x (1).txt")
        );
        assert_eq!(download_file_name("x.txt", 32, |_| true), None);
        assert_eq!(
            download_file_name("", 32, |_| false).as_deref(),
            Some("download")
        );
    }
}
