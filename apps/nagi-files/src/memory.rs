use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    CancellationToken, EntryAvailability, EntryKind, FileEntry, FileName, FilesError,
    FilesErrorKind, FilesystemProvider, Location, ProviderAvailability, ResourceId, TrashEntry,
};

#[derive(Clone)]
struct Node {
    id: ResourceId,
    kind: EntryKind,
    bytes: Vec<u8>,
    tags: Vec<String>,
    created_at: i64,
    modified_at: i64,
}

struct TrashedSubtree {
    entry: TrashEntry,
    original: Location,
    nodes: Vec<(Location, Node)>,
}

/// Deterministic provider used by orchestration tests and UI previews.
/// It is in-memory only and does not claim persistence.
pub struct InMemoryProvider {
    nodes: BTreeMap<Location, Node>,
    trash: BTreeMap<ResourceId, TrashedSubtree>,
    next_id: u128,
    availability: ProviderAvailability,
}

impl InMemoryProvider {
    pub fn new() -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            Location::root(),
            Node {
                id: ResourceId(1),
                kind: EntryKind::Folder,
                bytes: Vec::new(),
                tags: Vec::new(),
                created_at: now(),
                modified_at: now(),
            },
        );
        Self {
            nodes,
            trash: BTreeMap::new(),
            next_id: 2,
            availability: ProviderAvailability::Available,
        }
    }

    pub fn set_unavailable(&mut self, reason: impl Into<String>) {
        self.availability = ProviderAvailability::Unavailable(reason.into());
    }

    pub fn set_available(&mut self) {
        self.availability = ProviderAvailability::Available;
    }

    /// Adds deterministic test content. Normal app code should use the typed
    /// operation service instead of seeding a provider directly.
    pub fn insert_file(
        &mut self,
        location: &Location,
        contents: &[u8],
    ) -> Result<FileEntry, FilesError> {
        let name = location
            .file_name()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidName))?;
        let name = FileName::parse(name)?;
        let parent = location
            .parent()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        self.require_folder(&parent)?;
        if self.nodes.contains_key(location) {
            return Err(FilesError::new(FilesErrorKind::AlreadyExists).at(location.clone()));
        }
        let id = self.allocate_id();
        let timestamp = now();
        self.nodes.insert(
            location.clone(),
            Node {
                id,
                kind: EntryKind::File,
                bytes: contents.to_vec(),
                tags: Vec::new(),
                created_at: timestamp,
                modified_at: timestamp,
            },
        );
        self.to_entry(location, &name)
    }

    pub fn insert_folder(&mut self, location: &Location) -> Result<FileEntry, FilesError> {
        let name = location
            .file_name()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidName))?;
        self.create_folder(
            &location
                .parent()
                .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?,
            &FileName::parse(name)?,
        )
    }

    fn allocate_id(&mut self) -> ResourceId {
        let id = ResourceId(self.next_id);
        self.next_id += 1;
        id
    }

    fn require_folder(&self, location: &Location) -> Result<(), FilesError> {
        match self.nodes.get(location) {
            Some(node) if node.kind == EntryKind::Folder => Ok(()),
            Some(_) => Err(FilesError::new(FilesErrorKind::NotDirectory).at(location.clone())),
            None => Err(FilesError::new(FilesErrorKind::NotFound).at(location.clone())),
        }
    }

    fn to_entry(&self, location: &Location, name: &FileName) -> Result<FileEntry, FilesError> {
        let node = self
            .nodes
            .get(location)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound).at(location.clone()))?;
        Ok(FileEntry {
            id: node.id,
            name: name.clone(),
            location: location.parent().unwrap_or_else(Location::root),
            kind: node.kind,
            size_bytes: (node.kind == EntryKind::File).then_some(node.bytes.len() as u64),
            created_at: Some(node.created_at),
            modified_at: Some(node.modified_at),
            tags: node.tags.clone(),
            availability: EntryAvailability::Available,
        })
    }

    fn resolve_name(location: &Location) -> Result<FileName, FilesError> {
        FileName::parse(
            location
                .file_name()
                .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?,
        )
    }

    fn subtree_paths(&self, source: &Location) -> Result<Vec<Location>, FilesError> {
        if !self.nodes.contains_key(source) {
            return Err(FilesError::new(FilesErrorKind::NotFound).at(source.clone()));
        }
        Ok(self
            .nodes
            .keys()
            .filter(|path| path == &source || is_descendant(path, source))
            .cloned()
            .collect())
    }

    fn subtree_size(&self, source: &Location) -> u64 {
        self.nodes
            .iter()
            .filter(|(path, _)| *path == source || is_descendant(path, source))
            .map(|(_, node)| node.bytes.len() as u64)
            .sum()
    }
}

