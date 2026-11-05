//! Ordinary-engine orchestration prerequisite; no production backend yet.
//!
//! The eventual Nagi backend must own actual Servo/WebViews/Surface/input and
//! verify the live Session before construction. This local trait and numeric
//! binding provide no authorization. FirstPixel/acceptance paths are separate.

use std::rc::Rc;

use nagi_ai::{
    BrowserContextApiError, BrowserContextRequest, BrowserPageSnapshot, PublicBrowserContextApi,
};

use crate::browser_state::{NavigationEventResult, StopRequest};
use crate::navigation::NavigationRequest;
use crate::runtime_owner::{
    AlbertRuntimeOwner, Attachment, InputOutcome, OwnerError, PageInput, PageTicket, SurfaceBinding,
};
use crate::tabs::TabId;
use crate::ui::{BrowserChromeAction, BrowserChromeOutcome, BrowserChromeView};

/// Local WebView identity. A backend must retain this exact token with its
/// actual WebView, never derive it from a tab number on callback delivery.
#[derive(Clone, Debug)]
pub struct ViewToken {
    identity: Rc<()>,
    tab: TabId,
}

/// Captured when submitting one navigation. Reuse of a WebView or URL must not
/// relabel an old callback with the token of a newer load.
#[derive(Clone, Debug)]
pub struct NavigationToken {
    view: ViewToken,
    identity: Rc<()>,
    request: NavigationRequest,
}

/// Narrow synchronous backend boundary. No implementation is provided here.
/// Production implementations must retain guest resources on the event thread;
/// callbacks must be captured with their original view/navigation/page tokens.
/// Drop must release all resources and local input state even after a failure.
/// Methods must not reenter this step owner. Errors retire the entire backend:
/// potentially applied input/navigation is never retried automatically.
pub trait EngineBackend {
    type Error;
    fn create_view(&mut self, view: &ViewToken) -> Result<(), Self::Error>;
    fn remove_view(&mut self, view: &ViewToken) -> Result<(), Self::Error>;
    /// Select visible content without overriding chrome/address input focus.
    fn activate(&mut self, view: &ViewToken) -> Result<(), Self::Error>;
    fn navigate(&mut self, navigation: &NavigationToken) -> Result<(), Self::Error>;
    fn stop(&mut self, view: &ViewToken) -> Result<(), Self::Error>;
    /// Clear pressed keys, buttons, capture, modifiers and pending composition
    /// for ALL WebViews, including the previously active one.
    fn clear_input(&mut self) -> Result<(), Self::Error>;
    fn input(&mut self, view: &ViewToken, input: PageInput) -> Result<(), Self::Error>;
    /// One Servo dispatch, no nested application loop. The bool is a wake hint;
    /// it is neither an exit signal nor a wall-clock bound on Servo execution.
    fn spin_once(&mut self) -> Result<bool, Self::Error>;
}

impl ViewToken {
    pub fn tab_id(&self) -> TabId {
        self.tab
    }
}

impl NavigationToken {
    pub fn view(&self) -> &ViewToken {
        &self.view
    }
    pub fn request(&self) -> &NavigationRequest {
        &self.request
    }
}

#[derive(Debug, PartialEq)]
pub enum StepError<E> {
    Owner(OwnerError),
    Backend(E),
    Retired,
    StaleCallback,
}

struct ViewRecord {
    token: ViewToken,
    navigation: Option<NavigationToken>,
}

/// Owns the local core and one backend by value. No engine/owner mutable escape
/// hatch, raw actor constructor, deferred input queue, or source-enabling switch.
/// This is host-testable control flow, NOT an actual ordinary Servo constructor.
pub struct AlbertEngineStep<E: EngineBackend> {
    owner: AlbertRuntimeOwner,
    attachment: Attachment,
    backend: Option<E>,
    views: Vec<ViewRecord>,
}

