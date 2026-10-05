//! Clearly labeled host-preview fixtures. No provider reads the host
//! filesystem, app process table, or Nagi guest storage.

use std::sync::Arc;

use nagi_model::{AppId, ObjectId, WorkspaceId};

use crate::actions::{ActionAvailability, CapabilityContext, CapabilityId, TypedAction};
use crate::home::{ContinuationReference, HomeData, HomeDataSource, HomeError, WorkspaceReference};
use crate::localization::{Locale, LocalizationCatalog};
use crate::registry::{
    AppAvailability, AppDescriptor, AppRegistry, IconMetadata, RegistryError, RegistrySnapshot,
};
use crate::search::{
    normalize_search_text, ProviderDescriptor, ProviderError, ProviderId, SearchAction,
    SearchCandidate, SearchCategory, SearchIdentity, SearchProvider, SearchQuery, SearchText,
};
use crate::{HomeController, SearchCoordinator};
use std::time::Duration;

pub fn demo_app_registry() -> RegistrySnapshot {
    let entries = [
        app(
            "com.nagi.albert",
            "app.albert",
            'A',
            [62, 111, 214],
            10,
            AppAvailability::Unavailable {
                reason_key: "app.albert.runtime_unavailable".to_owned(),
            },
        ),
        app(
            "com.nagi.files",
            "app.files",
            'F',
            [67, 160, 71],
            20,
            AppAvailability::ComingSoon,
        ),
        app(
            "com.nagi.notes",
            "app.notes",
            'N',
            [245, 176, 65],
            30,
            AppAvailability::ComingSoon,
        ),
        app(
            "com.nagi.terminal",
            "app.terminal",
            '›',
            [50, 57, 69],
            40,
            AppAvailability::Unlaunchable {
                reason_key: "app.launch.runtime_unavailable".to_owned(),
            },
        ),
        app(
            "com.nagi.activity",
            "app.activity",
            '◷',
            [21, 148, 159],
            50,
            AppAvailability::ComingSoon,
        ),
        app(
            "com.nagi.wayback",
            "app.wayback",
            '↶',
            [127, 87, 194],
            60,
            AppAvailability::ComingSoon,
        ),
        app(
            "com.nagi.search",
            "app.search",
            '⌕',
            [37, 99, 235],
            70,
            AppAvailability::HostPreview,
        ),
        app(
            "com.nagi.home",
            "app.home",
            '⌂',
            [14, 165, 164],
            80,
            AppAvailability::HostPreview,
        ),
    ];
    RegistrySnapshot::new(entries.into_iter().collect()).expect("demo app identities are unique")
}

fn app(
    id: &str,
    name_key: &str,
    glyph: char,
    accent_rgb: [u8; 3],
    order: u16,
    availability: AppAvailability,
) -> AppDescriptor {
    let app_id = AppId::from_identifier(id.as_bytes());
    AppDescriptor {
        app_id,
        localization_name_key: name_key.to_owned(),
        localization_description_key: format!("{name_key}.description"),
        manifest_name_fallback: None,
        manifest_description_fallback: None,
        icon: IconMetadata {
            asset_key: format!("icons/{id}"),
            fallback_glyph: glyph,
            accent_rgb,
        },
        launcher_order: order,
        availability,
        launch_capability: Some(
            CapabilityId::new("apps.launch").expect("static capability identifier is valid"),
        ),
        preview_route: match id {
            "com.nagi.search" => Some("#/search".to_owned()),
            "com.nagi.home" => Some("#/".to_owned()),
            _ => None,
        },
        launch_action: TypedAction::LaunchApp { app_id },
    }
}

#[derive(Clone, Debug)]
pub struct FixtureHomeSource {
    data: HomeData,
    fail: bool,
}

impl FixtureHomeSource {
    pub fn new(data: HomeData) -> Self {
        Self { data, fail: false }
    }

    #[cfg(test)]
    pub fn failing() -> Self {
        Self {
            data: HomeData::default(),
            fail: true,
        }
    }
}

