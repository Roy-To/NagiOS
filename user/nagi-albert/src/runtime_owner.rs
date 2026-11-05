//! Event-thread control core for a future ordinary Albert embedder.
//!
//! Uses the existing browser/chrome and revocable context state. Local handles
//! fence routing and freshness; they are NOT kernel capabilities, authenticated
//! IPC actors, display handles or consent. No production source adapter exists
//! yet, so the public Context API fails closed even after local attachment.

use std::rc::Rc;

use nagi_ai::{
    BrowserContextApiError, BrowserContextRequest, BrowserPageSnapshot, PublicBrowserContextApi,
};
use nagi_model::{AppSessionId, NodeId, WorkspaceId};

use crate::assistant_context::BrowserContextOwner;
use crate::browser_state::{BrowserState, BrowserStateError, NavigationEventResult};
use crate::chrome_surface::{place_page, render_chrome, ChromeRenderError, PAGE_TOP};
use crate::input::chrome_action_at;
use crate::navigation::NavigationRequest;
use crate::tabs::TabId;
use crate::ui::{self, BrowserChromeAction, BrowserChromeOutcome, BrowserChromeView};

/// Routing metadata supplied by the trusted local embedder AFTER it validates
/// its real resources/session. Copying these values never grants authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceBinding {
    pub app_session_id: AppSessionId,
    pub node_id: NodeId,
    pub workspace_id: Option<WorkspaceId>,
    pub session_epoch: u64,
    pub surface_id: u64,
    pub width: u32,
    pub height: u32,
}

/// Opaque local attachment identity, unique even across identical detach/attach
/// or owner recreation. Event-thread only. No raw-identity constructor.
#[derive(Clone, Debug)]
pub struct Attachment {
    identity: Rc<()>,
}

/// A callback/extraction ticket for precisely one active document generation.
/// Obtain a new ticket after every invalidating event. Keeping a ticket does
/// not keep an attachment or page alive.
#[derive(Clone, Debug)]
pub struct PageTicket {
    attachment: Attachment,
    document: Rc<()>,
    tab_id: TabId,
}

impl PageTicket {
    /// Engine routing key only; validate the ticket before using this tab.
    pub fn tab_id(&self) -> TabId {
        self.tab_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OwnerError {
    Closed,
    InvalidBinding,
    AlreadyAttached,
    StaleAttachment,
    StalePage,
    NotFocused,
    ChromeFocused,
    RepeatedInput,
    OutsideSurface,
    Browser(BrowserStateError),
    Render(ChromeRenderError),
}

/// Data for the real engine input adapter; never a host input event source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PageInput {
    PrimaryPress { x: u32, y: u32 },
    Key { code: u16, pressed: bool },
}

#[derive(Clone, Debug)]
pub enum InputOutcome {
    Chrome(BrowserChromeOutcome),
    Page {
        ticket: PageTicket,
        input: PageInput,
    },
}

struct Attached {
    binding: SurfaceBinding,
    handle: Attachment,
    focused: bool,
    last_input: u64,
}

/// Owns browser state, focus, attachment lifetime and the context epoch together.
/// Mutable browser/context access is deliberately not exported: all supported
/// state mutations invalidate before changing the underlying BrowserState.
/// This control core does not construct Servo, acquire a Surface, dispatch IPC,
/// authenticate a Session or launch an assistant/model.
pub struct AlbertRuntimeOwner {
    browser: BrowserState,
    context: BrowserContextOwner,
    attached: Option<Attached>,
    document: Rc<()>,
    closed: bool,
}

impl Default for AlbertRuntimeOwner {
    fn default() -> Self {
        Self::new()
    }
}

impl AlbertRuntimeOwner {
    pub fn new() -> Self {
        Self {
            browser: BrowserState::new(),
            context: BrowserContextOwner::new(),
            attached: None,
            document: Rc::new(()),
            closed: false,
        }
    }

    pub fn view(&self) -> BrowserChromeView {
        ui::view(&self.browser)
    }