impl<E: EngineBackend> AlbertEngineStep<E> {
    /// Caller must already have acquired and verified real resources and the
    /// live Session. `binding` is only their local routing record.
    pub fn new(binding: SurfaceBinding, backend: E) -> Result<Self, StepError<E::Error>> {
        let mut owner = AlbertRuntimeOwner::new();
        let attachment = owner.attach(binding).map_err(StepError::Owner)?;
        let mut result = Self {
            owner,
            attachment,
            backend: Some(backend),
            views: Vec::new(),
        };
        result.sync_views()?;
        Ok(result)
    }

    pub fn view(&self) -> BrowserChromeView {
        self.owner.view()
    }

    fn backend_call<T>(
        &mut self,
        call: impl FnOnce(&mut E) -> Result<T, E::Error>,
    ) -> Result<T, StepError<E::Error>> {
        let result = call(self.backend.as_mut().ok_or(StepError::Retired)?);
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                // No more backend calls after an uncertain failure, including
                // a failed clear_input. Drop must release its owned resources.
                self.owner.close();
                self.views.clear();
                self.backend.take();
                Err(StepError::Backend(error))
            }
        }
    }

    fn live(&self) -> Result<(), StepError<E::Error>> {
        self.backend.as_ref().map(|_| ()).ok_or(StepError::Retired)
    }

    /// Trusted lifecycle must be applied before any further input or step.
    /// Replacement/resize/session/workspace change requires retirement and a
    /// new verified backend; numeric identity reuse cannot revive old tokens.
    /// Explicit teardown returns input-release errors for diagnostics. Resources
    /// are dropped even on error. Drop performs the same cleanup best-effort.
    pub fn retire(&mut self) -> Result<(), E::Error> {
        self.owner.close();
        self.views.clear();
        // Invalidate local state BEFORE engine/resource destruction callbacks.
        // Backend Drop must revoke guest resources, not run a nested event loop.
        if let Some(mut backend) = self.backend.take() {
            // Best-effort input release once, then unconditionally drop all
            // resources. Cleanup failure must never preserve a dispatchable
            // backend or retry the operation that triggered retirement.
            return backend.clear_input();
        }
        Ok(())
    }

    pub fn set_focus(&mut self, focused: bool) -> Result<(), StepError<E::Error>> {
        self.live()?;
        self.owner
            .set_focus(&self.attachment, focused)
            .map_err(StepError::Owner)?;
        self.backend_call(|engine| engine.clear_input())
    }

    fn sync_views(&mut self) -> Result<(), StepError<E::Error>> {
        let state = self.owner.view();
        let removed: Vec<_> = self
            .views
            .iter()
            .filter(|record| !state.tabs.iter().any(|tab| tab.id == record.token.tab))
            .map(|record| record.token.clone())
            .collect();
        // Remove routing before calling the backend; late callbacks cannot map
        // onto a newly created view even if the numeric tab ID is reused.
        self.views
            .retain(|record| state.tabs.iter().any(|tab| tab.id == record.token.tab));
        for token in removed {
            self.backend_call(|engine| engine.remove_view(&token))?;
        }
        for tab in &state.tabs {
            if self.views.iter().any(|record| record.token.tab == tab.id) {
                continue;
            }
            let token = ViewToken {
                identity: Rc::new(()),
                tab: tab.id,
            };
            self.backend_call(|engine| engine.create_view(&token))?;
            self.views.push(ViewRecord {
                token,
                navigation: None,
            });
        }
        if let Some(active) = state.active_tab {
            let token = self
                .views
                .iter()
                .find(|record| record.token.tab == active)
                .unwrap()
                .token
                .clone();
            self.backend_call(|engine| engine.activate(&token))?;
        }
        Ok(())
    }

    fn route_outcome(&mut self, outcome: BrowserChromeOutcome) -> Result<(), StepError<E::Error>> {
        self.sync_views()?;
        match outcome {
            BrowserChromeOutcome::Changed => Ok(()),
            BrowserChromeOutcome::Navigate(request) => {
                let record = self
                    .views
                    .iter_mut()
                    .find(|record| record.token.tab == request.tab_id)
                    .ok_or(StepError::StaleCallback)?;
                let token = NavigationToken {
                    view: record.token.clone(),
                    identity: Rc::new(()),
                    request,
                };
                record.navigation = Some(token.clone());
                self.backend_call(|engine| engine.navigate(&token))
            }
            BrowserChromeOutcome::Stop(StopRequest { tab_id, .. }) => {
                let record = self
                    .views
                    .iter_mut()
                    .find(|record| record.token.tab == tab_id)
                    .ok_or(StepError::StaleCallback)?;
                record.navigation = None;
                let token = record.token.clone();
                self.backend_call(|engine| engine.stop(&token))
            }
        }
    }

    pub fn chrome(
        &mut self,
        sequence: u64,
        action: BrowserChromeAction,
        now: u64,
    ) -> Result<(), StepError<E::Error>> {
        self.live()?;
        let result = self
            .owner
            .dispatch_chrome(&self.attachment, sequence, action, now);
        // Even an invalid chrome action can invalidate focus/page state. Clear
        // engine input before reporting it; never strand a key in another tab.
        self.backend_call(|engine| engine.clear_input())?;
        self.route_outcome(result.map_err(StepError::Owner)?)
    }

    fn deliver(&mut self, outcome: InputOutcome) -> Result<(), StepError<E::Error>> {
        match outcome {
            InputOutcome::Chrome(outcome) => {
                self.backend_call(|engine| engine.clear_input())?;
                self.route_outcome(outcome)
            }
            InputOutcome::Page { ticket, input } => {
                self.owner
                    .validate_page_input(&ticket)
                    .map_err(StepError::Owner)?;
                let token = self
                    .views
                    .iter()
                    .find(|record| record.token.tab == ticket.tab_id())
                    .ok_or(StepError::StaleCallback)?
                    .token
                    .clone();
                // Synchronous, by-value delivery, with no yield after validation.
                self.backend_call(|engine| engine.input(&token, input))
            }
        }
    }

    pub fn primary_press(
        &mut self,
        sequence: u64,
        x: u32,
        y: u32,
        now: u64,
    ) -> Result<(), StepError<E::Error>> {
        self.live()?;
        let outcome = self
            .owner
            .primary_press(&self.attachment, sequence, x, y, now);
        match outcome {
            Ok(outcome) => self.deliver(outcome),
            Err(error) => {
                self.backend_call(|engine| engine.clear_input())?;
                Err(StepError::Owner(error))
            }
        }
    }

    pub fn page_key(
        &mut self,
        sequence: u64,
        code: u16,
        pressed: bool,
    ) -> Result<(), StepError<E::Error>> {
        self.live()?;
        let outcome = self
            .owner
            .page_key(&self.attachment, sequence, code, pressed)
            .map_err(StepError::Owner)?;
        self.deliver(outcome)
    }

    pub fn step(&mut self) -> Result<bool, StepError<E::Error>> {
        self.backend_call(|engine| engine.spin_once())
    }

    pub fn validate_view(&self, view: &ViewToken) -> Result<(), StepError<E::Error>> {
        self.live()?;
        if !self
            .views
            .iter()
            .any(|record| Rc::ptr_eq(&record.token.identity, &view.identity))
        {
            return Err(StepError::StaleCallback);
        }
        Ok(())
    }

    /// Use only callbacks whose engine mapping proves the original request.
    /// Pinned Servo callbacks lacking load IDs cannot be assigned to the latest
    /// pending request by URL or by looking up only a current WebView/tab ID.
    pub fn complete_navigation(
        &mut self,
        token: &NavigationToken,
        title: &str,
        now: u64,
    ) -> Result<NavigationEventResult, StepError<E::Error>> {
        self.validate_navigation(token)?;
        let record = self
            .views
            .iter_mut()
            .find(|record| Rc::ptr_eq(&record.token.identity, &token.view.identity))
            .unwrap();
        // Consume before applying; duplicate completion cannot be retried.
        record.navigation = None;
        self.owner
            .complete_navigation(&self.attachment, &token.request, title, now)
            .map_err(StepError::Owner)
    }

    fn validate_navigation(&self, token: &NavigationToken) -> Result<(), StepError<E::Error>> {
        self.validate_view(&token.view)?;
        let record = self
            .views
            .iter()
            .find(|record| Rc::ptr_eq(&record.token.identity, &token.view.identity))
            .unwrap();
        if !record
            .navigation
            .as_ref()
            .is_some_and(|pending| Rc::ptr_eq(&pending.identity, &token.identity))
        {
            return Err(StepError::StaleCallback);
        }
        Ok(())
    }

    pub fn redirect_navigation(
        &mut self,
        token: &NavigationToken,
        url: &str,
    ) -> Result<NavigationEventResult, StepError<E::Error>> {
        self.validate_navigation(token)?;
        self.owner
            .redirect_navigation(&self.attachment, &token.request, url)
            .map_err(StepError::Owner)
    }

    pub fn fail_navigation(
        &mut self,
        token: &NavigationToken,
        message: &str,
    ) -> Result<NavigationEventResult, StepError<E::Error>> {
        self.validate_navigation(token)?;
        let record = self
            .views
            .iter_mut()
            .find(|record| Rc::ptr_eq(&record.token.identity, &token.view.identity))
            .unwrap();
        record.navigation = None;
        self.owner
            .fail_navigation(&self.attachment, &token.request, message)
            .map_err(StepError::Owner)
    }

    /// Source-initiated navigation must first invalidate with page_will_change
    /// before the engine mutates the document, then pass that returned ticket.
    pub fn content_navigated(
        &mut self,
        ticket: &PageTicket,
        url: &str,
        title: &str,
        now: u64,
    ) -> Result<NavigationEventResult, StepError<E::Error>> {
        self.validate_page(ticket)?;
        let result = self
            .owner
            .content_navigated(ticket, url, title, now)
            .map_err(StepError::Owner)?;
        if result == NavigationEventResult::Applied {
            let record = self
                .views
                .iter_mut()
                .find(|record| record.token.tab == ticket.tab_id())
                .ok_or(StepError::StaleCallback)?;
            record.navigation = None;
        }
        Ok(result)
    }

    /// Capture BEFORE asynchronous page/frame work; never capture a new ticket
    /// to label bytes/results produced under an older generation.
    pub fn page_ticket(&self, view: &ViewToken) -> Result<PageTicket, StepError<E::Error>> {
        self.validate_view(view)?;
        let ticket = self
            .owner
            .page_ticket(&self.attachment)
            .map_err(StepError::Owner)?;
        if ticket.tab_id() != view.tab {
            return Err(StepError::StaleCallback);
        }
        Ok(ticket)
    }

    pub fn page_will_change(
        &mut self,
        ticket: &PageTicket,
    ) -> Result<PageTicket, StepError<E::Error>> {
        self.live()?;
        self.owner
            .page_will_change(ticket)
            .map_err(StepError::Owner)
    }

    /// Caller still must validate the same ticket immediately before real
    /// Surface presentation. This function only composes caller-owned buffers.
    pub fn compose_page(
        &self,
        ticket: &PageTicket,
        frame: &mut [u8],
        stride: usize,
        page: &[u8],
    ) -> Result<(), StepError<E::Error>> {
        self.live()?;
        self.owner
            .compose_page(ticket, frame, stride, page)
            .map_err(StepError::Owner)
    }

    pub fn validate_page(&self, ticket: &PageTicket) -> Result<(), StepError<E::Error>> {
        self.live()?;
        self.owner.validate_page(ticket).map_err(StepError::Owner)
    }
}

impl<E: EngineBackend> Drop for AlbertEngineStep<E> {
    fn drop(&mut self) {
        let _ = self.retire();
    }
}

impl<E: EngineBackend> PublicBrowserContextApi for AlbertEngineStep<E> {
    fn current_page_context(
        &mut self,
        _: BrowserContextRequest,
    ) -> Result<Option<BrowserPageSnapshot>, BrowserContextApiError> {
        Err(BrowserContextApiError::Unavailable)
    }
}

#[cfg(test)]
#[path = "engine_step_tests.rs"]
mod tests;
