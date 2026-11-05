//! Launch-mediated Bar endpoint ownership for one live Desktop epoch.
//! No endpoint is adopted from a caller, and no Session is attached after launch.
//! Normal runtime has no signed Bar package/teardown/IPC owner yet; it does not
//! call these staged APIs or enable live Browser context as a result.

use super::{Desktop, PageSummaryBinding};
use crate::supervisor::{self, LaunchFailure, Launched, ReceiveFailure, ReceivedAppMessage};
use libnagi::launch::{LaunchPlacement, LaunchRecord};
use nagi_action_ipc::bar_summary as wire;

#[must_use = "retain this endpoint or explicitly transfer it to the process reaper"]
pub struct BarEndpoint {
    launched: Launched,
    binding: PageSummaryBinding,
    exchange: wire::Exchange,
}

/// A rejected spawned launch stays owned by the caller for explicit reaping.
/// Never discard Rejected's endpoint or block the Desktop event loop on reap.
#[must_use]
pub enum BarLaunchFailure {
    NotReady,
    Launch(LaunchFailure),
    Rejected(Launched),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarReceiveFailure {
    Unbound,
    Receive(ReceiveFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarWireFailure {
    Inactive,
    SendUnavailable,
    Receive(BarReceiveFailure),
    Wire(wire::Error),
}

/// One consumed wire exchange, not a model/Browser permission. The current
/// Desktop intent, epoch, live actor and clock must still match at use.
#[must_use]
pub struct ValidatedSummaryRequest {
    binding: PageSummaryBinding,
    record: LaunchRecord,
    request: wire::ValidatedRequest,
    terminal_sent: bool,
}
impl ValidatedSummaryRequest {
    pub fn into_actor_for(self, desktop: &Desktop) -> Option<(nagi_ai::CallerIdentity, u64)> {
        // Resolve first: registry waiting must not make an earlier clock read
        // appear fresh. This still is not an atomic authorization transaction.
        let live = supervisor::resolve(self.record.process_id);
        self.actor_with(desktop, libnagi::time_ticks(), |_| live)
    }
    fn actor_with(
        &self,
        desktop: &Desktop,
        now: u64,
        resolve: impl Fn(u32) -> Option<LaunchRecord>,
    ) -> Option<(nagi_ai::CallerIdentity, u64)> {
        if self.terminal_sent
            || current_binding(desktop).as_ref() != Some(&self.binding)
            || desktop.bar.wire_summary_intent() != Some(self.request.id())
            || !self.request.is_fresh(now)
            || resolve(self.record.process_id) != Some(self.record)
        {
            return None;
        }
        Some((
            nagi_ai::CallerIdentity {
                app_id: self.record.app_id,
                app_session_id: self.record.app_session_id,
                node_id: self.record.node_id,
                workspace_id: self.record.workspace_id,
            },
            self.request.id(),
        ))
    }
}

/// Can only be produced from the real Supervisor receiver below. This carries
/// app sender plus its launch-time Desktop epoch, not protocol/intent or model
/// authorization. No method accepts payload-supplied CallerIdentity fields.
pub struct BarMessage<'a> {
    received: ReceivedAppMessage<'a>,
    binding: PageSummaryBinding,
}

fn current_binding(desktop: &Desktop) -> Option<PageSummaryBinding> {
    let user = desktop.session?;
    if user.is_locked() || user.token() == 0 {
        return None;
    }
    desktop.page_summary_binding()
}

impl Desktop {
    /// Trusted init only. Package bytes must come from its normal signed-package
    /// source; Supervisor verifies the signature and exact NAGI_BAR_APP_ID.
    /// Placement is Supervisor/OS state, never browser/page/client payload.
    /// This does NOT accept model terms, enable sharing or create a Browser.
    pub fn launch_bar_endpoint(
        &mut self,
        package: &[u8],
        placement: LaunchPlacement,
    ) -> Result<BarEndpoint, BarLaunchFailure> {
        launch_bound(
            self,
            package,
            placement,
            |_, package, app, placement| supervisor::launch(package, app, placement),
            supervisor::resolve,
        )
    }
}

// Private seams model launch/registry outcomes in host tests; public production
// entry always supplies actual signed launch and live Supervisor resolution.
fn launch_bound(
    desktop: &mut Desktop,
    package: &[u8],
    placement: LaunchPlacement,
    launch: impl FnOnce(
        &mut Desktop,
        &[u8],
        nagi_model::AppId,
        LaunchPlacement,
    ) -> Result<Launched, LaunchFailure>,
    resolve: impl Fn(u32) -> Option<LaunchRecord>,
) -> Result<BarEndpoint, BarLaunchFailure> {
    let binding = current_binding(desktop).ok_or(BarLaunchFailure::NotReady)?;
    let launched = launch(desktop, package, nagi_ai::NAGI_BAR_APP_ID, placement)
        .map_err(BarLaunchFailure::Launch)?;
    if current_binding(desktop).as_ref() != Some(&binding)
        || launched.record.app_id != nagi_ai::NAGI_BAR_APP_ID
        || resolve(launched.record.process_id) != Some(launched.record)
    {
        return Err(BarLaunchFailure::Rejected(launched));
    }
    Ok(BarEndpoint {
        launched,
        binding,
        exchange: wire::Exchange::new(),
    })
}

impl BarEndpoint {
    fn is_current_with(
        &self,
        desktop: &Desktop,
        resolve: impl Fn(u32) -> Option<LaunchRecord>,
    ) -> bool {
        current_binding(desktop).as_ref() == Some(&self.binding)
            && self.launched.record.app_id == nagi_ai::NAGI_BAR_APP_ID
            && resolve(self.launched.record.process_id) == Some(self.launched.record)
    }

    /// No retrospective binding to a different Desktop/Session is offered.
    /// Registry validation/use is not an atomic authorization transaction, and
    /// the existing registry spinlock has no proven time bound.
    pub fn try_receive_for(
        &self,
        desktop: &Desktop,
    ) -> Result<Option<BarMessage<'_>>, BarReceiveFailure> {
        if !self.is_current_with(desktop, supervisor::resolve) {
            return Err(BarReceiveFailure::Unbound);
        }
        let received =
            supervisor::try_receive_from(&self.launched).map_err(BarReceiveFailure::Receive)?;
        if !self.is_current_with(desktop, supervisor::resolve) {
            return Err(BarReceiveFailure::Unbound);
        }
        Ok(received.map(|received| BarMessage {
            received,
            binding: self.binding.clone(),
        }))
    }

    /// Issue only for the current explicit local UI gesture; the client never
    /// chooses the request ID. This transports no page/model data or identity.
    pub fn offer_summary_wire(&mut self, desktop: &mut Desktop) -> Result<(), BarWireFailure> {
        self.offer_with(
            desktop,
            libnagi::time_ticks(),
            supervisor::resolve,
            libnagi::channel_send,
        )
    }
    fn offer_with(
        &mut self,
        desktop: &mut Desktop,
        now: u64,
        resolve: impl Fn(u32) -> Option<LaunchRecord>,
        send: impl FnOnce(u64, &libnagi::ChannelSendRequest) -> bool,
    ) -> Result<(), BarWireFailure> {
        if !self.is_current_with(desktop, resolve) {
            self.exchange.cancel();
            return Err(BarWireFailure::Inactive);
        }
        let id = desktop
            .bar
            .claim_summary_wire()
            .ok_or(BarWireFailure::Inactive)?;
        let offer = self.exchange.begin(id, now).map_err(BarWireFailure::Wire)?;
        if !send(self.launched.endpoint, &offer) {
            self.exchange.cancel();
            return Err(BarWireFailure::SendUnavailable);
        }
        Ok(())
    }

    /// Read at most one authenticated frame, then consume that pending intent.
    /// No model submission/terms/sharing acknowledgement follows automatically.
    pub fn consume_summary_wire(
        &mut self,
        desktop: &Desktop,
    ) -> Result<Option<ValidatedSummaryRequest>, BarWireFailure> {
        if !self.is_current_with(desktop, supervisor::resolve)
            || desktop.bar.wire_summary_intent().is_none()
        {
            self.exchange.cancel();
            return Err(BarWireFailure::Inactive);
        }
        let received = match self.try_receive_for(desktop) {
            Ok(received) => received,
            Err(error) => {
                self.exchange.cancel();
                return Err(BarWireFailure::Receive(error));
            }
        };
        let Some(message) = received else {
            self.exchange
                .expire(libnagi::time_ticks())
                .map_err(BarWireFailure::Wire)?;
            return Ok(None);
        };
        let record = message
            .received
            .live_launch()
            .ok_or(BarWireFailure::Inactive)?;
        let envelope = *message
            .envelope_for(desktop)
            .ok_or(BarWireFailure::Inactive)?;
        // End the immutable endpoint borrow before mutating the exchange.
        drop(message);
        self.consume_envelope_with(
            desktop,
            record,
            &envelope,
            libnagi::time_ticks,
            supervisor::resolve,
        )
        .map(Some)
    }
    // Private seam follows the real receiver above; host fixtures are never
    // presented as actual kernel receipts or authorized client processes.
    fn consume_envelope_with(
        &mut self,
        desktop: &Desktop,
        record: LaunchRecord,
        message: &libnagi::ChannelReceiveResult,
        now: impl FnOnce() -> u64,
        resolve: impl Fn(u32) -> Option<LaunchRecord>,
    ) -> Result<ValidatedSummaryRequest, BarWireFailure> {
        if !self.is_current_with(desktop, resolve)
            || record != self.launched.record
            || message.sender_process_id != record.process_id
            || desktop.bar.wire_summary_intent() != Some(message.request_id)
        {
            self.exchange.cancel();
            return Err(BarWireFailure::Inactive);
        }
        let request = self
            .exchange
            .accept(message, now())
            .map_err(BarWireFailure::Wire)?;
        Ok(ValidatedSummaryRequest {
            binding: self.binding.clone(),
            record,
            request,
            terminal_sent: false,
        })
    }

    /// Finish an unavailable/denied/cancelled transport only. No positive
    /// model-result terminal is defined; this burns the validation token too.
    /// A failed send is burned; cleanup is explicit retirement, not hidden retry.
    pub fn finish_summary_wire(
        &self,
        desktop: &Desktop,
        request: &mut ValidatedSummaryRequest,
        status: wire::Terminal,
    ) -> Result<(), BarWireFailure> {
        let live = supervisor::resolve(self.launched.record.process_id);
        self.finish_with(
            desktop,
            request,
            status,
            libnagi::time_ticks(),
            |_| live,
            libnagi::channel_send,
        )
    }
    fn finish_with(
        &self,
        desktop: &Desktop,
        request: &mut ValidatedSummaryRequest,
        status: wire::Terminal,
        now: u64,
        resolve: impl Fn(u32) -> Option<LaunchRecord>,
        send: impl FnOnce(u64, &libnagi::ChannelSendRequest) -> bool,
    ) -> Result<(), BarWireFailure> {
        if request.terminal_sent {
            return Err(BarWireFailure::Wire(wire::Error::Stale));
        }
        if request.record != self.launched.record
            || request.binding != self.binding
            || request.actor_with(desktop, now, resolve).is_none()
        {
            return Err(BarWireFailure::Inactive);
        }
        request.terminal_sent = true;
        let message = wire::terminal(request.request.id(), status).map_err(BarWireFailure::Wire)?;
        if !send(self.launched.endpoint, &message) {
            return Err(BarWireFailure::SendUnavailable);
        }
        Ok(())
    }

    /// Revoke registry authority and close this endpoint without process_wait.
    /// The returned child still needs a reaper outside the UI step; retirement
    /// is not proof of process exit or successful status consumption.
    pub fn retire(mut self) -> Result<supervisor::RetiredLaunch, supervisor::RetireRejected> {
        self.exchange.cancel();
        supervisor::retire(self.launched)
    }

    /// The lifecycle owner must explicitly reap/revoke the launch outside the
    /// latency-sensitive UI step. Dropping this value is not process teardown.
    pub fn into_launch_for_reaping(self) -> Launched {
        self.launched
    }
}

impl BarMessage<'_> {
    /// Recheck the CURRENT Desktop and live launch at use. A retained receipt
    /// cannot authorize work after lock/sign-out/readiness loss or a new epoch.
    /// Protocol, request freshness, sharing, authority, terms and budgets remain
    /// independent checks; this actor alone never permits a model submission.
    pub fn caller_for(&self, desktop: &Desktop) -> Option<nagi_ai::CallerIdentity> {
        if current_binding(desktop).as_ref() != Some(&self.binding) {
            return None;
        }
        let record = self.received.live_launch()?;
        (record.app_id == nagi_ai::NAGI_BAR_APP_ID).then_some(nagi_ai::CallerIdentity {
            app_id: record.app_id,
            app_session_id: record.app_session_id,
            node_id: record.node_id,
            workspace_id: record.workspace_id,
        })
    }

