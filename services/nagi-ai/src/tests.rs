use alloc::{
    collections::BTreeSet,
    format,
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::cell::Cell;

use nagi_model::{AppId, AppSessionId, NodeId, ObjectId};
use nagi_model_manager::{
    BackendId, CancellationToken, CapabilityId, GenerativeProvider, ModelId, ModelRequest,
    ModelResponse, ModelStreamResponse, ProviderId, RuntimeError, TokenUsage,
};
use nagi_search::{
    AccessContext, BackendError, MetadataRecord, ObjectKind, SearchService, SnapshotBackend,
    VisibilityFilter, VisibilityScope, Workspace,
};
use serde_json::json;

use crate::{
    execute_plan, register_file_search_action, route_decision_candidate,
    route_with_decision_provider, validate_plan, ActionDescriptor, ActionHandler, ActionInvocation,
    ActionOutput, ActionPolicy, ActionRegistry, CallerIdentity, ContextAuthority, ContextRequest,
    ContextResolver, DecisionCandidate, DecisionProvider, DecisionRequest, DecisionRoute,
    ExecutionStatus, FallbackRoute, GenerativePlanProvider, HandlerError, LlmDecisionAdapter,
    ModelManagerPlanAdapter, NagiPlan, ObjectAccess, ParameterKind, ParameterRule, PlanPrompt,
    PlanProviderError, Planner, PlannerError, PolicyDenied, ResolvedContext, ValidationError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TestGrant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TestObjectHandle(ObjectId);

#[derive(Default)]
struct TestPolicy {
    denied_capabilities: BTreeSet<String>,
    denied_objects: BTreeSet<ObjectId>,
    denied_writes: BTreeSet<ObjectId>,
}

impl ActionPolicy for TestPolicy {
    type CapabilityGrant = TestGrant;
    type ObjectHandle = TestObjectHandle;

    fn check_capability(
        &self,
        _caller: CallerIdentity,
        capability: &CapabilityId,
    ) -> Result<(), PolicyDenied> {
        if self.denied_capabilities.contains(capability.as_str()) {
            Err(PolicyDenied::Capability)
        } else {
            Ok(())
        }
    }

    fn check_object_access(
        &self,
        _caller: CallerIdentity,
        object_id: ObjectId,
        access: ObjectAccess,
    ) -> Result<(), PolicyDenied> {
        if self.denied_objects.contains(&object_id)
            || (access == ObjectAccess::Modify && self.denied_writes.contains(&object_id))
        {
            Err(PolicyDenied::Object)
        } else {
            Ok(())
        }
    }

    fn acquire_capability(
        &self,
        caller: CallerIdentity,
        capability: &CapabilityId,
    ) -> Result<Self::CapabilityGrant, PolicyDenied> {
        self.check_capability(caller, capability)?;
        Ok(TestGrant)
    }

    fn resolve_object(
        &self,
        caller: CallerIdentity,
        object_id: ObjectId,
        access: ObjectAccess,
    ) -> Result<Self::ObjectHandle, PolicyDenied> {
        self.check_object_access(caller, object_id, access)?;
        Ok(TestObjectHandle(object_id))
    }
}

struct TestContextAuthority {
    visible: BTreeSet<ObjectId>,
}

impl ContextAuthority for TestContextAuthority {
    fn can_read_object(&self, _caller: CallerIdentity, object_id: ObjectId) -> bool {
        self.visible.contains(&object_id)
    }
}

struct TestHandler {
    calls: Rc<Cell<usize>>,
    result: Result<ActionOutput, HandlerError>,
}

impl ActionHandler<TestPolicy> for TestHandler {
    fn execute(
        &mut self,
        invocation: ActionInvocation<'_, TestPolicy>,
    ) -> Result<ActionOutput, HandlerError> {
        assert_eq!(invocation.capability_grants().len(), 1);
        assert_eq!(
            invocation.object_handles().len(),
            invocation.object_ids().len()
        );
        for (id, handle) in invocation
            .object_ids()
            .iter()
            .zip(invocation.object_handles())
        {
            assert_eq!(*id, handle.0);
        }
        self.calls.set(self.calls.get() + 1);
        match &self.result {
            Ok(output) => Ok(ActionOutput {
                summary: output.summary.clone(),
                object_ids: output.object_ids.clone(),
            }),
            Err(error) => Err(*error),
        }
    }
}

struct TestPlanProvider(Result<String, PlanProviderError>);

impl GenerativePlanProvider for TestPlanProvider {
    fn generate_complete_plan(
        &mut self,
        prompt: &PlanPrompt,
        _cancellation: &dyn CancellationToken,
    ) -> Result<String, PlanProviderError> {
        assert!(!prompt.action_schemas().is_empty());
        self.0.clone()
    }
}

struct NotCancelled;

impl CancellationToken for NotCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

struct TestGenerativeProvider {
    model_id: ModelId,
    response: Result<String, RuntimeError>,
}

impl GenerativeProvider for TestGenerativeProvider {
    fn model_id(&self) -> &ModelId {
        &self.model_id
    }

    fn generate(
        &mut self,
        request: &ModelRequest<'_>,
        _cancellation: &dyn CancellationToken,
    ) -> Result<ModelResponse, RuntimeError> {
        let text = self.response.clone()?;
        Ok(ModelResponse {
            request_id: request.request_id,
            model_id: self.model_id.clone(),
            provider_id: ProviderId::new("test-provider").expect("provider ID"),
            backend_id: BackendId::new("test-backend").expect("backend ID"),
            text,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 4,
            },
        })
    }

    fn generate_stream(
        &mut self,
        _request: &ModelRequest<'_>,
        _cancellation: &dyn CancellationToken,
        _sink: &mut dyn nagi_model_manager::TextChunkSink,
    ) -> Result<ModelStreamResponse, RuntimeError> {
        Err(RuntimeError::UnsupportedCapability)
    }
}

fn caller() -> CallerIdentity {
    CallerIdentity {
        app_id: AppId(1),
        app_session_id: AppSessionId(2),
        node_id: NodeId(3),
        workspace_id: None,
    }
}

fn context(objects: &[u64]) -> ResolvedContext {
    let ids = objects.iter().copied().map(ObjectId).collect::<Vec<_>>();
    let authority = TestContextAuthority {
        visible: ids.iter().copied().collect(),
    };
    ContextResolver
        .resolve(
            ContextRequest {
                caller: caller(),
                selected_object: None,
                candidate_objects: ids,
            },
            &authority,
        )
        .expect("resolve visible context")
}

fn capability(name: &str) -> CapabilityId {
    CapabilityId::new(name).expect("valid capability ID")
}

fn search_descriptor() -> ActionDescriptor {
    ActionDescriptor::new(
        "file.search",
        vec![capability("files.search")],
        ObjectAccess::Read,
        0,
        0,
        vec![ParameterRule::new(
            "query",
            ParameterKind::String { max_bytes: 128 },
            true,
        )],
    )
    .expect("valid file.search descriptor")
}

fn move_descriptor() -> ActionDescriptor {
    ActionDescriptor::new(
        "file.move",
        vec![capability("files.move")],
        ObjectAccess::Modify,
        2,
        2,
        vec![],
    )
    .expect("valid file.move descriptor")
}

fn handler(calls: &Rc<Cell<usize>>, summary: &str, object_ids: Vec<ObjectId>) -> TestHandler {
    TestHandler {
        calls: calls.clone(),
        result: Ok(ActionOutput {
            summary: summary.to_string(),
            object_ids,
        }),
    }
}

fn plan(json: &str) -> NagiPlan {
    NagiPlan::parse_complete(json).expect("complete NagiPlan@1 JSON")
}

#[test]
fn orchestration_accepts_only_complete_strict_nagi_plan_v1_json() {
    let valid = r#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","parameters":{"query":"budget"}}]}"#;
    assert!(NagiPlan::parse_complete(valid).is_ok());
    assert_eq!(
        NagiPlan::parse_complete(&format!("{valid} {{}}")),
        Err(crate::PlanParseError::InvalidJson)
    );
    assert_eq!(
        NagiPlan::parse_complete(
            r#"{"plan_version":1,"intent":"x","steps":[],"shell":"echo unsafe"}"#
        ),
        Err(crate::PlanParseError::InvalidJson)
    );
    let oversized = alloc::string::String::from_utf8(vec![b' '; crate::MAX_PLAN_JSON_BYTES + 1])
        .expect("valid UTF-8 fixture");
    assert_eq!(
        NagiPlan::parse_complete(&oversized),
        Err(crate::PlanParseError::TooLarge)
    );
}

#[test]
fn context_resolver_filters_hidden_objects_before_provider_context() {
    let authority = TestContextAuthority {
        visible: [ObjectId(1)].into_iter().collect(),
    };
    let resolved = ContextResolver
        .resolve(
            ContextRequest {
                caller: caller(),
                selected_object: None,
                candidate_objects: vec![ObjectId(1), ObjectId(2)],
            },
            &authority,
        )
        .expect("resolve context");
    assert_eq!(resolved.visible_objects(), [ObjectId(1)]);
}

#[test]
fn context_resolver_rejects_duplicate_candidates() {
    let authority = TestContextAuthority {
        visible: [ObjectId(1)].into_iter().collect(),
    };
    let result = ContextResolver.resolve(
        ContextRequest {
            caller: caller(),
            selected_object: None,
            candidate_objects: vec![ObjectId(1), ObjectId(1)],
        },
        &authority,
    );
    assert_eq!(result, Err(crate::ContextError::DuplicateObject));
}

#[test]
fn validator_rejects_unknown_action_and_never_runs_another_action() {
    let calls = Rc::new(Cell::new(0));
    let mut registry = ActionRegistry::new();
    registry
        .register(search_descriptor(), handler(&calls, "searched", vec![]))
        .expect("register action");
    let unknown = plan(r#"{"plan_version":1,"intent":"test","steps":[{"action":"app.launch"}]}"#);
    let result = validate_plan(unknown, &context(&[]), &registry, &TestPolicy::default());
    assert_eq!(result.err(), Some(ValidationError::UnsupportedAction));
    assert_eq!(calls.get(), 0);
}

#[test]
fn validator_denies_capability_before_any_step_executes() {
    let calls = Rc::new(Cell::new(0));
    let mut registry = ActionRegistry::new();
    registry
        .register(search_descriptor(), handler(&calls, "searched", vec![]))
        .expect("register action");
    let denied = TestPolicy {
        denied_capabilities: ["files.search".to_string()].into_iter().collect(),
        ..TestPolicy::default()
    };
    let parsed = plan(
        r#"{"plan_version":1,"intent":"test","steps":[{"action":"file.search","parameters":{"query":"budget"}}]}"#,
    );
    assert_eq!(
        validate_plan(parsed, &context(&[]), &registry, &denied).err(),
        Some(ValidationError::CapabilityDenied)
    );
    assert_eq!(calls.get(), 0);
}

#[test]
fn validator_denies_invisible_and_out_of_context_object_ids() {
    let mut registry = ActionRegistry::new();
    registry
        .register(
            move_descriptor(),
            handler(&Rc::new(Cell::new(0)), "moved", vec![]),
        )
        .expect("register action");
    let parsed = plan(
        r#"{"plan_version":1,"intent":"move","steps":[{"action":"file.move","object_ids":[1,2]}]}"#,
    );
    assert_eq!(
        validate_plan(parsed, &context(&[1]), &registry, &TestPolicy::default()).err(),
        Some(ValidationError::ObjectOutsideContext)
    );
    let policy = TestPolicy {
        denied_writes: [ObjectId(2)].into_iter().collect(),
        ..TestPolicy::default()
    };
    assert_eq!(
        validate_plan(
            plan(r#"{"plan_version":1,"intent":"move","steps":[{"action":"file.move","object_ids":[1,2]}]}"#),
            &context(&[1, 2]),
            &registry,
            &policy
        )
        .err(),
        Some(ValidationError::ObjectDenied)
    );
}

#[test]
fn validator_rejects_path_injection_and_out_of_range_parameters() {
    assert_eq!(
        ActionDescriptor::new(
            "file.move",
            vec![capability("files.move")],
            ObjectAccess::Modify,
            2,
            2,
            vec![ParameterRule::new(
                "path",
                ParameterKind::String { max_bytes: 256 },
                true
            )],
        )
        .err(),
        Some(crate::RegistryError::InvalidParameterName)
    );

    let mut registry = ActionRegistry::new();
    registry
        .register(
            search_descriptor(),
            handler(&Rc::new(Cell::new(0)), "searched", vec![]),
        )
        .expect("register action");
    assert!(matches!(
        validate_plan(
            plan(
                r#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","parameters":{"query":""}}]}"#
            ),
            &context(&[]),
            &registry,
            &TestPolicy::default()
        ),
        Err(ValidationError::InvalidParameters(
            crate::ParameterError::InvalidValue
        ))
    ));
    assert!(matches!(
        validate_plan(
            plan(
                r#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","parameters":{"query":"budget","path":"/host/etc/passwd"}}]}"#
            ),
            &context(&[]),
            &registry,
            &TestPolicy::default()
        ),
        Err(ValidationError::InvalidParameters(
            crate::ParameterError::Unknown
        ))
    ));
}

#[test]
fn successful_search_action_returns_a_bounded_executor_result() {
    let calls = Rc::new(Cell::new(0));
    let mut registry = ActionRegistry::new();
    registry
        .register(
            search_descriptor(),
            handler(&calls, "2 visible matches", vec![ObjectId(9)]),
        )
        .expect("register action");
    let parsed = plan(
        r#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","parameters":{"query":"budget"}}]}"#,
    );
    let validated = validate_plan(parsed, &context(&[]), &registry, &TestPolicy::default())
        .expect("validate complete plan");
    let report = execute_plan(validated, &mut registry, &TestPolicy::default());
    assert_eq!(report.status, ExecutionStatus::Succeeded);
    assert_eq!(report.completed.len(), 1);
    assert_eq!(report.completed[0].summary, "2 visible matches");
    assert_eq!(report.completed[0].object_ids, [ObjectId(9)]);
    assert_eq!(calls.get(), 1);
}

#[test]
fn executor_reports_partial_failure_after_completed_steps() {
    let first_calls = Rc::new(Cell::new(0));
    let second_calls = Rc::new(Cell::new(0));
    let mut registry = ActionRegistry::new();
    registry
        .register(
            search_descriptor(),
            handler(&first_calls, "completed search", vec![]),
        )
        .expect("register first action");
    let failing = TestHandler {
        calls: second_calls.clone(),
        result: Err(HandlerError::Unavailable),
    };
    let settings = ActionDescriptor::new(
        "system.volume.set",
        vec![capability("system.volume.set")],
        ObjectAccess::Modify,
        0,
        0,
        vec![ParameterRule::new(
            "level",
            ParameterKind::Integer { min: 0, max: 100 },
            true,
        )],
    )
    .expect("valid volume descriptor");
    registry
        .register(settings, failing)
        .expect("register second action");
    let parsed = plan(
        r#"{"plan_version":1,"intent":"find then volume","steps":[{"action":"file.search","parameters":{"query":"budget"}},{"action":"system.volume.set","parameters":{"level":20}}]}"#,
    );
    let validated = validate_plan(parsed, &context(&[]), &registry, &TestPolicy::default())
        .expect("validate both steps before execution");
    let report = execute_plan(validated, &mut registry, &TestPolicy::default());
    assert_eq!(report.status, ExecutionStatus::Partial);
    assert_eq!(report.completed.len(), 1);
    assert_eq!(report.failed_step, Some(1));
    assert_eq!(
        report.error,
        Some(crate::ExecutionError::Handler(HandlerError::Unavailable))
    );
    assert_eq!((first_calls.get(), second_calls.get()), (1, 1));
}

#[test]
fn executor_rejects_results_that_name_hidden_objects() {
    let calls = Rc::new(Cell::new(0));
    let mut registry = ActionRegistry::new();
    registry
        .register(
            search_descriptor(),
            handler(&calls, "unexpected hidden id", vec![ObjectId(404)]),
        )
        .expect("register action");
    let parsed = plan(
        r#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","parameters":{"query":"private"}}]}"#,
    );
    let validated = validate_plan(parsed, &context(&[]), &registry, &TestPolicy::default())
        .expect("validate plan");
    let policy = TestPolicy {
        denied_objects: [ObjectId(404)].into_iter().collect(),
        ..TestPolicy::default()
    };
    let report = execute_plan(validated, &mut registry, &policy);
    assert_eq!(report.status, ExecutionStatus::Failed);
    assert_eq!(report.error, Some(crate::ExecutionError::ObjectDenied));
}

#[test]
fn decision_confidence_and_provider_output_never_authorize_actions() {
    let allowed = vec!["file.search".to_string()];
    let route = route_decision_candidate(
        &allowed,
        Some(DecisionCandidate {
            action_id: "file.move".to_string(),
            confidence_percent: 100,
        }),
        50,
        false,
    )
    .expect("bounded route");
    assert_eq!(
        route,
        DecisionRoute::Fallback(FallbackRoute::DeterministicOrManual)
    );
    let selected = route_decision_candidate(
        &allowed,
        Some(DecisionCandidate {
            action_id: "file.search".to_string(),
            confidence_percent: 99,
        }),
        50,
        true,
    )
    .expect("bounded route");
    assert_eq!(
        selected,
        DecisionRoute::Candidate("file.search".to_string())
    );
}

#[test]
fn validator_enforces_schema_version_plan_size_and_action_parameters() {
    let mut registry = ActionRegistry::new();
    registry
        .register(
            search_descriptor(),
            handler(&Rc::new(Cell::new(0)), "searched", vec![]),
        )
        .expect("register action");
    let wrong_version = plan(
        r#"{"plan_version":2,"intent":"search","steps":[{"action":"file.search","parameters":{"query":"x"}}]}"#,
    );
    assert_eq!(
        validate_plan(
            wrong_version,
            &context(&[]),
            &registry,
            &TestPolicy::default()
        )
        .err(),
        Some(ValidationError::UnsupportedVersion)
    );
    let unknown_parameter = plan(
        r#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","parameters":{"query":"x","extra":true}}]}"#,
    );
    assert_eq!(
        validate_plan(
            unknown_parameter,
            &context(&[]),
            &registry,
            &TestPolicy::default()
        )
        .err(),
        Some(ValidationError::InvalidParameters(
            crate::ParameterError::Unknown
        ))
    );
}

#[test]
fn schema_fixtures_are_valid_json_and_bound_action_plan_shape() {
    let schema = include_str!("../../../schemas/NagiPlan@1.json");
    let parsed: serde_json::Value = serde_json::from_str(schema).expect("valid schema JSON");
    assert_eq!(parsed["title"], json!("NagiPlan@1"));
    assert_eq!(parsed["properties"]["plan_version"]["const"], json!(1));
    assert_eq!(parsed["properties"]["steps"]["maxItems"], json!(16));
}

#[test]
fn planner_sends_only_filtered_action_schemas_and_rejects_incomplete_provider_output() {
    let calls = Rc::new(Cell::new(0));
    let mut registry = ActionRegistry::new();
    registry
        .register(search_descriptor(), handler(&calls, "searched", vec![]))
        .expect("register search action");
    let settings = ActionDescriptor::new(
        "system.volume.set",
        vec![capability("system.volume.set")],
        ObjectAccess::Modify,
        0,
        0,
        vec![ParameterRule::new(
            "level",
            ParameterKind::Integer { min: 0, max: 100 },
            true,
        )],
    )
    .expect("volume action descriptor");
    registry
        .register(settings, handler(&calls, "volume set", vec![]))
        .expect("register volume action");

    let planner = Planner;
    let prompt = planner
        .prepare(
            55,
            "find my budget file".to_string(),
            context(&[]),
            &["file.search".to_string()],
            &registry,
        )
        .expect("prepare bounded prompt");
    assert_eq!(prompt.action_schemas().len(), 1);
    assert_eq!(prompt.action_schemas()[0].action_id, "file.search");
    assert_eq!(prompt.user_intent(), "find my budget file");

    let mut provider = TestPlanProvider(Ok(
        r#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","parameters":{"query":"budget"}}]}"#.to_string(),
    ));
    let candidate = planner
        .generate(&mut provider, &prompt, &NotCancelled)
        .expect("complete provider plan");
    assert!(validate_plan(
        candidate,
        prompt.context(),
        &registry,
        &TestPolicy::default()
    )
    .is_ok());

    let model_manager_provider = TestGenerativeProvider {
        model_id: ModelId::new("test-model").expect("model ID"),
        response: Ok(r#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search","parameters":{"query":"budget"}}]}"#.to_string()),
    };
    let mut runtime_adapter = ModelManagerPlanAdapter::new(
        model_manager_provider,
        capability("structured.generate"),
        3_000,
    );
    assert!(planner
        .generate(&mut runtime_adapter, &prompt, &NotCancelled)
        .is_ok());

    let mut partial = TestPlanProvider(Ok(
        r#"{"plan_version":1,"intent":"search","steps":[{"action":"file.search"}]"#.to_string(),
    ));
    assert_eq!(
        planner.generate(&mut partial, &prompt, &NotCancelled).err(),
        Some(PlannerError::InvalidPlan)
    );

    let mut unavailable = TestPlanProvider(Err(PlanProviderError::Unavailable));
    assert_eq!(
        planner
            .generate(&mut unavailable, &prompt, &NotCancelled)
            .err(),
        Some(PlannerError::ProviderUnavailable)
    );
}

#[test]
fn llm_decision_adapter_is_bounded_and_returns_only_an_advisory_candidate() {
    let provider = TestGenerativeProvider {
        model_id: ModelId::new("test-model").expect("model ID"),
        response: Ok(r#"{"action_id":"file.search","confidence_percent":87}"#.to_string()),
    };
    let mut adapter = LlmDecisionAdapter::new(provider, 2_000);
    let capability = capability("decision.choice");
    let candidates = vec!["file.search".to_string()];
    let request = DecisionRequest {
        request_id: 88,
        caller: AppId(4),
        capability: &capability,
        intent: "find the budget file",
        candidate_action_ids: &candidates,
    };
    let candidate = adapter
        .decide(&request, &NotCancelled)
        .expect("parse bounded decision output");
    assert_eq!(
        candidate,
        DecisionCandidate {
            action_id: "file.search".to_string(),
            confidence_percent: 87
        }
    );
    assert_eq!(
        route_decision_candidate(&candidates, Some(candidate), 80, false).expect("route"),
        DecisionRoute::Candidate("file.search".to_string())
    );
    assert_eq!(
        route_with_decision_provider(None, &request, 80, true, &NotCancelled).expect("fallback"),
        DecisionRoute::Fallback(FallbackRoute::GenerativePlanner)
    );

    let bad_provider = TestGenerativeProvider {
        model_id: ModelId::new("test-model").expect("model ID"),
        response: Ok(r#"{"action_id":"file.move","confidence_percent":100}"#.to_string()),
    };
    let mut bad_adapter = LlmDecisionAdapter::new(bad_provider, 2_000);
    assert_eq!(
        bad_adapter.decide(&request, &NotCancelled).err(),
        Some(crate::DecisionProviderError::UnsupportedCandidate)
    );

    let failed_provider = TestGenerativeProvider {
        model_id: ModelId::new("test-model").expect("model ID"),
        response: Err(RuntimeError::BackendUnavailable),
    };
    let mut failed_adapter = LlmDecisionAdapter::new(failed_provider, 2_000);
    assert_eq!(
        route_with_decision_provider(
            Some(&mut failed_adapter),
            &request,
            80,
            false,
            &NotCancelled
        )
        .expect("safe failure fallback"),
        DecisionRoute::Fallback(FallbackRoute::DeterministicOrManual)
    );
}

#[derive(Default)]
struct SearchMemoryBackend(Option<Vec<u8>>);

impl SnapshotBackend for SearchMemoryBackend {
    fn load_snapshot(&mut self) -> Result<Option<Vec<u8>>, BackendError> {
        Ok(self.0.clone())
    }

    fn write_snapshot(&mut self, snapshot: &[u8]) -> Result<(), BackendError> {
        self.0 = Some(snapshot.to_vec());
        Ok(())
    }
}

struct CallerSearchVisibility;

impl VisibilityFilter for CallerSearchVisibility {
    fn can_read_object(&self, access: AccessContext, record: &MetadataRecord) -> bool {
        record.visibility == VisibilityScope::Public
            || (record.source_app == access.app_id
                && record.source_session == access.app_session_id)
    }

    fn can_read_workspace(&self, access: AccessContext, workspace: &Workspace) -> bool {
        workspace.visibility == VisibilityScope::Public || workspace.owner_app == access.app_id
    }
}

#[test]
fn registered_file_search_action_runs_search_and_returns_only_visible_ids() {
    let caller = caller();
    let mut service = SearchService::open(SearchMemoryBackend::default(), CallerSearchVisibility)
        .expect("open metadata search service");
    let mut visible = MetadataRecord::new(ObjectId(41), ObjectKind::File, "Servo notes");
    visible.source_app = Some(caller.app_id);
    visible.source_session = Some(caller.app_session_id);
    visible.visibility = VisibilityScope::Private;
    service.upsert_record(visible).expect("index caller file");
    let mut page = MetadataRecord::new(ObjectId(43), ObjectKind::Page, "Servo article");
    page.source_app = Some(caller.app_id);
    page.source_session = Some(caller.app_session_id);
    page.visibility = VisibilityScope::Private;
    service.upsert_record(page).expect("index caller page");
    let mut hidden = MetadataRecord::new(ObjectId(42), ObjectKind::File, "Servo private");
    hidden.source_app = Some(AppId(caller.app_id.0 + 1));
    hidden.visibility = VisibilityScope::Private;
    service
        .upsert_record(hidden)
        .expect("index another app file");

    let mut registry = ActionRegistry::new();
    register_file_search_action(&mut registry, service).expect("register file.search");
    let validated = validate_plan(
        plan(r#"{"plan_version":1,"intent":"find Servo notes","steps":[{"action":"file.search","parameters":{"query":"Servo"}}]}"#),
        &context(&[]),
        &registry,
        &TestPolicy::default(),
    )
    .expect("validate bounded search plan");
    let report = execute_plan(validated, &mut registry, &TestPolicy::default());

    assert_eq!(report.status, ExecutionStatus::Succeeded);
    assert_eq!(report.completed.len(), 1);
    assert_eq!(report.completed[0].summary, "Found 1 visible object(s).");
    assert_eq!(report.completed[0].object_ids, vec![ObjectId(41)]);
}