impl HomeDataSource for FixtureHomeSource {
    fn snapshot(&self, _capabilities: &CapabilityContext) -> Result<HomeData, HomeError> {
        if self.fail {
            return Err(HomeError::DataSourceUnavailable);
        }
        Ok(self.data.clone())
    }
}

pub fn demo_home_source() -> FixtureHomeSource {
    FixtureHomeSource::new(HomeData {
        current_workspace: Some(WorkspaceReference {
            workspace_id: WorkspaceId(42),
            title: "Home + Search Design".to_owned(),
            related_object_count: 4,
            last_active_unix_seconds: Some(1_800_000_000 - 3_600),
            visibility_capability: Some(CapabilityId::new("workspaces.read").unwrap()),
            open_capability: Some(CapabilityId::new("workspaces.open").unwrap()),
        }),
        continuations: vec![
            ContinuationReference {
                object_id: ObjectId(1001),
                app_id: AppId::from_identifier(b"com.nagi.notes"),
                title: "Home 検索設計メモ".to_owned(),
                subtitle: Some("Provider contract and keyboard flow".to_owned()),
                workspace_id: Some(WorkspaceId(42)),
                last_opened_unix_seconds: Some(1_800_000_000 - 900),
                availability: ActionAvailability::ComingSoon {
                    reason_key: "app.notes.runtime_unavailable".to_owned(),
                },
                visibility_capability: Some(CapabilityId::new("notes.metadata.read").unwrap()),
                open_capability: Some(CapabilityId::new("notes.open").unwrap()),
            },
            ContinuationReference {
                object_id: ObjectId(1002),
                app_id: AppId::from_identifier(b"com.nagi.files"),
                title: "Search Workstream.md".to_owned(),
                subtitle: Some("Mock file · project notes".to_owned()),
                workspace_id: Some(WorkspaceId(42)),
                last_opened_unix_seconds: Some(1_800_000_000 - 86_400),
                availability: ActionAvailability::ComingSoon {
                    reason_key: "app.files.runtime_unavailable".to_owned(),
                },
                visibility_capability: Some(CapabilityId::new("files.metadata.read").unwrap()),
                open_capability: Some(CapabilityId::new("files.open").unwrap()),
            },
        ],
    })
}

pub fn demo_capabilities() -> CapabilityContext {
    CapabilityContext::from_grants(
        [
            "apps.discover",
            "apps.launch",
            "files.search",
            "files.metadata.read",
            "files.open",
            "notes.search",
            "notes.metadata.read",
            "notes.open",
            "activity.search",
            "activity.read",
            "actions.discover",
            "workspaces.read",
            "workspaces.open",
        ]
        .into_iter()
        .map(|capability| CapabilityId::new(capability).expect("static capability id is valid")),
    )
}

pub fn demo_search_providers() -> Vec<Arc<dyn SearchProvider>> {
    let registry: Arc<dyn AppRegistry> = Arc::new(demo_app_registry());
    demo_search_providers_for_registry(registry)
}