    /// The consumer still validates every wire field; receiving authentic app
    /// metadata does not authenticate a request or grant Browser access.
    pub fn envelope_for<'a>(
        &'a self,
        desktop: &Desktop,
    ) -> Option<&'a libnagi::ChannelReceiveResult> {
        self.caller_for(desktop)?;
        Some(self.received.envelope())
    }
}

#[cfg(all(test, not(target_os = "nagi"), feature = "desktop-login"))]
mod tests {
    use super::*;
    use libnagi::launch::LaunchError;
    use nagi_localization::Locale;
    use nagi_model::{AppSessionId, NodeId};
    fn fixture() -> (Desktop, super::super::UserDataVolume) {
        let (desktop, volume, _) =
            super::super::regression::summary_desktop_fixture(Locale::EnUs, false);
        (desktop, volume)
    }
    fn placement() -> LaunchPlacement {
        LaunchPlacement {
            app_session_id: AppSessionId(8),
            node_id: NodeId(9),
            workspace_id: None,
        }
    }
    fn record() -> LaunchRecord {
        LaunchRecord {
            process_id: 7,
            app_id: nagi_ai::NAGI_BAR_APP_ID,
            app_session_id: AppSessionId(8),
            node_id: NodeId(9),
            workspace_id: None,
        }
    }
    fn bound_endpoint(desktop: &mut Desktop) -> BarEndpoint {
        // Synthetic launch/registry only. The production entry uses real signed
        // packages and kernel-backed endpoints; no OS provenance is claimed here.
        match launch_bound(
            desktop,
            b"synthetic package",
            placement(),
            |_, package, app, _| {
                assert_eq!(package, b"synthetic package");
                assert_eq!(app, nagi_ai::NAGI_BAR_APP_ID);
                Ok(Launched {
                    endpoint: 42,
                    record: record(),
                })
            },
            |pid| (pid == 7).then_some(record()),
        ) {
            Ok(endpoint) => endpoint,
            Err(_) => panic!("ready fixture launch should bind"),
        }
    }
    #[test]
    fn absent_readiness_or_locked_live_session_never_calls_launch() {
        let (mut desktop, _) = fixture();
        desktop.model_services_ready = false;
        assert!(matches!(
            launch_bound(
                &mut desktop,
                b"",
                placement(),
                |_, _, _, _| panic!("not ready"),
                |_| None
            ),
            Err(BarLaunchFailure::NotReady)
        ));
        desktop.model_services_ready = true;
        desktop.session.as_mut().unwrap().lock();
        assert!(matches!(
            launch_bound(
                &mut desktop,
                b"",
                placement(),
                |_, _, _, _| panic!("locked"),
                |_| None
            ),
            Err(BarLaunchFailure::NotReady)
        ));
    }
    #[test]
    fn launch_binding_preserves_unaccepted_terms_and_empty_model_queue() {
        let (mut desktop, _) = fixture();
        let endpoint = bound_endpoint(&mut desktop);
        assert!(endpoint.is_current_with(&desktop, |_| Some(record())));
        let state = desktop
            .model_services
            .as_ref()
            .unwrap()
            .host_shared_snapshot()
            .unwrap();
        assert!(!state.terms_confirmed);
        assert!(!state.queued);
        assert_eq!(state.active_request_id, None);
        let launched = endpoint.into_launch_for_reaping();
        assert_eq!(launched.endpoint, 42);
        assert_eq!(launched.record, record());
    }
    #[test]
    fn foreign_missing_or_changed_registry_record_is_not_bound_or_adopted() {
        let (mut desktop, _) = fixture();
        let mut foreign = record();
        foreign.app_id = nagi_model::AppId::from_identifier(b"org.nagi.albert");
        let result = launch_bound(
            &mut desktop,
            b"",
            placement(),
            |_, _, _, _| {
                Ok(Launched {
                    endpoint: 42,
                    record: foreign,
                })
            },
            |_| Some(foreign),
        );
        assert!(matches!(
            result,
            Err(BarLaunchFailure::Rejected(Launched { endpoint: 42, .. }))
        ));
        let endpoint = bound_endpoint(&mut desktop);
        assert!(!endpoint.is_current_with(&desktop, |_| None));
        let mut changed = record();
        changed.node_id = NodeId(123);
        assert!(!endpoint.is_current_with(&desktop, |_| Some(changed)));
    }
    #[test]
    fn loss_during_launch_retains_child_for_cleanup_and_launch_failure_stays_typed() {
        let (mut desktop, _) = fixture();
        let result = launch_bound(
            &mut desktop,
            b"",
            placement(),
            |desktop, _, _, _| {
                desktop.model_services_ready = false;
                Ok(Launched {
                    endpoint: 42,
                    record: record(),
                })
            },
            |_| Some(record()),
        );
        assert!(matches!(
            result,
            Err(BarLaunchFailure::Rejected(Launched { endpoint: 42, .. }))
        ));
        desktop.model_services_ready = true;
        assert!(matches!(
            launch_bound(
                &mut desktop,
                b"",
                placement(),
                |_, _, _, _| Err(LaunchFailure::Registry(LaunchError::UnknownApplication)),
                |_| None
            ),
            Err(BarLaunchFailure::Launch(LaunchFailure::Registry(
                LaunchError::UnknownApplication
            )))
        ));
    }
    #[test]
    fn old_epoch_and_readiness_loss_deny_before_touching_endpoint() {
        let (mut desktop, _) = fixture();
        let endpoint = bound_endpoint(&mut desktop);
        desktop.invalidate_page_summary_binding();
        desktop.refresh_page_summary_binding();
        assert!(!endpoint.is_current_with(&desktop, |_| Some(record())));
        assert!(matches!(
            endpoint.try_receive_for(&desktop),
            Err(BarReceiveFailure::Unbound)
        ));
        let endpoint = bound_endpoint(&mut desktop);
        desktop.model_services_ready = false;
        assert!(matches!(
            endpoint.try_receive_for(&desktop),
            Err(BarReceiveFailure::Unbound)
        ));
    }
    #[test]
    fn equal_session_and_generation_on_another_desktop_do_not_adopt_endpoint() {
        let (mut desktop, _) = fixture();
        let endpoint = bound_endpoint(&mut desktop);
        let (mut other, _) = fixture();
        let user = desktop.session.unwrap();
        other.session = Some(user);
        other
            .bar
            .controller
            .sign_in(&user, other.model_services.as_mut().unwrap());
        other.refresh_page_summary_binding();
        other.summary_binding_generation = endpoint.binding.generation;
        assert!(current_binding(&other).is_some());
        assert!(!endpoint.is_current_with(&other, |_| Some(record())));
    }
    fn gesture(desktop: &mut Desktop, volume: &mut super::super::UserDataVolume) {
        desktop.handle_event(
            libnagi::InputEvent {
                event_type: libnagi::INPUT_EVENT_KEY,
                code: super::super::bar_panel::SUMMARY_KEY,
                value: 1,
            },
            volume,
        );
    }
    fn response(id: u64) -> libnagi::ChannelReceiveResult {
        let offer = wire::offer(id).unwrap();
        let received = libnagi::ChannelReceiveResult {
            sender_process_id: 1,
            protocol_id: offer.protocol_id,
            version: offer.version,
            opcode: offer.opcode,
            request_id: id,
            payload_len: offer.payload_len,
            payload: offer.payload,
            ..libnagi::ChannelReceiveResult::default()
        };
        let request = wire::request_for_offer(&received).unwrap();
        libnagi::ChannelReceiveResult {
            sender_process_id: 7,
            protocol_id: request.protocol_id,
            version: request.version,
            opcode: request.opcode,
            request_id: id,
            payload_len: request.payload_len,
            payload: request.payload,
            ..libnagi::ChannelReceiveResult::default()
        }
    }
    #[test]
    fn wire_requires_explicit_local_intent_and_accepts_one_current_frame_without_model_rights() {
        let (mut desktop, mut volume) = fixture();
        let mut endpoint = bound_endpoint(&mut desktop);
        assert!(matches!(
            endpoint.offer_with(
                &mut desktop,
                100,
                |_| Some(record()),
                |_, _| panic!("no gesture")
            ),
            Err(BarWireFailure::Inactive)
        ));
        gesture(&mut desktop, &mut volume);
        let id = desktop.bar.summary_intent().unwrap();
        endpoint
            .offer_with(
                &mut desktop,
                100,
                |_| Some(record()),
                |handle, p| {
                    assert_eq!(handle, 42);
                    assert_eq!(p.request_id, id);
                    assert_eq!(p.payload_len, 1);
                    true
                },
            )
            .unwrap();
        let mut validated = endpoint
            .consume_envelope_with(
                &desktop,
                record(),
                &response(id),
                || 101,
                |_| Some(record()),
            )
            .ok()
            .unwrap();
        let (actor, wire_id) = validated
            .actor_with(&desktop, 102, |_| Some(record()))
            .unwrap();
        assert_eq!(actor.app_id, nagi_ai::NAGI_BAR_APP_ID);
        assert_eq!(wire_id, id);
        endpoint
            .finish_with(
                &desktop,
                &mut validated,
                wire::Terminal::Unavailable,
                102,
                |_| Some(record()),
                |_, p| {
                    assert_eq!(p.opcode, wire::RESULT);
                    assert_eq!(p.request_id, id);
                    assert_eq!(p.payload[0], wire::Terminal::Unavailable as u8);
                    true
                },
            )
            .unwrap();
        assert!(matches!(
            endpoint.finish_with(
                &desktop,
                &mut validated,
                wire::Terminal::Denied,
                102,
                |_| Some(record()),
                |_, _| panic!("terminal replay")
            ),
            Err(BarWireFailure::Wire(wire::Error::Stale))
        ));
        assert!(
            validated
                .actor_with(&desktop, 102, |_| Some(record()))
                .is_none(),
            "a negative terminal must not leave an admissible model token"
        );
        assert!(endpoint
            .consume_envelope_with(
                &desktop,
                record(),
                &response(id),
                || 102,
                |_| Some(record())
            )
            .is_err());
        let state = desktop
            .model_services
            .as_ref()
            .unwrap()
            .host_shared_snapshot()
            .unwrap();
        assert!(!state.terms_confirmed);
        assert!(!state.queued);
    }
    #[test]
    fn pending_wire_rejects_new_gesture_clock_expiry_sender_and_old_desktop_epoch() {
        for mode in 0..5 {
            let (mut desktop, mut volume) = fixture();
            let mut endpoint = bound_endpoint(&mut desktop);
            gesture(&mut desktop, &mut volume);
            let id = desktop.bar.summary_intent().unwrap();
            endpoint
                .offer_with(&mut desktop, 100, |_| Some(record()), |_, _| true)
                .unwrap();
            let mut frame = response(id);
            let mut now = 101;
            match mode {
                0 => gesture(&mut desktop, &mut volume),
                1 => now = 100 + wire::MAX_PENDING_TICKS,
                2 => frame.sender_process_id = 8,
                3 => {
                    desktop.invalidate_page_summary_binding();
                    desktop.refresh_page_summary_binding();
                }
                _ => now = 99,
            }
            assert!(endpoint
                .consume_envelope_with(&desktop, record(), &frame, || now, |_| Some(record()))
                .is_err());
        }
    }
    #[test]
    fn send_failure_burns_gesture_and_validated_frame_does_not_survive_cancel_or_clock_rollback() {
        let (mut desktop, mut volume) = fixture();
        let mut endpoint = bound_endpoint(&mut desktop);
        gesture(&mut desktop, &mut volume);
        assert!(matches!(
            endpoint.offer_with(&mut desktop, 100, |_| Some(record()), |_, _| false),
            Err(BarWireFailure::SendUnavailable)
        ));
        assert!(matches!(
            endpoint.offer_with(&mut desktop, 101, |_| Some(record()), |_, _| true),
            Err(BarWireFailure::Inactive)
        ));
        gesture(&mut desktop, &mut volume);
        let id = desktop.bar.summary_intent().unwrap();
        endpoint
            .offer_with(&mut desktop, 102, |_| Some(record()), |_, _| true)
            .unwrap();
        let validated = endpoint
            .consume_envelope_with(
                &desktop,
                record(),
                &response(id),
                || 103,
                |_| Some(record()),
            )
            .ok()
            .unwrap();
        assert!(validated
            .actor_with(&desktop, 102, |_| Some(record()))
            .is_none());
        gesture(&mut desktop, &mut volume);
        let id = desktop.bar.summary_intent().unwrap();
        endpoint
            .offer_with(&mut desktop, 104, |_| Some(record()), |_, _| true)
            .unwrap();
        let validated = endpoint
            .consume_envelope_with(
                &desktop,
                record(),
                &response(id),
                || 105,
                |_| Some(record()),
            )
            .ok()
            .unwrap();
        desktop.bar.cancel_requested = true;
        assert!(validated
            .actor_with(&desktop, 106, |_| Some(record()))
            .is_none());
    }
    #[test]
    fn replacing_endpoint_does_not_reoffer_an_already_claimed_ui_gesture() {
        let (mut desktop, mut volume) = fixture();
        let mut first = bound_endpoint(&mut desktop);
        gesture(&mut desktop, &mut volume);
        assert!(matches!(
            first.offer_with(&mut desktop, 100, |_| Some(record()), |_, _| false),
            Err(BarWireFailure::SendUnavailable)
        ));
        let mut replacement = bound_endpoint(&mut desktop);
        assert!(matches!(
            replacement.offer_with(
                &mut desktop,
                101,
                |_| Some(record()),
                |_, _| panic!("gesture already claimed")
            ),
            Err(BarWireFailure::Inactive)
        ));
        gesture(&mut desktop, &mut volume);
        replacement
            .offer_with(&mut desktop, 102, |_| Some(record()), |_, _| true)
            .unwrap();
    }
    fn key_event(desktop: &mut Desktop, volume: &mut super::super::UserDataVolume, code: u16) {
        assert!(desktop.handle_event(
            libnagi::InputEvent {
                event_type: libnagi::INPUT_EVENT_KEY,
                code,
                value: 1,
            },
            volume
        ));
    }
    fn close_and_reopen(
        desktop: &mut Desktop,
        volume: &mut super::super::UserDataVolume,
        route: u8,
    ) {
        match route {
            0 => key_event(desktop, volume, libnagi::INPUT_KEY_ESCAPE),
            1 => key_event(desktop, volume, 61), // F3 close
            2 => {
                desktop.pointer_x = super::super::SETTINGS_BUTTON.x + 1;
                desktop.pointer_y = super::super::SETTINGS_BUTTON.y + 1;
                key_event(desktop, volume, libnagi::INPUT_KEY_LEFT);
                assert!(desktop.settings_open);
                assert!(!desktop.bar.is_open());
                key_event(desktop, volume, libnagi::INPUT_KEY_LEFT);
                assert!(!desktop.settings_open);
            }
            _ => desktop.bar.clear_presentation(),
        }
        assert!(!desktop.bar.is_open());
        key_event(desktop, volume, 61); // F3 open, no new summary gesture
        assert!(desktop.bar.is_open());
    }
    #[test]
    fn all_close_routes_reject_old_validated_actor_and_terminal_until_new_gesture() {
        for route in 0..4 {
            let (mut desktop, mut volume) = fixture();
            let mut endpoint = bound_endpoint(&mut desktop);
            gesture(&mut desktop, &mut volume);
            let old = desktop.bar.summary_intent().unwrap();
            endpoint
                .offer_with(&mut desktop, 100, |_| Some(record()), |_, _| true)
                .unwrap();
            let mut token = endpoint
                .consume_envelope_with(
                    &desktop,
                    record(),
                    &response(old),
                    || 101,
                    |_| Some(record()),
                )
                .ok()
                .unwrap();
            assert!(token
                .actor_with(&desktop, 102, |_| Some(record()))
                .is_some());
            close_and_reopen(&mut desktop, &mut volume, route);
            assert!(
                token
                    .actor_with(&desktop, 103, |_| Some(record()))
                    .is_none(),
                "close route {route}"
            );
            assert!(endpoint
                .finish_with(
                    &desktop,
                    &mut token,
                    wire::Terminal::Denied,
                    103,
                    |_| Some(record()),
                    |_, _| panic!("closed intent terminal")
                )
                .is_err());
            assert!(endpoint
                .offer_with(
                    &mut desktop,
                    103,
                    |_| Some(record()),
                    |_, _| panic!("no new gesture")
                )
                .is_err());
            gesture(&mut desktop, &mut volume);
            let new = desktop.bar.summary_intent().unwrap();
            assert!(new > old);
            endpoint
                .offer_with(&mut desktop, 104, |_| Some(record()), |_, _| true)
                .unwrap();
            let current = endpoint
                .consume_envelope_with(
                    &desktop,
                    record(),
                    &response(new),
                    || 105,
                    |_| Some(record()),
                )
                .ok()
                .unwrap();
            assert!(current
                .actor_with(&desktop, 106, |_| Some(record()))
                .is_some());
            assert!(token
                .actor_with(&desktop, 106, |_| Some(record()))
                .is_none());
        }
    }
    #[test]
    fn all_close_routes_reject_pending_frame_then_allow_fresh_explicit_request() {
        for route in 0..4 {
            let (mut desktop, mut volume) = fixture();
            let mut endpoint = bound_endpoint(&mut desktop);
            gesture(&mut desktop, &mut volume);
            let old = desktop.bar.summary_intent().unwrap();
            endpoint
                .offer_with(&mut desktop, 100, |_| Some(record()), |_, _| true)
                .unwrap();
            close_and_reopen(&mut desktop, &mut volume, route);
            assert!(
                endpoint
                    .consume_envelope_with(
                        &desktop,
                        record(),
                        &response(old),
                        || 101,
                        |_| Some(record())
                    )
                    .is_err(),
                "close route {route}"
            );
            gesture(&mut desktop, &mut volume);
            let new = desktop.bar.summary_intent().unwrap();
            assert!(new > old);
            endpoint
                .offer_with(&mut desktop, 102, |_| Some(record()), |_, _| true)
                .unwrap();
            let current = endpoint
                .consume_envelope_with(
                    &desktop,
                    record(),
                    &response(new),
                    || 103,
                    |_| Some(record()),
                )
                .ok()
                .unwrap();
            assert!(current
                .actor_with(&desktop, 104, |_| Some(record()))
                .is_some());
        }
    }
}
