use std::sync::Arc;

use nagi_model::{AppId, ObjectId, WorkspaceId};

use crate::actions::{ActionAvailability, CapabilityContext, CapabilityId, TypedAction};
use crate::localization::{Locale, LocalizationCatalog};
use crate::registry::{AppDescriptor, AppRegistry, RegistryError};
use crate::search::{
    SearchCoordinator, SearchError, SearchProvider, SearchQuery, SearchRequestId, SearchResponse,
};

/// A Home projection over the shared Workspace/Object identities. This is a
/// read model only; it does not introduce a second workspace store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceReference {
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub related_object_count: usize,
    pub last_active_unix_seconds: Option<u64>,
    pub visibility_capability: Option<CapabilityId>,
    pub open_capability: Option<CapabilityId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinuationReference {
    pub object_id: ObjectId,
    pub app_id: AppId,
    pub title: String,
    pub subtitle: Option<String>,
    pub workspace_id: Option<WorkspaceId>,
    pub last_opened_unix_seconds: Option<u64>,
    pub availability: ActionAvailability,
    pub visibility_capability: Option<CapabilityId>,
    pub open_capability: Option<CapabilityId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HomeData {
    pub current_workspace: Option<WorkspaceReference>,
    pub continuations: Vec<ContinuationReference>,
}

pub trait HomeDataSource: Send + Sync {
    /// Implementations must apply their platform visibility policy before
    /// returning workspace and continuation metadata.
    fn snapshot(&self, capabilities: &CapabilityContext) -> Result<HomeData, HomeError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HomeError {
    Registry(RegistryError),
    DataSourceUnavailable,
    Search(SearchError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HomeQuickAction {
    pub localization_key: &'static str,
    pub action: Option<TypedAction>,
    pub required_capability: Option<CapabilityId>,
    pub availability: ActionAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HomeApp {
    pub descriptor: AppDescriptor,
    pub action_availability: ActionAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HomeSnapshot {
    pub current_workspace: Option<WorkspaceReference>,
    pub continuations: Vec<ContinuationReference>,
    pub apps: Vec<HomeApp>,
    pub quick_actions: Vec<HomeQuickAction>,
    pub search_action: TypedAction,
    pub intent_action: TypedAction,
    pub locale: Locale,
    pub source_is_fixture: bool,
}

pub struct HomeController {
    app_registry: Arc<dyn AppRegistry>,
    data_source: Arc<dyn HomeDataSource>,
    search: Arc<SearchCoordinator>,
    providers: Vec<Arc<dyn SearchProvider>>,
    catalog: LocalizationCatalog,
    source_is_fixture: bool,
}

impl HomeController {
    pub fn new(
        app_registry: Arc<dyn AppRegistry>,
        data_source: Arc<dyn HomeDataSource>,
        search: Arc<SearchCoordinator>,
        providers: Vec<Arc<dyn SearchProvider>>,
        source_is_fixture: bool,
    ) -> Self {
        Self {
            app_registry,
            data_source,
            search,
            providers,
            catalog: LocalizationCatalog,
            source_is_fixture,
        }
    }

    pub fn snapshot(
        &self,
        locale: Locale,
        capabilities: CapabilityContext,
    ) -> Result<HomeSnapshot, HomeError> {
        let data = self
            .data_source
            .snapshot(&capabilities)
            .map_err(|_| HomeError::DataSourceUnavailable)?;
        let current_workspace = data
            .current_workspace
            .filter(|workspace| capabilities.allows(workspace.visibility_capability.as_ref()));
        let continuations = data
            .continuations
            .into_iter()
            .filter(|item| capabilities.allows(item.visibility_capability.as_ref()))
            .map(|mut item| {
                if !capabilities.allows(item.open_capability.as_ref()) {
                    item.availability = ActionAvailability::PermissionRequired {
                        capability: item
                            .open_capability
                            .clone()
                            .expect("permission denial has a capability"),
                    };
                }
                item
            })
            .collect();
        let apps = self
            .app_registry
            .list()
            .map_err(HomeError::Registry)?
            .into_iter()
            .map(|descriptor| {
                let action_availability =
                    if !capabilities.allows(descriptor.launch_capability.as_ref()) {
                        ActionAvailability::PermissionRequired {
                            capability: descriptor
                                .launch_capability
                                .clone()
                                .expect("permission denial has a capability"),
                        }
                    } else {
                        descriptor.availability.action_state()
                    };
                HomeApp {
                    descriptor,
                    action_availability,
                }
            })
            .collect();
        let quick_actions = vec![
            HomeQuickAction {
                localization_key: "home.open_workspace",
                action: current_workspace
                    .as_ref()
                    .map(|workspace| TypedAction::OpenWorkspace {
                        workspace_id: workspace.workspace_id,
                    }),
                required_capability: current_workspace
                    .as_ref()
                    .and_then(|workspace| workspace.open_capability.clone()),
                availability: if let Some(workspace) = &current_workspace {
                    if !capabilities.allows(workspace.open_capability.as_ref()) {
                        ActionAvailability::PermissionRequired {
                            capability: workspace
                                .open_capability
                                .clone()
                                .expect("permission denial has a capability"),
                        }
                    } else {
                        ActionAvailability::HostPreviewOnly
                    }
                } else {
                    ActionAvailability::Unavailable {
                        reason_key: "home.no_workspace".to_owned(),
                    }
                },
            },
            HomeQuickAction {
                localization_key: "home.intent_entry",
                action: Some(TypedAction::OpenIntentEntry),
                required_capability: None,
                availability: ActionAvailability::ComingSoon {
                    reason_key: "intent.router.not_available".to_owned(),
                },
            },
        ];
        Ok(HomeSnapshot {
            current_workspace,
            continuations,
            apps,
            quick_actions,
            search_action: TypedAction::OpenSearch,
            intent_action: TypedAction::OpenIntentEntry,
            locale,
            source_is_fixture: self.source_is_fixture,
        })
    }

    /// Home's embedded search and the standalone Search view share this exact
    /// coordinator and provider set.
    pub fn search(&self, query: SearchQuery) -> Result<SearchResponse, SearchError> {
        self.search.search(query, &self.providers)
    }

    pub fn allocate_search_request_id(&self) -> SearchRequestId {
        self.search.allocate_request_id()
    }

    pub fn cancel_search(&self, request_id: SearchRequestId) -> bool {
        self.search.cancel(request_id)
    }

    pub fn localized(&self, key: &str, locale: Locale) -> String {
        self.catalog.resolve(key, locale)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use nagi_model::{AppId, WorkspaceId};

    use crate::fixtures::{demo_app_registry, demo_home_source, demo_search_providers};
    use crate::search::{SearchContext, SearchCoordinator, SearchQuery, SearchRequestId};
    use crate::{CapabilityContext, CapabilityId};

    use super::HomeController;

    fn controller() -> HomeController {
        let providers = demo_search_providers();
        HomeController::new(
            Arc::new(demo_app_registry()),
            Arc::new(demo_home_source()),
            Arc::new(SearchCoordinator::new(Duration::from_millis(100), 200)),
            providers,
            true,
        )
    }

    #[test]
    fn home_references_the_shared_workspace_and_app_registry() {
        let snapshot = controller()
            .snapshot(crate::Locale::JaJp, crate::fixtures::demo_capabilities())
            .unwrap();
        assert_eq!(
            snapshot.current_workspace.as_ref().unwrap().workspace_id,
            WorkspaceId(42)
        );
        assert_eq!(snapshot.apps.len(), 8);
        assert!(snapshot.source_is_fixture);
    }

    #[test]
    fn home_builds_typed_workspace_search_and_intent_actions() {
        let snapshot = controller()
            .snapshot(crate::Locale::EnUs, crate::fixtures::demo_capabilities())
            .unwrap();
        assert_eq!(
            snapshot.quick_actions[0].action,
            Some(crate::TypedAction::OpenWorkspace {
                workspace_id: WorkspaceId(42)
            })
        );
        assert_eq!(snapshot.search_action, crate::TypedAction::OpenSearch);
        assert_eq!(snapshot.intent_action, crate::TypedAction::OpenIntentEntry);
    }

    #[test]
    fn home_search_uses_shared_deterministic_search_service() {
        let home = controller();
        let request = SearchQuery::new(
            "東京",
            SearchRequestId(1),
            SearchContext {
                current_workspace: Some(WorkspaceId(42)),
                current_app: Some(AppId::from_identifier(b"com.nagi.home")),
                now_unix_seconds: 1_800_000_000,
            },
            crate::fixtures::demo_capabilities(),
            crate::Locale::JaJp,
        );
        let results = home.search(request).unwrap();
        assert!(results
            .results
            .iter()
            .any(|result| result.title.contains("東京")));
    }

    #[test]
    fn home_drops_workspace_and_continuation_metadata_without_visibility_rights() {
        let private = CapabilityId::new("workspaces.private.read").unwrap();
        let data = super::HomeData {
            current_workspace: Some(super::WorkspaceReference {
                workspace_id: WorkspaceId(99),
                title: "Secret workspace title".to_owned(),
                related_object_count: 1,
                last_active_unix_seconds: None,
                visibility_capability: Some(private.clone()),
                open_capability: None,
            }),
            continuations: vec![super::ContinuationReference {
                object_id: nagi_model::ObjectId(99),
                app_id: AppId::from_identifier(b"com.nagi.notes"),
                title: "Secret note title".to_owned(),
                subtitle: None,
                workspace_id: Some(WorkspaceId(99)),
                last_opened_unix_seconds: None,
                availability: crate::ActionAvailability::Ready,
                visibility_capability: Some(private),
                open_capability: None,
            }],
        };
        let providers = demo_search_providers();
        let controller = HomeController::new(
            Arc::new(demo_app_registry()),
            Arc::new(crate::fixtures::FixtureHomeSource::new(data)),
            Arc::new(SearchCoordinator::new(Duration::from_millis(100), 200)),
            providers,
            true,
        );
        let snapshot = controller
            .snapshot(crate::Locale::EnUs, CapabilityContext::default())
            .unwrap();
        assert!(snapshot.current_workspace.is_none());
        assert!(snapshot.continuations.is_empty());
    }

    #[test]
    fn home_keeps_visible_metadata_but_gates_workspace_open_action() {
        let data = super::HomeData {
            current_workspace: Some(super::WorkspaceReference {
                workspace_id: WorkspaceId(42),
                title: "Visible workspace".to_owned(),
                related_object_count: 2,
                last_active_unix_seconds: None,
                visibility_capability: None,
                open_capability: Some(CapabilityId::new("workspaces.open").unwrap()),
            }),
            continuations: Vec::new(),
        };
        let providers = demo_search_providers();
        let controller = HomeController::new(
            Arc::new(demo_app_registry()),
            Arc::new(crate::fixtures::FixtureHomeSource::new(data)),
            Arc::new(SearchCoordinator::new(Duration::from_millis(100), 200)),
            providers,
            true,
        );
        let snapshot = controller
            .snapshot(crate::Locale::EnUs, CapabilityContext::default())
            .unwrap();
        assert!(snapshot.current_workspace.is_some());
        assert_eq!(
            snapshot.quick_actions[0].availability,
            crate::ActionAvailability::PermissionRequired {
                capability: CapabilityId::new("workspaces.open").unwrap()
            }
        );
    }
}
