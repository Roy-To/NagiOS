//! Nagi Files domain and host-preview implementation.
//!
//! The host provider is deliberately sandboxed and is not the Nagi target
//! filesystem adapter. Target integration must implement [`FilesystemProvider`]
//! using the public Nagi storage and capability services.

mod action_api;
mod actions;
mod backend;
mod capability;
pub mod localization;
mod memory;
mod model;
mod sandbox;
mod search;
mod service;
mod ui;

pub use action_api::{FilesActionApi, FilesActionCall, FilesActionResponse};
pub use actions::{find_action, ActionDescriptor, ActionRisk, FILES_ACTIONS};
pub use backend::{FilesystemProvider, ProviderAvailability};
pub use capability::{CapabilityAuthorizer, CapabilityGrant, CapabilitySet};
pub use memory::InMemoryProvider;
pub use model::*;
pub use sandbox::SandboxProvider;
pub use search::{FilesSearchProvider, SearchRecord};
pub use service::{
    ActivityEvent, ActivityOutcome, ActivitySink, CheckpointHook, FilesService, HookStatus,
    NoopActivitySink, NoopCheckpointHook, OperationOutcome, OperationResult,
    WaybackCheckpointRequest, WorkspaceReferenceResult, WorkspaceReferenceSink,
};
pub use ui::{
    render_three_pane, ContextPublisher, FilesApp, FilesLayout, FilesViewState, InspectorTab,
    LocationItem, SidebarItem, ViewLoadState,
};

#[cfg(test)]
mod tests;
