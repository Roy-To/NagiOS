use crate::{
    Actor, CancellationToken, CapabilityAuthorizer, ConfirmationChallenge, FileEntry, FileName,
    FilesError, FilesSearchProvider, FilesService, FilesystemProvider, Location, OperationResult,
    ResourceId, SearchRecord, Tag, TrashEntry, WorkspaceReferenceResult,
};

/// Typed, local Action API for the Files action catalog. A production Action
/// Registry can adapt calls to this boundary without bypassing FilesService.
pub struct FilesActionApi;

#[derive(Clone, Debug)]
pub enum FilesActionCall {
    List {
        location: Location,
    },
    Search {
        start: Location,
        query: String,
        limit: usize,
    },
    Open {
        resource: FileEntry,
    },
    CreateFolder {
        parent: Location,
        name: FileName,
    },
    Copy {
        resource: FileEntry,
        destination: Location,
        name: FileName,
        cancellation: CancellationToken,
    },
    Move {
        resource: FileEntry,
        destination: Location,
        name: FileName,
    },
    Rename {
        resource: FileEntry,
        name: FileName,
    },
    Duplicate {
        resource: FileEntry,
        cancellation: CancellationToken,
    },
    Delete {
        resource: FileEntry,
    },
    Restore {
        trash_id: ResourceId,
    },
    GetMetadata {
        location: Location,
    },
    SetTags {
        resource: FileEntry,
        tags: Vec<Tag>,
    },
    AddToWorkspace {
        resource: FileEntry,
        workspace_id: String,
    },
    RemoveFromWorkspace {
        resource: FileEntry,
        workspace_id: String,
    },
    RequestPermanentDelete {
        trash_id: ResourceId,
    },
    ConfirmPermanentDelete {
        challenge: ConfirmationChallenge,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FilesActionResponse {
    Listed(Vec<FileEntry>),
    SearchResults(Vec<SearchRecord>),
    Opened {
        contents: Vec<u8>,
        operation: OperationResult,
    },
    ResourceChanged {
        resource: FileEntry,
        operation: OperationResult,
    },
    Trashed {
        entry: TrashEntry,
        operation: OperationResult,
    },
    Metadata(FileEntry),
    WorkspaceReference(WorkspaceReferenceResult),
    Confirmation(ConfirmationChallenge),
    Operation(OperationResult),
}

impl FilesActionApi {
    pub fn execute<P, A>(
        service: &mut FilesService<P, A>,
        call: FilesActionCall,
        actor: Actor,
    ) -> Result<FilesActionResponse, FilesError>
    where
        P: FilesystemProvider,
        A: CapabilityAuthorizer,
    {
        match call {
            FilesActionCall::List { location } => {
                service.list(&location).map(FilesActionResponse::Listed)
            }
            FilesActionCall::Search {
                start,
                query,
                limit,
            } => FilesSearchProvider::default()
                .search(service, &start, &query, limit)
                .map(FilesActionResponse::SearchResults),
            FilesActionCall::Open { resource } => {
                service
                    .open_file(&resource, actor)
                    .map(|(contents, operation)| FilesActionResponse::Opened {
                        contents,
                        operation,
                    })
            }
            FilesActionCall::CreateFolder { parent, name } => service
                .create_folder(&parent, &name, actor)
                .map(
                    |(resource, operation)| FilesActionResponse::ResourceChanged {
                        resource,
                        operation,
                    },
                ),
            FilesActionCall::Copy {
                resource,
                destination,
                name,
                cancellation,
            } => service
                .copy(&resource, &destination, &name, actor, &cancellation)
                .map(
                    |(resource, operation)| FilesActionResponse::ResourceChanged {
                        resource,
                        operation,
                    },
                ),
            FilesActionCall::Move {
                resource,
                destination,
                name,
            } => service
                .move_item(&resource, &destination, &name, actor)
                .map(
                    |(resource, operation)| FilesActionResponse::ResourceChanged {
                        resource,
                        operation,
                    },
                ),
            FilesActionCall::Rename { resource, name } => service
                .rename(&resource, &name, actor)
                .map(
                    |(resource, operation)| FilesActionResponse::ResourceChanged {
                        resource,
                        operation,
                    },
                ),
            FilesActionCall::Duplicate {
                resource,
                cancellation,
            } => service
                .duplicate(&resource, actor, &cancellation)
                .map(
                    |(resource, operation)| FilesActionResponse::ResourceChanged {
                        resource,
                        operation,
                    },
                ),
            FilesActionCall::Delete { resource } => service
                .trash(&resource, actor)
                .map(|(entry, operation)| FilesActionResponse::Trashed { entry, operation }),
            FilesActionCall::Restore { trash_id } => {
                service
                    .restore(trash_id, actor)
                    .map(
                        |(resource, operation)| FilesActionResponse::ResourceChanged {
                            resource,
                            operation,
                        },
                    )
            }
            FilesActionCall::GetMetadata { location } => service
                .metadata(&location)
                .map(FilesActionResponse::Metadata),
            FilesActionCall::SetTags { resource, tags } => service
                .set_tags(&resource, &tags, actor)
                .map(
                    |(resource, operation)| FilesActionResponse::ResourceChanged {
                        resource,
                        operation,
                    },
                ),
            FilesActionCall::AddToWorkspace {
                resource,
                workspace_id,
            } => service
                .add_to_workspace(&resource, &workspace_id, actor)
                .map(FilesActionResponse::WorkspaceReference),
            FilesActionCall::RemoveFromWorkspace {
                resource,
                workspace_id,
            } => service
                .remove_from_workspace(&resource, &workspace_id, actor)
                .map(FilesActionResponse::WorkspaceReference),
            FilesActionCall::RequestPermanentDelete { trash_id } => service
                .request_permanent_delete(trash_id)
                .map(FilesActionResponse::Confirmation),
            FilesActionCall::ConfirmPermanentDelete { challenge } => service
                .confirm_permanent_delete(challenge, actor)
                .map(FilesActionResponse::Operation),
        }
    }
}
