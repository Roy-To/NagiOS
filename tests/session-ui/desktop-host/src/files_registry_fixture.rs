//! Orchestration fixture only: does not claim signed package launch coverage.
use super::*;
pub(crate) fn launch() -> Launched {
    let app = AppId::from_identifier(b"org.nagi.files");
    with_registry(|registry| {
        *registry = LaunchRegistry::new();
        registry
            .register_manifest(
                AppManifest::from_declaration(
                    b"org.nagi.files",
                    [b"search.query".as_slice(), b"files.search".as_slice()],
                )
                .unwrap(),
            )
            .unwrap();
        let user = acceptance_user().unwrap();
        for grant in [b"search.query".as_slice(), b"files.search".as_slice()] {
            registry
                .record_decision(&user, app, grant, GrantDecision::Allow)
                .unwrap();
        }
        Launched {
            endpoint: 77,
            record: registry
                .record_launch(
                    7,
                    app,
                    LaunchPlacement {
                        app_session_id: nagi_model::AppSessionId(33),
                        node_id: nagi_model::NodeId(44),
                        workspace_id: None,
                    },
                )
                .unwrap(),
        }
    })
    .unwrap()
}
