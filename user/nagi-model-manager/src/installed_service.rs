//! OS-owned worker adapter for the installed selector and existing ModelBackend.
//! No canonical Session issuer, guest Store, native linker or inference is supplied.
use crate::{
    CancellationToken, InstalledModelSelector, InstalledReceipt, InstalledSelectionError,
    InstalledSelectionMode, LiveAuthority, ModelBackend, ModelId, ModelRequest, ModelResponse,
    ModelServiceClock, ModelServiceLimits, ReceiptError, RuntimeError, RuntimeResourceReport,
    SelectionRequest, TrustedInstalledModels,
};
use core::cell::Cell;

/// Actual pending UI/worker request epoch, independently checked by canonical
/// owner. Numeric arguments and copied Session/Role/token values are never grants.
pub trait LiveRequestAuthority: LiveAuthority {
    fn check_request_epoch(&self, live: &Self::Session, epoch: u64) -> Result<(), ReceiptError>;
}
/// Owner rechecks actual durable namespace/revision/current manifest/exact terms
/// and measured privacy/resource policy, not previously copied registry metadata.
pub trait TrustedResidentModels: TrustedInstalledModels {
    fn revalidate_resident(
        &self,
        receipt: &InstalledReceipt,
    ) -> Result<(), InstalledSelectionError>;
    fn admit_inference(
        &self,
        receipt: &InstalledReceipt,
        request: &ModelRequest<'_>,
    ) -> Result<(), InstalledSelectionError>;
    fn admit_response(
        &self,
        receipt: &InstalledReceipt,
        response: &ModelResponse,
    ) -> Result<(), InstalledSelectionError>;
}

pub struct InstalledModelService<
    B: ModelBackend,
    A: LiveRequestAuthority,
    S: TrustedResidentModels,
    C: ModelServiceClock,
