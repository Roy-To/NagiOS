use crate::{
    CapabilityAuthorizer, EntryKind, FilesError, FilesErrorKind, FilesService, FilesystemProvider,
    Location, ResourceId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchRecord {
    pub resource_id: ResourceId,
    pub title: String,
    pub location: Location,
    pub kind: EntryKind,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<i64>,
    pub tags: Vec<String>,
}

pub struct FilesSearchProvider {
    pub max_depth: usize,
}

impl Default for FilesSearchProvider {
    fn default() -> Self {
        Self { max_depth: 64 }
    }
}

impl FilesSearchProvider {
    pub fn search<P, A>(
        &self,
        service: &FilesService<P, A>,
        start: &Location,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchRecord>, FilesError>
    where
        P: FilesystemProvider,
        A: CapabilityAuthorizer,
    {
        let query = query.trim().to_lowercase();
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let mut pending = vec![(start.clone(), 0usize)];
        let mut records = Vec::new();
        while let Some((location, depth)) = pending.pop() {
            let entries = match service.list(&location) {
                Ok(entries) => entries,
                Err(error)
                    if matches!(
                        error.kind,
                        FilesErrorKind::PermissionDenied
                            | FilesErrorKind::PermissionRequired
                            | FilesErrorKind::CapabilityUnavailable
                    ) =>
                {
                    // Omit inaccessible subtrees completely. Search does not
                    // reveal that their names or children exist.
                    continue;
                }
                Err(error) => return Err(error),
            };
            for entry in entries {
                let child = entry.child_location();
                if (entry.name.as_str().to_lowercase().contains(&query)
                    || child.as_str().to_lowercase().contains(&query)
                    || entry
                        .tags
                        .iter()
                        .any(|tag| tag.to_lowercase().contains(&query)))
                    && entry.kind != EntryKind::Symlink
                {
                    match service.metadata(&child) {
                        Ok(authorized_entry) => records.push(SearchRecord {
                            resource_id: authorized_entry.id,
                            title: authorized_entry.name.to_string(),
                            location: child.clone(),
                            kind: authorized_entry.kind,
                            size_bytes: authorized_entry.size_bytes,
                            modified_at: authorized_entry.modified_at,
                            tags: authorized_entry.tags,
                        }),
                        Err(error)
                            if matches!(
                                error.kind,
                                FilesErrorKind::PermissionDenied
                                    | FilesErrorKind::PermissionRequired
                                    | FilesErrorKind::CapabilityUnavailable
                                    | FilesErrorKind::NotFound
                                    | FilesErrorKind::SymlinkNotAllowed
                            ) => {}
                        Err(error) => return Err(error),
                    }
                }
                if entry.kind == EntryKind::Folder && depth < self.max_depth {
                    pending.push((child, depth + 1));
                }
            }
        }
        records.sort_by(|left, right| {
            let left_exact = left.title.to_lowercase() == query;
            let right_exact = right.title.to_lowercase() == query;
            right_exact
                .cmp(&left_exact)
                .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
                .then_with(|| left.location.cmp(&right.location))
        });
        records.truncate(limit);
        Ok(records)
    }
}