    /// Register local routing only. The integrator must retain and validate the
    /// actual display/input capabilities outside this metadata-only core.
    /// Attachment always starts unfocused, with sharing disabled and fresh tabs.
    pub fn attach(&mut self, binding: SurfaceBinding) -> Result<Attachment, OwnerError> {
        if self.closed {
            return Err(OwnerError::Closed);
        }
        if self.attached.is_some() {
            return Err(OwnerError::AlreadyAttached);
        }
        if binding.app_session_id.0 == 0
            || binding.node_id.0 == 0
            || binding.workspace_id.is_some_and(|id| id.0 == 0)
            || binding.session_epoch == 0
            || binding.surface_id == 0
            || binding.width < 80
            || binding.height <= PAGE_TOP
        {
            return Err(OwnerError::InvalidBinding);
        }
        self.reset();
        let handle = Attachment {
            identity: Rc::new(()),
        };
        self.attached = Some(Attached {
            binding,
            handle: handle.clone(),
            focused: false,
            last_input: 0,
        });
        Ok(handle)
    }

    fn attached(&self, handle: &Attachment) -> Result<&Attached, OwnerError> {
        if self.closed {
            return Err(OwnerError::Closed);
        }
        self.attached
            .as_ref()
            .filter(|attached| Rc::ptr_eq(&handle.identity, &attached.handle.identity))
            .ok_or(OwnerError::StaleAttachment)
    }

    /// Freshness comparison for a connector's retained local resource record.
    /// This does not authenticate that record or its caller.
    pub fn validate_binding(
        &self,
        handle: &Attachment,
        binding: SurfaceBinding,
    ) -> Result<(), OwnerError> {
        if self.attached(handle)?.binding != binding {
            return Err(OwnerError::InvalidBinding);
        }
        Ok(())
    }

    fn invalidate(&mut self) {
        self.context.invalidate_page_context();
        self.document = Rc::new(());
    }

    fn reset(&mut self) {
        self.invalidate();
        // Drop the old epoch, including retained callback notifiers. Neither
        // sharing nor browser contents may cross surface/session boundaries.
        self.context = BrowserContextOwner::new();
        self.browser = BrowserState::new();
    }

    pub fn detach(&mut self, handle: &Attachment) -> Result<(), OwnerError> {
        self.attached(handle)?;
        self.reset();
        self.attached = None;
        Ok(())
    }

    /// The trusted session owner must deliver lock/signout before any more
    /// dispatch. Unlock requires a new attachment and a fresh focus decision.
    pub fn lock(&mut self) {
        self.reset();
        self.attached = None;
    }

    pub fn sign_out(&mut self) {
        self.lock();
    }

    /// Terminal teardown; this owner can never attach again.
    pub fn close(&mut self) {
        self.lock();
        self.closed = true;
    }

    pub fn set_focus(&mut self, handle: &Attachment, focused: bool) -> Result<(), OwnerError> {
        self.attached(handle)?;
        self.invalidate();
        self.attached.as_mut().unwrap().focused = focused;
        if !focused {
            // Cancel address preedit on focus loss rather than committing it
            // into another session/page. Keep the ordinary address focus flag.
            if self.browser.address_bar().is_composing() {
                self.browser
                    .address_bar_mut()
                    .cancel_composition()
                    .map_err(|error| OwnerError::Browser(BrowserStateError::Address(error)))?;
            }
        }
        Ok(())
    }

    /// Only a trusted chrome/settings action may call this. It conveys no
    /// terms acknowledgement, grant, workspace visibility or model admission.
    pub fn set_sharing_by_user(
        &mut self,
        handle: &Attachment,
        enabled: bool,
    ) -> Result<(), OwnerError> {
        self.require_focus(handle)?;
        self.invalidate();
        self.context.set_sharing_by_user(enabled);
        Ok(())
    }

    fn require_focus(&self, handle: &Attachment) -> Result<(), OwnerError> {
        if !self.attached(handle)?.focused {
            return Err(OwnerError::NotFocused);
        }
        Ok(())
    }