> {
    selector: InstalledModelSelector<B>,
    authority: A,
    installed: S,
    clock: C,
    limits: ModelServiceLimits,
    selected_stamp: Option<A::Stamp>,
    last_epoch: u64,
    last_request: u64,
    select_impl: fn(
        &mut InstalledModelSelector<B>,
        InstalledSelectionMode<'_>,
        &SelectionRequest<'_>,
        &A,
        &A::Session,
        &dyn CancellationToken,
        &mut S,
    ) -> Result<ModelId, InstalledSelectionError>,
}
impl<B: ModelBackend, A: LiveRequestAuthority, S: TrustedResidentModels, C: ModelServiceClock>
    InstalledModelService<B, A, S, C>
{
    /// Lazy constructor. Does not read artifacts, initialize or load the backend.
    pub fn new(
        backend: B,
        authority: A,
        installed: S,
        clock: C,
        limits: ModelServiceLimits,
    ) -> Result<Self, InstalledSelectionError> {
        if limits.max_input_bytes == 0
            || limits.max_output_bytes == 0
            || limits.max_output_tokens == 0
            || limits.max_request_millis == 0
        {
            return Err(InstalledSelectionError::AdmissionRejected);
        }
        Ok(Self {
            selector: InstalledModelSelector::new(backend),
            authority,
            installed,
            clock,
            limits,
            selected_stamp: None,
            last_epoch: 0,
            last_request: 0,
            select_impl: InstalledModelSelector::select::<A, S>,
        })
    }
    /// Typed opt-in: ordinary mutable readers cannot satisfy this sealed bound.
    /// ```compile_fail
    /// use nagi_model_manager::*;
    /// fn ordinary<B:ModelBackend,A:LiveRequestAuthority,S:TrustedResidentModels,C:ModelServiceClock>(b:B,a:A,s:S,c:C,limits:ModelServiceLimits) {
    ///     let _=InstalledModelService::new_authenticated_immutable_views(b,a,s,c,limits,None);
    /// }
    /// ```
    pub fn new_authenticated_immutable_views(
        backend: B,
        authority: A,
        installed: S,
        clock: C,
        limits: ModelServiceLimits,
        observer: Option<crate::PrivateInstalledStageObserver>,
    ) -> Result<Self, InstalledSelectionError>
    where
        S::Lease: crate::AuthenticatedImmutableArtifact,
    {
        let mut service = Self::new(backend, authority, installed, clock, limits)?;
        service.select_impl = InstalledModelSelector::select_authenticated::<A, S>;
        service.selector.set_private_stage_observer(observer);
        Ok(service)
    }
    pub fn model_id(&self) -> Option<&ModelId> {
        self.selector.model_id()
    }
    pub fn is_quarantined(&self) -> bool {
        self.selector.is_quarantined()
    }
    pub fn resource_report(&self) -> RuntimeResourceReport {
        self.selector.resource_report()
    }
    pub fn backend(&self) -> &B {
        self.selector.backend()
    }
    fn discard(&mut self, error: InstalledSelectionError) -> InstalledSelectionError {
        if error == InstalledSelectionError::Quarantined {
            self.selector.quarantine_for_owner();
        }
        let _ = self.selector.retire_for_owner();
        self.selected_stamp = None;
        error
    }
    pub fn select(
        &mut self,
        mode: InstalledSelectionMode<'_>,
        request: &SelectionRequest<'_>,
        live: &A::Session,
        cancel: &dyn CancellationToken,
    ) -> Result<ModelId, InstalledSelectionError> {
        self.sync_session(live, cancel)?;
        let stamp = match self.authority.check(live) {
            Ok(s) => s,
            Err(e) => return Err(self.discard(InstalledSelectionError::Owner(e))),
        };
        if self
            .selected_stamp
            .as_ref()
            .is_some_and(|old| *old != stamp)
        {
            return Err(self.discard(InstalledSelectionError::Owner(ReceiptError::Stale)));
        }
        // Generation requires a monotonic clock. Check it before selection can
        // open artifacts or load a resident backend session. Do not discard an
        // already selected session here: clock unavailability alone does not
        // revoke its owner or authorize destructive cleanup.
        if self.clock.now_millis().is_none() {
            return Err(InstalledSelectionError::Owner(ReceiptError::Unavailable));
        }
        let result = (self.select_impl)(
            &mut self.selector,
            mode,
            request,
            &self.authority,
            live,
            cancel,
            &mut self.installed,
        );
        if cancel.is_cancelled() {
            return Err(self.discard(InstalledSelectionError::Owner(ReceiptError::Cancelled)));
        }
        match self.authority.check(live) {
            Ok(now) if now == stamp => {
                if result.is_ok() {
                    self.selected_stamp = Some(stamp);
                }
            }
            Ok(_) => return Err(self.discard(InstalledSelectionError::Owner(ReceiptError::Stale))),
            Err(e) => return Err(self.discard(InstalledSelectionError::Owner(e))),
        }
        if result.is_err() && self.selector.model_id().is_none() {
            self.selected_stamp = None;
        }
        result
    }
    /// Worker calls on lock/signout/revocation notification and before request work.
    pub fn sync_session(
        &mut self,
        live: &A::Session,
        cancel: &dyn CancellationToken,
    ) -> Result<(), InstalledSelectionError> {
        let result = if cancel.is_cancelled() {
            Err(ReceiptError::Cancelled)
        } else {
            self.authority
                .check(live)
                .and_then(|now| match &self.selected_stamp {
                    Some(stamp) if *stamp != now => Err(ReceiptError::Stale),
                    _ => Ok(()),
                })
        };
        if let Err(e) = result {
            return Err(self.discard(InstalledSelectionError::Owner(e)));
        }
        Ok(())
    }
    pub fn unload(&mut self) -> Result<(), InstalledSelectionError> {
        self.selected_stamp = None;
        self.selector.retire_for_owner()
    }
    /// Complete response only, admitted against the exact live epoch/session before
    /// exposure. No execution authority or implicit model substitution is returned.
    pub fn generate(
        &mut self,
        epoch: u64,
        live: &A::Session,
        request: &ModelRequest<'_>,
        cancel: &dyn CancellationToken,
    ) -> Result<ModelResponse, InstalledSelectionError> {
        self.sync_session(live, cancel)?;
        if self.selector.is_quarantined() {
            return Err(InstalledSelectionError::Quarantined);
        }
        if request.caller.is_some() {
            return Err(self.discard(InstalledSelectionError::Owner(ReceiptError::Denied)));
        }
        if epoch == 0
            || epoch <= self.last_epoch
            || request.request_id == 0
            || request.request_id <= self.last_request
        {
            return Err(InstalledSelectionError::Owner(ReceiptError::Stale));
        }
        self.authority
            .check_request_epoch(live, epoch)
            .map_err(|e| self.discard(InstalledSelectionError::Owner(e)))?;
        let timeout = request
            .timeout_millis
            .filter(|t| *t > 0 && *t <= self.limits.max_request_millis)
            .ok_or(InstalledSelectionError::AdmissionRejected)?;
        let bytes = request
            .input
            .len()
            .checked_add(request.system_prompt.map_or(0, str::len))
            .ok_or(InstalledSelectionError::AdmissionRejected)?;
        if bytes > self.limits.max_input_bytes
            || request.max_output_tokens == 0
            || request.max_output_tokens > self.limits.max_output_tokens
        {
            return Err(InstalledSelectionError::AdmissionRejected);
        }
        let stamp = self
            .selected_stamp
            .clone()
            .ok_or(InstalledSelectionError::Unavailable)?;
        let start = self
            .clock
            .now_millis()
            .ok_or(InstalledSelectionError::Owner(ReceiptError::Unavailable))?;
        let deadline = start
            .checked_add(timeout)
            .ok_or(InstalledSelectionError::AdmissionRejected)?;
        self.last_epoch = epoch;
        self.last_request = request.request_id;
        let authority = &self.authority;
        let installed = &self.installed;
        let clock = &self.clock;
        let limits = self.limits;
        let result = self.selector.invoke_for_owner(|runtime, receipt| {
            let fence = RequestFence {
                authority,
                installed,
                clock,
                receipt,
                live,
                stamp: &stamp,
                cancel,
                epoch,
                start,
                deadline,
                request,
                failure: Cell::new(None),
            };
            fence.check()?;
            if runtime.resource_report().loaded_model_sessions != 1 {
                return Err(InstalledSelectionError::Quarantined);
            }
            installed.admit_inference(receipt, request)?;
            fence.check()?;
            let generated = runtime.generate(request, &fence);
            fence.check()?;
            let response = generated.map_err(InstalledSelectionError::Runtime)?;
            if response.request_id != request.request_id
                || response.model_id != receipt.manifest().model_id
                || response.text.len() > limits.max_output_bytes
                || response.text.trim().is_empty()
            {
                return Err(InstalledSelectionError::Runtime(
                    RuntimeError::InvalidBackendResponse,
                ));
            }
            installed.admit_response(receipt, &response)?;
            fence.check()?;
            Ok(response)
        });
        match result {
            Ok(response) => Ok(response),
            Err(e) => Err(self.discard(e)),
        }
    }
}
struct RequestFence<'a, A: LiveRequestAuthority, S: TrustedResidentModels, C: ModelServiceClock> {
    authority: &'a A,
    installed: &'a S,
    clock: &'a C,
    receipt: &'a InstalledReceipt,
    live: &'a A::Session,
    stamp: &'a A::Stamp,
    cancel: &'a dyn CancellationToken,
    epoch: u64,
    start: u64,
    deadline: u64,
    request: &'a ModelRequest<'a>,
    failure: Cell<Option<InstalledSelectionError>>,
}
impl<A: LiveRequestAuthority, S: TrustedResidentModels, C: ModelServiceClock>
    RequestFence<'_, A, S, C>
{
    fn check(&self) -> Result<(), InstalledSelectionError> {
        if let Some(e) = self.failure.get() {
            return Err(e);
        }
        self.clock.cooperate();
        let result = (|| {
            if self.cancel.is_cancelled() {
                return Err(InstalledSelectionError::Owner(ReceiptError::Cancelled));
            }
            let stamp = self
                .authority
                .check(self.live)
                .map_err(InstalledSelectionError::Owner)?;
            if stamp != *self.stamp {
                return Err(InstalledSelectionError::Owner(ReceiptError::Stale));
            }
            self.authority
                .check_request_epoch(self.live, self.epoch)
                .map_err(InstalledSelectionError::Owner)?;
            self.installed.revalidate_resident(self.receipt)?;
            self.installed.admit_inference(self.receipt, self.request)?;
            let now = self
                .clock
                .now_millis()
                .ok_or(InstalledSelectionError::Owner(ReceiptError::Unavailable))?;
            if now < self.start || now >= self.deadline {
                return Err(InstalledSelectionError::Runtime(RuntimeError::Timeout));
            }
            self.installed.revalidate_resident(self.receipt)?;
            let now = self
                .clock
                .now_millis()
                .ok_or(InstalledSelectionError::Owner(ReceiptError::Unavailable))?;
            if now < self.start || now >= self.deadline {
                return Err(InstalledSelectionError::Runtime(RuntimeError::Timeout));
            }
            // Owner callbacks can yield; the final clock read must be synchronous.
            // Canonical authority/epoch reads likewise must not schedule afterward. Revalidate live identity and epoch
            // after them, without another scheduling point before returning.
            if self.cancel.is_cancelled() {
                return Err(InstalledSelectionError::Owner(ReceiptError::Cancelled));
            }
            let stamp = self
                .authority
                .check(self.live)
                .map_err(InstalledSelectionError::Owner)?;
            if stamp != *self.stamp {
                return Err(InstalledSelectionError::Owner(ReceiptError::Stale));
            }
            self.authority
                .check_request_epoch(self.live, self.epoch)
                .map_err(InstalledSelectionError::Owner)?;
            Ok(())
        })();
        if let Err(e) = result {
            self.failure.set(Some(e));
        }
        result
    }
}
impl<A: LiveRequestAuthority, S: TrustedResidentModels, C: ModelServiceClock> CancellationToken
    for RequestFence<'_, A, S, C>
{
    fn is_cancelled(&self) -> bool {
        self.check().is_err()
    }
}