pub fn demo_search_providers_for_registry(
    registry: Arc<dyn AppRegistry>,
) -> Vec<Arc<dyn SearchProvider>> {
    let catalog = LocalizationCatalog;
    vec![
        Arc::new(AppSearchProvider { registry, catalog }),
        fixture_provider(
            "files",
            20,
            "files.search",
            vec![
                fixture_candidate(
                    SearchIdentity::Object(ObjectId(1002)),
                    SearchCategory::Files,
                    "Search Workstream.md",
                    Some("Home and Search implementation scope"),
                    Some("This in-memory file fixture describes provider contracts, cancellation, and ranking."),
                    &["home", "search", "workstream"],
                    Some("files.metadata.read"),
                    TypedAction::OpenObject {
                        object_id: ObjectId(1002),
                        app_id: AppId::from_identifier(b"com.nagi.files"),
                    },
                    Some("files.open"),
                    ActionAvailability::ComingSoon {
                        reason_key: "app.files.runtime_unavailable".to_owned(),
                    },
                    Some("Fixture preview only; this is not a Nagi filesystem object."),
                    Some(WorkspaceId(42)),
                ),
                fixture_candidate(
                    SearchIdentity::Object(ObjectId(1003)),
                    SearchCategory::Files,
                    "東京での日本語入力と検索.txt",
                    Some("Unicode sample"),
                    Some("Home and Search must preserve 日本語 and mixed English text."),
                    &["unicode", "日本語"],
                    Some("files.metadata.read"),
                    TypedAction::OpenObject {
                        object_id: ObjectId(1003),
                        app_id: AppId::from_identifier(b"com.nagi.files"),
                    },
                    Some("files.open"),
                    ActionAvailability::ComingSoon {
                        reason_key: "app.files.runtime_unavailable".to_owned(),
                    },
                    None,
                    Some(WorkspaceId(42)),
                ),
            ],
        ),
        fixture_provider(
            "notes",
            25,
            "notes.search",
            vec![
                fixture_candidate(
                    SearchIdentity::Object(ObjectId(1001)),
                    SearchCategory::Notes,
                    "Home 検索設計メモ",
                    Some("Search provider model · 2026-09-26"),
                    Some("App registry, Workspace entry, deterministic ranking, and permission filtering."),
                    &["home", "search", "design", "検索"],
                    Some("notes.metadata.read"),
                    TypedAction::OpenObject {
                        object_id: ObjectId(1001),
                        app_id: AppId::from_identifier(b"com.nagi.notes"),
                    },
                    Some("notes.open"),
                    ActionAvailability::ComingSoon {
                        reason_key: "app.notes.runtime_unavailable".to_owned(),
                    },
                    Some("Fixture preview only; no real Notes storage is connected."),
                    Some(WorkspaceId(42)),
                ),
                fixture_candidate(
                    SearchIdentity::Object(ObjectId(1004)),
                    SearchCategory::Notes,
                    "Confidential rehearsal plan",
                    Some("Private fixture"),
                    Some("This candidate must remain invisible without notes.private.read."),
                    &["private", "secret"],
                    Some("notes.private.read"),
                    TypedAction::OpenObject {
                        object_id: ObjectId(1004),
                        app_id: AppId::from_identifier(b"com.nagi.notes"),
                    },
                    Some("notes.open"),
                    ActionAvailability::ComingSoon {
                        reason_key: "app.notes.runtime_unavailable".to_owned(),
                    },
                    None,
                    Some(WorkspaceId(42)),
                ),
            ],
        ),
        fixture_provider(
            "activity",
            10,
            "activity.search",
            vec![fixture_candidate(
                SearchIdentity::Object(ObjectId(2001)),
                SearchCategory::Activity,
                "Workspace resumed",
                Some("Activity · today"),
                Some("A mock event says the Home + Search Design workspace was resumed."),
                &["workspace", "resume", "home"],
                Some("activity.read"),
                TypedAction::InvokeAction {
                    action_id: "activity.open_record".to_owned(),
                },
                Some("activity.open"),
                ActionAvailability::ComingSoon {
                    reason_key: "app.activity.runtime_unavailable".to_owned(),
                },
                None,
                Some(WorkspaceId(42)),
            )],
        ),
        fixture_provider(
            "actions",
            5,
            "actions.discover",
            vec![fixture_candidate(
                SearchIdentity::Action("search.query".to_owned()),
                SearchCategory::Actions,
                "Search workspace resources",
                Some("Action · search.query"),
                Some("A typed, read-only Action contract for finding permitted resources."),
                &["search", "workspace", "resources"],
                None,
                TypedAction::InvokeAction {
                    action_id: "search.query".to_owned(),
                },
                Some("actions.execute"),
                ActionAvailability::ComingSoon {
                    reason_key: "actions.runtime_unavailable".to_owned(),
                },
                None,
                Some(WorkspaceId(42)),
            )],
        ),
        fixture_provider(
            "workspaces",
            15,
            "workspaces.read",
            vec![fixture_candidate(
                SearchIdentity::Workspace(WorkspaceId(42)),
                SearchCategory::Workspaces,
                "Home + Search Design",
                Some("Workspace · 4 related objects"),
                Some("A fixture workspace entry shared with the Home projection."),
                &["home", "search", "design"],
                Some("workspaces.read"),
                TypedAction::OpenWorkspace {
                    workspace_id: WorkspaceId(42),
                },
                Some("workspaces.open"),
                ActionAvailability::HostPreviewOnly,
                Some("The preview references WorkspaceId(42); it does not persist a workspace."),
                Some(WorkspaceId(42)),
            )],
        ),
    ]
}

