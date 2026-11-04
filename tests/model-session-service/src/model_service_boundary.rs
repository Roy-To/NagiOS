//! Host-only tests of the exact production wrapper. No syscall/native/model run.
use super::*;
use ::libnagi::security::{AccountStore, Role};
use alloc::{string::String, vec::Vec};
use core::cell::Cell;
use nagi_ai::{
    BrowserContextApiError, BrowserContextRequest, BrowserPageSnapshot, CallerIdentity,
    ContextRequest, NAGI_BAR_APP_ID,
};
use nagi_model::{AppId, AppSessionId, NodeId, ObjectId, WorkspaceId};
use nagi_model_manager::{ModelServiceError, StoreError};

pub(super) mod syscalls {
    pub use ::libnagi::security;
    pub fn time_ticks() -> u64 {
        panic!("unexpected host guest-clock access")
    }
    pub fn thread_yield() -> bool {
        panic!("unexpected host guest yield")
    }
    pub fn block_read(_: u64, _: u64, _: &mut [u8; 512]) -> bool {
        panic!("unexpected model artifact read")
    }
}
struct Never;
impl CancellationToken for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}
struct Authority;
impl nagi_ai::ContextAuthority for Authority {
    fn can_read_object(&self, _: CallerIdentity, _: ObjectId) -> bool {
        false
    }
}
struct Browser {
    policy: crate::albert_context_policy::ContextSharingPolicy,
    calls: Cell<usize>,
}
impl nagi_ai::PublicBrowserContextApi for Browser {
    fn current_page_context(
        &mut self,
        request: BrowserContextRequest,
    ) -> Result<Option<BrowserPageSnapshot>, BrowserContextApiError> {
        self.calls.set(self.calls.get() + 1);
        self.policy.check(request.caller())?;
        Ok(Some(BrowserPageSnapshot::new(
            1,
            None,
            None,
            None,
            Some(String::from("synthetic page data")),
        )))
    }
}
fn actor() -> CallerIdentity {
    CallerIdentity {
        app_id: NAGI_BAR_APP_ID,
        app_session_id: AppSessionId(8),
        node_id: NodeId(9),
        workspace_id: None,
    }
}
fn request(caller: CallerIdentity) -> ContextRequest {
    ContextRequest {
        caller,
        selected_object: None,
        candidate_objects: Vec::new(),
    }
}
fn setup() -> (Session, Session, SessionModelService, Browser) {
    let mut accounts = AccountStore::new();
    accounts
        .add_account(b"fixture", Role::Owner, b"public test password")
        .unwrap();
    let user = accounts
        .authenticate(b"fixture", b"public test password")
        .unwrap();
    let other = accounts
        .authenticate(b"fixture", b"public test password")
        .unwrap();
    let service = SessionModelService::new(
        &user,
        42,
        ResourceBudget {
            available_ram_bytes: 3072 * 1024 * 1024,
            available_storage_bytes: 2560 * 1024 * 1024,
            cpu_cores: 1,
            gpu_available: false,
        },
    )
    .unwrap();
    (
        user,
        other,
        service,
        Browser {
            policy: crate::albert_context_policy::ContextSharingPolicy::disabled(),
            calls: Cell::new(0),
        },
    )
}
#[test]
fn actual_wrapper_denies_other_and_locked_session_before_context() {
    let (mut user, other, mut service, mut browser) = setup();
    assert_eq!(
        service.summarize_page(
            &other,
            actor(),
            1,
            request(actor()),
            &Authority,
            &mut browser,
            &Never
        ),
        Err(SessionPageSummaryError::Denied)
    );
    user.lock();
    assert_eq!(
        service.summarize_page(
            &user,
            actor(),
            1,
            request(actor()),
            &Authority,
            &mut browser,
            &Never
        ),
        Err(SessionPageSummaryError::Denied)
    );
    assert_eq!(browser.calls.get(), 0);
}
#[test]
fn actual_wrapper_denies_forged_full_identity_and_non_assistant_actor() {
    let (user, _, mut service, mut browser) = setup();
    for caller in [
        CallerIdentity {
            app_session_id: AppSessionId(99),
            ..actor()
        },
        CallerIdentity {
            node_id: NodeId(99),
            ..actor()
        },
        CallerIdentity {
            workspace_id: Some(WorkspaceId(99)),
            ..actor()
        },
    ] {
        assert_eq!(
            service.summarize_page(
                &user,
                actor(),
                1,
                request(caller),
                &Authority,
                &mut browser,
                &Never
            ),
            Err(SessionPageSummaryError::Denied)
        );
    }
    let browser_actor = CallerIdentity {
        app_id: AppId::from_identifier(b"org.nagi.albert"),
        ..actor()
    };
    assert_eq!(
        service.summarize_page(
            &user,
            browser_actor,
            1,
            request(browser_actor),
            &Authority,
            &mut browser,
            &Never
        ),
        Err(SessionPageSummaryError::Denied)
    );
    assert_eq!(browser.calls.get(), 0);
}
#[test]
fn actual_wrapper_preserves_albert_sharing_and_explicit_license_requirement() {
    let (user, _, mut service, mut browser) = setup();
    let result = service.summarize_page(
        &user,
        actor(),
        1,
        request(actor()),
        &Authority,
        &mut browser,
        &Never,
    );
    assert!(matches!(result, Err(SessionPageSummaryError::Summary(_))));
    assert_eq!(browser.calls.get(), 1);
    // Context permission is explicitly enabled in this synthetic host fixture;
    // it does not acknowledge model-license terms.
    browser.policy.enable_by_user();
    assert_eq!(
        service.summarize_page(
            &user,
            actor(),
            2,
            request(actor()),
            &Authority,
            &mut browser,
            &Never
        ),
        Err(SessionPageSummaryError::Service(ModelServiceError::Store(
            StoreError::LicenseAcknowledgementRequired
        )))
    );
}
#[test]
fn ordinary_generate_still_rejects_external_assistant_app_id() {
    let (user, _, mut service, _) = setup();
    let capability = CapabilityId::new("text.generate").unwrap();
    let req = ModelRequest {
        request_id: 1,
        caller: Some(NAGI_BAR_APP_ID),
        capability: &capability,
        system_prompt: None,
        input: "synthetic",
        input_tokens: None,
        max_output_tokens: 1,
        options: nagi_model_manager::GenerationOptions::default(),
        timeout_millis: Some(1000),
        structured_output: None,
    };
    assert_eq!(
        service.generate(&user, None, None, &req, &Never),
        Err(SessionModelError::Denied)
    );
}
// Guard symbols only. Reaching any native model entry aborts this host suite;
// they never provide a replacement inference implementation.
#[unsafe(no_mangle)]
extern "C" fn nagi_m20_llama_backend_initialize() -> i32 {
    std::process::abort()
}
#[unsafe(no_mangle)]
extern "C" fn nagi_m20_llama_load_from_fd(_: i32, _: u32, _: i32) -> *mut core::ffi::c_void {
    std::process::abort()
}
#[unsafe(no_mangle)]
extern "C" fn nagi_m20_llama_set_cancel_callback(
    _: *mut core::ffi::c_void,
    _: Option<unsafe extern "C" fn(*mut core::ffi::c_void) -> bool>,
    _: *mut core::ffi::c_void,
) {
    std::process::abort()
}
#[unsafe(no_mangle)]
extern "C" fn nagi_m20_llama_free(_: *mut core::ffi::c_void) {
    std::process::abort()
}
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
extern "C" fn nagi_m20_llama_generate(
    _: *mut core::ffi::c_void,
    _: *const core::ffi::c_char,
    _: *const core::ffi::c_char,
    _: *const core::ffi::c_char,
    _: i32,
    _: f32,
    _: f32,
    _: u32,
    _: *mut core::ffi::c_char,
    _: usize,
    _: *mut usize,
    _: *mut u32,
    _: *mut u32,
) -> i32 {
    std::process::abort()
}