    fn consume_input(&mut self, handle: &Attachment, sequence: u64) -> Result<(), OwnerError> {
        self.require_focus(handle)?;
        if sequence <= self.attached(handle)?.last_input {
            return Err(OwnerError::RepeatedInput);
        }
        // Record before executing even a rejected action: retries must not
        // duplicate navigation, tab creation, or page-side input effects.
        self.attached.as_mut().unwrap().last_input = sequence;
        Ok(())
    }

    pub fn dispatch_chrome(
        &mut self,
        handle: &Attachment,
        sequence: u64,
        action: BrowserChromeAction,
        now: u64,
    ) -> Result<BrowserChromeOutcome, OwnerError> {
        self.consume_input(handle, sequence)?;
        self.invalidate();
        ui::dispatch(&mut self.browser, action, now).map_err(OwnerError::Browser)
    }

    /// Hit testing uses the existing chrome layout. Page coordinates exclude
    /// chrome; a page press hands address-bar focus back to the page.
    pub fn primary_press(
        &mut self,
        handle: &Attachment,
        sequence: u64,
        x: u32,
        y: u32,
        now: u64,
    ) -> Result<InputOutcome, OwnerError> {
        self.consume_input(handle, sequence)?;
        let binding = self.attached(handle)?.binding;
        if x >= binding.width || y >= binding.height {
            return Err(OwnerError::OutsideSurface);
        }
        self.invalidate();
        if y < PAGE_TOP {
            let action = chrome_action_at(&self.view(), binding.width, x, y);
            return action.map_or(
                Ok(InputOutcome::Chrome(BrowserChromeOutcome::Changed)),
                |action| {
                    ui::dispatch(&mut self.browser, action, now)
                        .map(InputOutcome::Chrome)
                        .map_err(OwnerError::Browser)
                },
            );
        }
        self.browser.address_bar_mut().blur();
        Ok(InputOutcome::Page {
            ticket: self.page_ticket(handle)?,
            input: PageInput::PrimaryPress { x, y: y - PAGE_TOP },
        })
    }

    /// The chrome IME/key translator remains responsible for address input.
    /// Page key delivery is forbidden while the address bar owns focus.
    pub fn page_key(
        &mut self,
        handle: &Attachment,
        sequence: u64,
        code: u16,
        pressed: bool,
    ) -> Result<InputOutcome, OwnerError> {
        self.consume_input(handle, sequence)?;
        if self.browser.address_bar().is_focused() {
            return Err(OwnerError::ChromeFocused);
        }
        self.invalidate();
        Ok(InputOutcome::Page {
            ticket: self.page_ticket(handle)?,
            input: PageInput::Key { code, pressed },
        })
    }

    pub fn page_ticket(&self, handle: &Attachment) -> Result<PageTicket, OwnerError> {
        self.attached(handle)?;
        let tab_id = self.browser.active_tab_id().ok_or(OwnerError::StalePage)?;
        Ok(PageTicket {
            attachment: handle.clone(),
            document: self.document.clone(),
            tab_id,
        })
    }

    /// Validate again immediately before presenting/delivering deferred engine
    /// output. Tickets from other owners, tabs, sessions or generations fail.
    pub fn validate_page(&self, ticket: &PageTicket) -> Result<(), OwnerError> {
        self.attached(&ticket.attachment)?;
        if !Rc::ptr_eq(&ticket.document, &self.document)
            || self.browser.active_tab_id() != Some(ticket.tab_id)
        {
            return Err(OwnerError::StalePage);
        }
        Ok(())
    }

    /// Deferred input additionally needs live page focus at delivery time.
    pub fn validate_page_input(&self, ticket: &PageTicket) -> Result<(), OwnerError> {
        self.validate_page(ticket)?;
        self.require_focus(&ticket.attachment)?;
        if self.browser.address_bar().is_focused() {
            return Err(OwnerError::ChromeFocused);
        }
        Ok(())
    }