impl Default for InMemoryProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FilesystemProvider for InMemoryProvider {
    fn availability(&self) -> ProviderAvailability {
        self.availability.clone()
    }

    fn list(&self, location: &Location) -> Result<Vec<FileEntry>, FilesError> {
        self.require_folder(location)?;
        let mut entries = Vec::new();
        for (path, node) in &self.nodes {
            if path.parent().as_ref() != Some(location) {
                continue;
            }
            let name = Self::resolve_name(path)?;
            entries.push(FileEntry {
                id: node.id,
                name,
                location: location.clone(),
                kind: node.kind,
                size_bytes: (node.kind == EntryKind::File).then_some(node.bytes.len() as u64),
                created_at: Some(node.created_at),
                modified_at: Some(node.modified_at),
                tags: node.tags.clone(),
                availability: EntryAvailability::Available,
            });
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
        Ok(entries)
    }

    fn metadata(&self, location: &Location) -> Result<FileEntry, FilesError> {
        self.to_entry(location, &Self::resolve_name(location)?)
    }

    fn read_file(&self, location: &Location, max_bytes: usize) -> Result<Vec<u8>, FilesError> {
        let node = self
            .nodes
            .get(location)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound).at(location.clone()))?;
        if node.kind != EntryKind::File {
            return Err(FilesError::new(FilesErrorKind::IsDirectory).at(location.clone()));
        }
        if node.bytes.len() > max_bytes {
            return Err(FilesError::new(FilesErrorKind::FileTooLarge).at(location.clone()));
        }
        Ok(node.bytes.clone())
    }

    fn set_tags(&mut self, location: &Location, tags: &[String]) -> Result<FileEntry, FilesError> {
        let node = self
            .nodes
            .get_mut(location)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound).at(location.clone()))?;
        node.tags = tags.to_vec();
        let name = Self::resolve_name(location)?;
        self.to_entry(location, &name)
    }

    fn create_folder(
        &mut self,
        parent: &Location,
        name: &FileName,
    ) -> Result<FileEntry, FilesError> {
        self.require_folder(parent)?;
        let location = parent.join(name);
        if self.nodes.contains_key(&location) {
            return Err(FilesError::new(FilesErrorKind::AlreadyExists).at(location));
        }
        let id = self.allocate_id();
        let timestamp = now();
        self.nodes.insert(
            location.clone(),
            Node {
                id,
                kind: EntryKind::Folder,
                bytes: Vec::new(),
                tags: Vec::new(),
                created_at: timestamp,
                modified_at: timestamp,
            },
        );
        self.to_entry(&location, name)
    }

    fn rename(&mut self, source: &Location, name: &FileName) -> Result<FileEntry, FilesError> {
        if source.is_root() {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        if !self.nodes.contains_key(source) {
            return Err(FilesError::new(FilesErrorKind::NotFound).at(source.clone()));
        }
        let parent = source.parent().expect("non-root location has parent");
        let destination = parent.join(name);
        if destination != *source && self.nodes.contains_key(&destination) {
            return Err(FilesError::new(FilesErrorKind::Conflict).at(destination));
        }
        if destination != *source {
            self.relocate_subtree(source, &destination)?;
        }
        self.to_entry(&destination, name)
    }

    fn copy(
        &mut self,
        source: &Location,
        destination: &Location,
        name: &FileName,
        cancellation: &CancellationToken,
    ) -> Result<FileEntry, FilesError> {
        self.require_folder(destination)?;
        let target = destination.join(name);
        if self.nodes.contains_key(&target) {
            return Err(FilesError::new(FilesErrorKind::Conflict).at(target));
        }
        let source_paths = self.subtree_paths(source)?;
        let mut additions = Vec::new();
        for old_path in source_paths {
            if cancellation.is_cancelled() {
                return Err(FilesError::new(FilesErrorKind::Cancelled));
            }
            let node = self
                .nodes
                .get(&old_path)
                .expect("enumerated node exists")
                .clone();
            let suffix = old_path
                .as_str()
                .strip_prefix(source.as_str())
                .expect("subtree path prefix");
            let new_path = Location::parse(&format!("{}{}", target.as_str(), suffix))?;
            let id = self.allocate_id();
            additions.push((
                new_path,
                Node {
                    id,
                    tags: Vec::new(),
                    ..node
                },
            ));
        }
        let root_entry_path = target.clone();
        for (path, node) in additions {
            self.nodes.insert(path, node);
        }
        self.to_entry(&root_entry_path, name)
    }

    fn move_item(
        &mut self,
        source: &Location,
        destination: &Location,
        name: &FileName,
    ) -> Result<FileEntry, FilesError> {
        self.require_folder(destination)?;
        let target = destination.join(name);
        if self.nodes.contains_key(&target) {
            return Err(FilesError::new(FilesErrorKind::Conflict).at(target));
        }
        if target == *source || is_descendant(&target, source) {
            return Err(FilesError::new(FilesErrorKind::DestinationInsideSource).at(target));
        }
        self.relocate_subtree(source, &target)?;
        self.to_entry(&target, name)
    }

    fn trash(&mut self, source: &Location) -> Result<TrashEntry, FilesError> {
        if source.is_root() {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        let id = self
            .nodes
            .get(source)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound).at(source.clone()))?
            .id;
        let kind = self.nodes.get(source).expect("checked").kind;
        let size_bytes = Some(self.subtree_size(source));
        let deleted_at = now();
        let paths = self.subtree_paths(source)?;
        let mut nodes = Vec::with_capacity(paths.len());
        for path in paths {
            if let Some(node) = self.nodes.remove(&path) {
                nodes.push((path, node));
            }
        }
        let entry = TrashEntry {
            id,
            original_location: source.clone(),
            kind,
            size_bytes,
            deleted_at,
        };
        self.trash.insert(
            id,
            TrashedSubtree {
                entry: entry.clone(),
                original: source.clone(),
                nodes,
            },
        );
        Ok(entry)
    }

    fn list_trash(&self) -> Result<Vec<TrashEntry>, FilesError> {
        Ok(self.trash.values().map(|item| item.entry.clone()).collect())
    }

    fn restore(&mut self, id: ResourceId) -> Result<FileEntry, FilesError> {
        let trashed = self
            .trash
            .get(&id)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
        let parent = trashed
            .original
            .parent()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        self.require_folder(&parent)?;
        if self.nodes.contains_key(&trashed.original) {
            return Err(FilesError::new(FilesErrorKind::Conflict).at(trashed.original.clone()));
        }
        let trashed = self.trash.remove(&id).expect("checked");
        let name = Self::resolve_name(&trashed.original)?;
        for (path, node) in trashed.nodes {
            self.nodes.insert(path, node);
        }
        self.to_entry(&trashed.original, &name)
    }

    fn permanently_delete(&mut self, id: ResourceId) -> Result<(), FilesError> {
        self.trash
            .remove(&id)
            .map(|_| ())
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))
    }
}

impl InMemoryProvider {
    fn relocate_subtree(
        &mut self,
        source: &Location,
        destination: &Location,
    ) -> Result<(), FilesError> {
        if !self.nodes.contains_key(source) {
            return Err(FilesError::new(FilesErrorKind::NotFound).at(source.clone()));
        }
        let paths = self.subtree_paths(source)?;
        let mut moved = Vec::with_capacity(paths.len());
        for path in paths {
            let node = self.nodes.remove(&path).expect("enumerated node exists");
            let suffix = path
                .as_str()
                .strip_prefix(source.as_str())
                .expect("subtree prefix");
            let new_path = Location::parse(&format!("{}{}", destination.as_str(), suffix))?;
            moved.push((new_path, node));
        }
        for (path, node) in moved {
            self.nodes.insert(path, node);
        }
        Ok(())
    }
}

fn is_descendant(path: &Location, parent: &Location) -> bool {
    path.as_str()
        .strip_prefix(parent.as_str())
        .is_some_and(|suffix| !suffix.is_empty() && suffix.starts_with('/'))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}
