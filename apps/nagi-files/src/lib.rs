//! Nagi Files domain and host-preview implementation.
//!
//! The host provider is deliberately sandboxed and is not the Nagi target
//! filesystem adapter. Target integration must implement [`FilesystemProvider`]
//! using the public Nagi storage and capability services.

mod action_api;
mod actions;
mod backend;
mod canvas;
mod capability;
mod component;
pub mod localization;
mod memory;
mod model;
#[doc(hidden)]
pub mod preview_cli;
mod sandbox;
mod search;
mod service;
mod ui;

pub use action_api::{FilesActionApi, FilesActionCall, FilesActionResponse};
pub use actions::{ActionDescriptor, ActionRisk, FILES_ACTIONS, find_action};
pub use backend::{FilesOperationScopeToken, FilesystemProvider, ProviderAvailability};
pub use canvas::{FilesCanvas, FilesCanvasRect, draw_three_pane};
pub use capability::{CapabilityAuthorizer, CapabilityGrant, CapabilitySet};
pub use component::FilesComponent;
pub use memory::InMemoryProvider;
pub use model::*;
pub use sandbox::SandboxProvider;
pub use search::{FilesSearchProvider, SearchRecord};
pub use service::{
    ActivityEvent, ActivityOutcome, ActivitySink, CheckpointHook, CheckpointSnapshot, FilesService,
    HookStatus, NoopActivitySink, NoopCheckpointHook, OperationOutcome, OperationResult,
    WaybackCheckpointRequest, WorkspaceReferenceResult, WorkspaceReferenceSink,
};
pub use ui::{
    ContextPublisher, FilesApp, FilesLayout, FilesViewState, InspectorTab, LocationItem,
    SidebarItem, ViewLoadState, render_three_pane,
};

#[cfg(test)]
mod tests;