    /// Trusted engine callback BEFORE DOM/selection/page mutations. This
    /// invalidates all queued snapshots/output even if text is later restored.
    pub fn page_will_change(&mut self, ticket: &PageTicket) -> Result<PageTicket, OwnerError> {
        self.validate_page(ticket)?;
        self.invalidate();
        self.page_ticket(&ticket.attachment)
    }

    /// Compose an actual engine page buffer with the existing Albert chrome.
    /// The caller owns the destination and must validate the same ticket again
    /// immediately before its real Surface present. This performs no OS I/O.
    pub fn compose_page(
        &self,
        ticket: &PageTicket,
        frame: &mut [u8],
        stride: usize,
        page: &[u8],
    ) -> Result<(), OwnerError> {
        self.validate_page(ticket)?;
        let binding = self.attached(&ticket.attachment)?.binding;
        place_page(
            frame,
            binding.width,
            binding.height,
            stride,
            page,
            binding.height - PAGE_TOP,
        )
        .map_err(OwnerError::Render)?;
        render_chrome(frame, binding.width, binding.height, stride, &self.view())
            .map_err(OwnerError::Render)
    }

    fn pending_matches(&self, request: &NavigationRequest) -> bool {
        self.browser
            .tab(request.tab_id)
            .and_then(|tab| tab.pending_request())
            .is_some_and(|pending| pending.id == request.id)
    }

    pub fn redirect_navigation(
        &mut self,
        handle: &Attachment,
        request: &NavigationRequest,
        url: &str,
    ) -> Result<NavigationEventResult, OwnerError> {
        self.attached(handle)?;
        if !self.pending_matches(request) {
            return Ok(NavigationEventResult::IgnoredStale);
        }
        self.invalidate();
        self.browser
            .redirect(request.tab_id, request.id, url)
            .map_err(OwnerError::Browser)
    }

    pub fn fail_navigation(
        &mut self,
        handle: &Attachment,
        request: &NavigationRequest,
        message: &str,
    ) -> Result<NavigationEventResult, OwnerError> {
        self.attached(handle)?;
        if !self.pending_matches(request) {
            return Ok(NavigationEventResult::IgnoredStale);
        }
        self.invalidate();
        self.browser
            .fail_navigation(request.tab_id, request.id, message)
            .map_err(OwnerError::Browser)
    }

    /// A source-initiated navigation, using the existing normal state path.
    /// The engine must invalidate before mutation with page_will_change, then
    /// use its returned ticket for the changed document's callback.
    pub fn content_navigated(
        &mut self,
        ticket: &PageTicket,
        url: &str,
        title: &str,
        now: u64,
    ) -> Result<NavigationEventResult, OwnerError> {
        self.validate_page(ticket)?;
        self.invalidate();
        self.browser
            .content_navigated(ticket.tab_id, url, title, now)
            .map_err(OwnerError::Browser)
    }

    /// Navigation completion comes from the attached engine, not web payloads.
    /// Existing BrowserState navigation IDs reject old responses, including
    /// same-URL reloads. No callback can install a different session identity.
    pub fn complete_navigation(
        &mut self,
        handle: &Attachment,
        request: &NavigationRequest,
        title: &str,
        now: u64,
    ) -> Result<NavigationEventResult, OwnerError> {
        self.attached(handle)?;
        if !self.pending_matches(request) {
            return Ok(NavigationEventResult::IgnoredStale);
        }
        self.invalidate();
        self.browser
            .complete_navigation(request.tab_id, request.id, title, now)
            .map_err(OwnerError::Browser)
    }
}

impl PublicBrowserContextApi for AlbertRuntimeOwner {
    fn current_page_context(
        &mut self,
        _request: BrowserContextRequest,
    ) -> Result<Option<BrowserPageSnapshot>, BrowserContextApiError> {
        // No authenticated request/session connector or coherent ordinary Servo
        // source exists. Raw caller metadata and local attachment cannot enable
        // extraction. Ordinary Desktop must continue passing None.
        Err(BrowserContextApiError::Unavailable)
    }
}

#[cfg(test)]
#[path = "runtime_owner_tests.rs"]
mod tests;