pub fn create_preview_controller() -> Arc<HomeController> {
    let registry: Arc<dyn AppRegistry> = Arc::new(demo_app_registry());
    let home_source: Arc<dyn HomeDataSource> = Arc::new(demo_home_source());
    let providers = demo_search_providers_for_registry(Arc::clone(&registry));
    Arc::new(HomeController::new(
        registry,
        home_source,
        Arc::new(SearchCoordinator::new(Duration::from_millis(150), 40)),
        providers,
        true,
    ))
}

pub struct AppSearchProvider {
    registry: Arc<dyn AppRegistry>,
    catalog: LocalizationCatalog,
}

impl SearchProvider for AppSearchProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: ProviderId::new("apps").expect("static provider id is valid"),
            priority: 30,
            required_capability: Some(
                CapabilityId::new("apps.discover").expect("static capability is valid"),
            ),
        }
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchCandidate>, ProviderError> {
        if query.cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let entries = self.registry.list().map_err(|error| match error {
            RegistryError::BackendUnavailable => ProviderError::Unavailable,
            RegistryError::DuplicateAppId(_) | RegistryError::InvalidDescriptor(_) => {
                ProviderError::InvalidData
            }
        })?;
        Ok(entries
            .into_iter()
            .map(|app| SearchCandidate {
                identity: SearchIdentity::App(app.app_id),
                category: SearchCategory::Apps,
                text: SearchText {
                    title: app.display_name(&self.catalog, query.locale),
                    subtitle: Some(app.description(&self.catalog, query.locale)),
                    content: None,
                    tags: Vec::new(),
                },
                modified_at_unix_seconds: None,
                workspace_ids: Vec::new(),
                visibility_capability: None,
                action: Some(SearchAction {
                    action: app.launch_action,
                    required_capability: app.launch_capability.clone(),
                    availability: app.availability.action_state(),
                }),
                preview: None,
                is_fixture: true,
            })
            .collect())
    }
}

struct FixtureSearchProvider {
    descriptor: ProviderDescriptor,
    candidates: Vec<SearchCandidate>,
}

impl SearchProvider for FixtureSearchProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchCandidate>, ProviderError> {
        if query.cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        Ok(self.candidates.clone())
    }
}

fn fixture_provider(
    id: &str,
    priority: i16,
    capability: &str,
    candidates: Vec<SearchCandidate>,
) -> Arc<dyn SearchProvider> {
    Arc::new(FixtureSearchProvider {
        descriptor: ProviderDescriptor {
            id: ProviderId::new(id).expect("static provider id is valid"),
            priority,
            required_capability: Some(
                CapabilityId::new(capability).expect("static capability id is valid"),
            ),
        },
        candidates,
    })
}

