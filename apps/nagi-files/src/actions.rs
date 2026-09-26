use crate::CapabilityRight;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionRisk {
    Read,
    Modify,
    Destructive,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionDescriptor {
    pub id: &'static str,
    pub input_schema: &'static str,
    pub output_schema: &'static str,
    pub risk: ActionRisk,
    pub reversible: bool,
    pub required_rights: &'static [CapabilityRight],
}

const READ: &[CapabilityRight] = &[CapabilityRight::Read];
const ENUMERATE: &[CapabilityRight] = &[CapabilityRight::Enumerate];
const SEARCH: &[CapabilityRight] = &[CapabilityRight::Enumerate, CapabilityRight::Read];
const CREATE: &[CapabilityRight] = &[CapabilityRight::Create];
const COPY: &[CapabilityRight] = &[CapabilityRight::Read, CapabilityRight::Create];
const MOVE: &[CapabilityRight] = &[CapabilityRight::Move, CapabilityRight::Create];
const RENAME: &[CapabilityRight] = &[CapabilityRight::Rename];
const DUPLICATE: &[CapabilityRight] = &[
    CapabilityRight::Enumerate,
    CapabilityRight::Read,
    CapabilityRight::Create,
];
const DELETE: &[CapabilityRight] = &[CapabilityRight::Delete];
const RESTORE: &[CapabilityRight] = &[CapabilityRight::Restore];
const METADATA: &[CapabilityRight] = &[CapabilityRight::SetMetadata];
const WORKSPACE: &[CapabilityRight] = &[CapabilityRight::Read, CapabilityRight::WorkspaceReference];
const PERMANENT_DELETE: &[CapabilityRight] = &[CapabilityRight::PermanentDelete];

pub const FILES_ACTIONS: &[ActionDescriptor] = &[
    descriptor(
        "files.list",
        "files.list.v1",
        "files.entries.v1",
        ActionRisk::Read,
        true,
        ENUMERATE,
    ),
    descriptor(
        "files.search",
        "files.search.v1",
        "files.search_results.v1",
        ActionRisk::Read,
        true,
        SEARCH,
    ),
    descriptor(
        "files.open",
        "files.open.v1",
        "files.open_result.v1",
        ActionRisk::Read,
        true,
        READ,
    ),
    descriptor(
        "files.create_folder",
        "files.create_folder.v1",
        "files.resource.v1",
        ActionRisk::Modify,
        true,
        CREATE,
    ),
    descriptor(
        "files.copy",
        "files.copy.v1",
        "files.resource.v1",
        ActionRisk::Modify,
        true,
        COPY,
    ),
    descriptor(
        "files.move",
        "files.move.v1",
        "files.resource.v1",
        ActionRisk::Modify,
        true,
        MOVE,
    ),
    descriptor(
        "files.rename",
        "files.rename.v1",
        "files.resource.v1",
        ActionRisk::Modify,
        true,
        RENAME,
    ),
    descriptor(
        "files.duplicate",
        "files.duplicate.v1",
        "files.resource.v1",
        ActionRisk::Modify,
        true,
        DUPLICATE,
    ),
    descriptor(
        "files.delete",
        "files.delete.v1",
        "files.trash_entry.v1",
        ActionRisk::Modify,
        true,
        DELETE,
    ),
    descriptor(
        "files.restore",
        "files.restore.v1",
        "files.resource.v1",
        ActionRisk::Modify,
        true,
        RESTORE,
    ),
    descriptor(
        "files.get_metadata",
        "files.get_metadata.v1",
        "files.metadata.v1",
        ActionRisk::Read,
        true,
        READ,
    ),
    descriptor(
        "files.set_tags",
        "files.set_tags.v1",
        "files.metadata.v1",
        ActionRisk::Modify,
        true,
        METADATA,
    ),
    descriptor(
        "files.add_to_workspace",
        "files.add_to_workspace.v1",
        "files.workspace_reference.v1",
        ActionRisk::Modify,
        true,
        WORKSPACE,
    ),
    descriptor(
        "files.remove_from_workspace",
        "files.remove_from_workspace.v1",
        "files.workspace_reference.v1",
        ActionRisk::Modify,
        true,
        WORKSPACE,
    ),
    descriptor(
        "files.delete_permanently",
        "files.delete_permanently.v1",
        "files.operation_result.v1",
        ActionRisk::Destructive,
        false,
        PERMANENT_DELETE,
    ),
];

pub fn find_action(id: &str) -> Option<&'static ActionDescriptor> {
    FILES_ACTIONS.iter().find(|action| action.id == id)
}

const fn descriptor(
    id: &'static str,
    input_schema: &'static str,
    output_schema: &'static str,
    risk: ActionRisk,
    reversible: bool,
    required_rights: &'static [CapabilityRight],
) -> ActionDescriptor {
    ActionDescriptor {
        id,
        input_schema,
        output_schema,
        risk,
        reversible,
        required_rights,
    }
}
