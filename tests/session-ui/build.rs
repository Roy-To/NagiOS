use std::{env, fs, path::PathBuf};

#[path = "path_guard.rs"]
mod path_guard;

const HOST_TEST_HELPERS: &str = r#"
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HostTestSnapshot {
    pub user_token: Option<u64>,
    pub terms_confirmed: bool,
    pub active_request_id: Option<u64>,
    pub cancellation_requested: bool,
    pub queued: bool,
    pub has_reply: bool,
    pub last_request_id: u64,
    pub generation: u64,
}

#[cfg(test)]
pub(crate) fn host_test_bind_for_test(
    service: &SessionServices,
    user: &Session,
) -> Result<(), SubmitError> {
    service.shared.bind(user)
}

#[cfg(test)]
pub(crate) fn host_test_snapshot_for_test(service: &SessionServices) -> HostTestSnapshot {
    let state = service.shared.state.lock();
    HostTestSnapshot {
        user_token: state.user.map(|user| user.token()),
        terms_confirmed: state.terms_confirmed,
        active_request_id: state.active.as_ref().map(|slot| slot.request_id),
        cancellation_requested: state
            .active
            .as_ref()
            .is_some_and(|slot| slot.cancellation.load(Ordering::Acquire)),
        queued: state.queued.is_some(),
        has_reply: state.reply.is_some(),
        last_request_id: state.last_request_id,
        generation: service.shared.generation.load(Ordering::Acquire),
    }
}

#[cfg(test)]
struct HostScriptedBackend;

#[cfg(test)]
impl WorkerBackend for HostScriptedBackend {
    fn bind(_: &Session, _: u64, _: ResourceBudget) -> Result<Self, ReplyError> {
        Ok(Self)
    }

    fn acknowledge(&mut self, _: &Session, _: &str) -> Result<(), ReplyError> {
        Ok(())
    }

    fn generate(
        &mut self,
        job: &Job,
        cancellation: &dyn CancellationToken,
    ) -> Result<ModelResponse, ReplyError> {
        if cancellation.is_cancelled() {
            return Err(ReplyError::Model(RuntimeError::Cancelled.into()));
        }
        Ok(ModelResponse {
            request_id: job.request_id,
            model_id: nagi_model_manager::ModelId::new("fixture.model").unwrap(),
            provider_id: nagi_model_manager::ProviderId::new("fixture.provider").unwrap(),
            backend_id: nagi_model_manager::BackendId::new("fixture.backend").unwrap(),
            text: String::from("scripted close/reopen reply"),
            usage: nagi_model_manager::TokenUsage {
                input_tokens: 1,
                output_tokens: 1,
            },
        })
    }

    fn unload(&mut self) -> Result<(), ReplyError> {
        Ok(())
    }
}

/// Drive the real Shared queue/completion path with a deterministic host reply.
#[cfg(test)]
pub(crate) fn host_test_complete_queued_request(service: &SessionServices) -> bool {
    let mut worker = WorkerMachine::<HostScriptedBackend>::new();
    worker.step(
        &service.shared,
        1,
        ResourceBudget {
            available_ram_bytes: 1,
            available_storage_bytes: 1,
            cpu_cores: 1,
            gpu_available: false,
        },
        &service.terms_reference,
    )
}
"#;

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let repo_root = path_guard::expected_repository_root(&manifest_dir)
        .unwrap_or_else(|error| panic!("resolve expected repository root: {error}"));
    let is_desktop_host = path_guard::is_desktop_host_test_package(&manifest_dir)
        .unwrap_or_else(|error| panic!("identify host-test package: {error}"));
    println!("cargo:rustc-check-cfg=cfg(nagi_desktop_host_test)");
    if is_desktop_host {
        println!("cargo:rustc-cfg=nagi_desktop_host_test");
    }
    if is_desktop_host {
        // Compile the exact modal loop, without launching a signed guest child
        // or substituting a host implementation of its authority boundary.
        let path = path_guard::canonical_file_within_root(
            &repo_root,
            &repo_root.join("user/nagi-init/src/m19_files_client.rs"),
            "Files consent wait source",
        )
        .expect("validate Files consent wait source");
        println!("cargo:rerun-if-changed={}", path.display());
        let source = fs::read_to_string(path).expect("read Files consent wait source");
        let start = "fn wait_for_answer(";
        let end = "\nfn abort_and_reap(";
        assert_eq!(source.matches(start).count(), 1);
        assert_eq!(source.matches(end).count(), 1);
        let function = source
            .split_once(start)
            .unwrap()
            .1
            .split_once(end)
            .unwrap()
            .0;
        let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("files_consent_wait.rs");
        fs::write(output, format!("{start}{function}")).expect("write exact Files consent wait");
    }
    let source_path = path_guard::canonical_file_within_root(
        &repo_root,
        &repo_root.join("user/nagi-init/src/session_services.rs"),
        "SessionServices source",
    )
    .unwrap_or_else(|error| panic!("validate SessionServices source: {error}"));
    println!("cargo:rerun-if-changed={}", source_path.display());
    let source = fs::read_to_string(&source_path).expect("read frozen SessionServices source");
    // SessionServices is copied into OUT_DIR, so rebase its fixture include only
    // in this host-test copy and leave the frozen production source untouched.
    const MODEL_FIXTURE_SOURCE_PATH: &str =
        r#""../../nagi-model-manager/tests/fixtures/granite-4.2-3b.json""#;
    let fixture_path = path_guard::canonical_file_within_root(
        &repo_root,
        &repo_root.join("user/nagi-model-manager/tests/fixtures/granite-4.2-3b.json"),
        "model manager fixture",
    )
    .unwrap_or_else(|error| panic!("validate model manager fixture: {error}"));
    println!("cargo:rerun-if-changed={}", fixture_path.display());
    println!(
        "cargo:rustc-env=NAGI_MODEL_MANAGER_FIXTURE_PATH={}",
        fixture_path.display()
    );
    assert_eq!(
        source.matches(MODEL_FIXTURE_SOURCE_PATH).count(),
        1,
        "expected one frozen model manager fixture include",
    );
    let source = source.replace(
        MODEL_FIXTURE_SOURCE_PATH,
        r#"env!("NAGI_MODEL_MANAGER_FIXTURE_PATH")"#,
    );
    let materialized = source
        .lines()
        .map(|line| {
            let indent = line.len() - line.trim_start().len();
            let body = &line[indent..];
            match body.strip_prefix("//!") {
                Some(doc) => format!("{}//{}", " ".repeat(indent), doc),
                None => line.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("build output dir"));
    let output = out_dir.join("session_services.rs");
    let host_test_helpers = if is_desktop_host {
        HOST_TEST_HELPERS
    } else {
        ""
    };
    fs::write(output, format!("{materialized}\n{host_test_helpers}"))
        .expect("write host SessionServices module");
}