#[allow(clippy::too_many_arguments)]
fn fixture_candidate(
    identity: SearchIdentity,
    category: SearchCategory,
    title: &str,
    subtitle: Option<&str>,
    content: Option<&str>,
    tags: &[&str],
    visibility_capability: Option<&str>,
    action: TypedAction,
    action_capability: Option<&str>,
    availability: ActionAvailability,
    preview: Option<&str>,
    workspace_id: Option<WorkspaceId>,
) -> SearchCandidate {
    SearchCandidate {
        identity,
        category,
        text: SearchText {
            title: title.to_owned(),
            subtitle: subtitle.map(str::to_owned),
            content: content.map(str::to_owned),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        },
        modified_at_unix_seconds: Some(1_800_000_000 - 600),
        workspace_ids: workspace_id.into_iter().collect(),
        visibility_capability: visibility_capability
            .map(|capability| CapabilityId::new(capability).expect("static capability id")),
        action: Some(SearchAction {
            action,
            required_capability: action_capability
                .map(|capability| CapabilityId::new(capability).expect("static capability id")),
            availability,
        }),
        preview: preview.map(str::to_owned),
        is_fixture: true,
    }
}

pub fn app_availability_label(app: &AppDescriptor, locale: Locale) -> String {
    LocalizationCatalog.resolve(app.availability.message_key(), locale)
}

pub fn normalize_fixture(value: &str) -> String {
    normalize_search_text(value)
}

#[cfg(test)]
mod tests {
    use crate::localization::{Locale, LocalizationCatalog};
    use crate::registry::{AppAvailability, AppRegistry};
    use crate::search::{SearchContext, SearchCoordinator, SearchQuery};
    use crate::{fixtures, SearchRequestId};
    use std::time::Duration;

    #[test]
    fn first_party_apps_are_data_driven_and_unavailable_apps_are_not_fake_ready() {
        let registry = fixtures::demo_app_registry();
        let apps = registry.list().unwrap();
        assert_eq!(apps.len(), 8);
        assert!(matches!(
            apps[0].availability,
            AppAvailability::Unavailable { .. }
        ));
        assert!(matches!(apps[1].availability, AppAvailability::ComingSoon));
        assert!(apps[6].availability.is_launchable());
        assert!(apps[7].availability.is_launchable());
    }

    #[test]
    fn app_provider_localizes_names_and_retains_typed_launch_identity() {
        let providers = fixtures::demo_search_providers();
        let apps = providers
            .iter()
            .find(|provider| provider.descriptor().id.as_str() == "apps")
            .unwrap();
        let response = SearchCoordinator::new(Duration::from_millis(100), 20)
            .search(
                SearchQuery::new(
                    "検索",
                    SearchRequestId(1),
                    SearchContext::default(),
                    fixtures::demo_capabilities(),
                    Locale::JaJp,
                ),
                std::slice::from_ref(apps),
            )
            .unwrap();
        let search_app = response
            .results
            .iter()
            .find(|result| result.title == "検索")
            .unwrap();
        assert_eq!(
            search_app.action,
            Some(crate::TypedAction::LaunchApp {
                app_id: nagi_model::AppId::from_identifier(b"com.nagi.search")
            })
        );
        assert_eq!(
            LocalizationCatalog.resolve("availability.coming_soon", Locale::JaJp),
            "開発予定"
        );
    }

    #[test]
    fn file_and_note_fixtures_return_typed_open_object_actions() {
        let providers = fixtures::demo_search_providers();
        let service = SearchCoordinator::new(Duration::from_millis(100), 40);

        for (request_id, query, expected_object, expected_app) in [
            (
                2,
                "Workstream",
                nagi_model::ObjectId(1002),
                nagi_model::AppId::from_identifier(b"com.nagi.files"),
            ),
            (
                3,
                "検索設計",
                nagi_model::ObjectId(1001),
                nagi_model::AppId::from_identifier(b"com.nagi.notes"),
            ),
        ] {
            let response = service
                .search(
                    SearchQuery::new(
                        query,
                        SearchRequestId(request_id),
                        SearchContext::default(),
                        fixtures::demo_capabilities(),
                        Locale::JaJp,
                    ),
                    &providers,
                )
                .unwrap();
            assert!(response.results.iter().any(|result| {
                result.action
                    == Some(crate::TypedAction::OpenObject {
                        object_id: expected_object,
                        app_id: expected_app,
                    })
            }));
        }
    }
}