pub(super) mod posix {
    type ReadAt = unsafe extern "C" fn(*mut core::ffi::c_void, u64, *mut u8, usize) -> isize;
    pub unsafe fn open_readonly_callback(
        _: *mut core::ffi::c_void,
        _: u64,
        _: ReadAt,
    ) -> Result<i32, i32> {
        panic!("unexpected host model artifact callback")
    }
}

struct Cancel(Cell<bool>);
impl CancellationToken for Cancel {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}
#[test]
fn actual_wrapper_cancellation_stops_before_page_access() {
    let (user, _, mut service, mut browser) = setup();
    let cancel = Cancel(Cell::new(true));
    assert_eq!(
        service.summarize_page(
            &user,
            actor(),
            1,
            request(actor()),
            &Authority,
            &mut browser,
            &cancel
        ),
        Err(SessionPageSummaryError::Cancelled)
    );
    assert_eq!(browser.calls.get(), 0);
}
#[test]
fn cancellation_after_synchronous_failure_takes_precedence_over_service_error() {
    let (user, _, _, mut browser) = setup();
    browser.policy.enable_by_user();
    let cancel = Cancel(Cell::new(false));
    let access = ModelSessionAccess::bind(&user).unwrap();
    let result = super::session_page_summary::summarize(
        &access,
        &user,
        actor(),
        1,
        request(actor()),
        &Authority,
        &mut browser,
        ModelId::new("synthetic-model").unwrap(),
        LIMITS,
        |request, _| {
            assert!(request.caller.is_none());
            assert!(request.system_prompt.is_some());
            cancel.0.set(true);
            Err(SessionPageSummaryError::Service(
                ModelServiceError::ResourceLimit,
            ))
        },
        &cancel,
    );
    assert_eq!(result, Err(SessionPageSummaryError::Cancelled));
}

#[test]
fn prepared_worker_entry_keeps_live_session_and_typed_terms_gate() {
    let (user, other, mut service, mut browser) = setup();
    browser.policy.enable_by_user();
    let prompt =
        nagi_ai::prepare_page_summary(7, request(actor()), &Authority, &mut browser).unwrap();
    assert_eq!(
        service.generate_page_summary(&other, &prompt, &Never),
        Err(SessionPageSummaryError::Denied)
    );
    assert_eq!(
        service.generate_page_summary(&user, &prompt, &Never),
        Err(SessionPageSummaryError::Service(ModelServiceError::Store(
            StoreError::LicenseAcknowledgementRequired
        )))
    );
    let mut locked = user;
    locked.lock();
    assert_eq!(
        service.generate_page_summary(&locked, &prompt, &Never),
        Err(SessionPageSummaryError::Denied)
    );
}
