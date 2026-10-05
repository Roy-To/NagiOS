//! Nagi user-space clipboard service.
//!
//! The clipboard is an ordinary user-space service; the kernel has no
//! clipboard syscall. Authority is split in two:
//!
//! - a [`ClipboardEndpoint`] carries attenuable READ/WRITE rights for one
//!   registered client, and
//! - a [`GestureSource`] lets only that client's trusted input path record
//!   user gestures.
//!
//! Holding READ is not enough to read: a read also consumes a one-shot paste
//! gesture recorded for the same client and scope (for example one browser
//! tab). Writes require a recent activation gesture in that scope. Untrusted
//! content that can reach an endpoint but not the gesture source therefore
//! cannot silently read or overwrite the clipboard.

#![no_std]

extern crate alloc;

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

/// Largest UTF-8 clipboard payload accepted by the service.
pub const MAX_CLIPBOARD_TEXT_BYTES: usize = 64 * 1024;
/// Upper bound on concurrently registered clients.
pub const MAX_CLIPBOARD_CLIENTS: usize = 8;
/// Upper bound on live gesture records across all clients.
pub const MAX_GESTURE_SCOPES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipboardRights(u8);

impl ClipboardRights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const READ_WRITE: Self = Self(3);

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Keep only rights present in both sets. The result is never stronger
    /// than `self`.
    #[must_use]
    pub const fn attenuate(self, mask: Self) -> Self {
        Self(self.0 & mask.0)
    }
}

/// Gesture lifetimes in caller time units (Nagi timer ticks in the guest).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipboardPolicy {
    pub paste_grant_ticks: u64,
    pub activation_ticks: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ClientId(u32);

/// A client-defined gesture scope, such as one browser tab.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct GestureScope(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardError {
    /// The endpoint or client does not hold the required right.
    MissingRight,
    /// No unexpired user gesture authorizes the operation in this scope.
    NoUserGesture,
    /// The client was never registered or has been revoked.
    UnknownClient,
    TooLarge,
    ClientTableFull,
    GestureTableFull,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClipboardStats {
    pub reads: u64,
    pub writes: u64,
    pub denied_reads: u64,
    pub denied_writes: u64,
}

#[derive(Clone, Copy, Debug)]
struct ClientRecord {
    id: ClientId,
    rights: ClipboardRights,
}

#[derive(Clone, Copy, Debug)]
struct GestureRecord {
    client: ClientId,
    scope: GestureScope,
    paste_expires_at: Option<u64>,
    activation_expires_at: Option<u64>,
}

impl GestureRecord {
    fn is_live(&self, now: u64) -> bool {
        self.paste_expires_at.is_some_and(|at| now <= at)
            || self.activation_expires_at.is_some_and(|at| now <= at)
    }
}

#[derive(Debug)]
struct ClipboardService {
    policy: ClipboardPolicy,
    text: String,
    sequence: u64,
    clients: Vec<ClientRecord>,
    next_client: u32,
    gestures: Vec<GestureRecord>,
    stats: ClipboardStats,
}

impl ClipboardService {
    fn client_rights(&self, client: ClientId) -> Result<ClipboardRights, ClipboardError> {
        self.clients
            .iter()
            .find(|record| record.id == client)
            .map(|record| record.rights)
            .ok_or(ClipboardError::UnknownClient)
    }

    fn gesture_mut(
        &mut self,
        client: ClientId,
        scope: GestureScope,
        now: u64,
    ) -> Result<&mut GestureRecord, ClipboardError> {
        if let Some(index) = self
            .gestures
            .iter()
            .position(|record| record.client == client && record.scope == scope)
        {
            return Ok(&mut self.gestures[index]);
        }
        if self.gestures.len() == MAX_GESTURE_SCOPES {
            self.gestures.retain(|record| record.is_live(now));
        }
        if self.gestures.len() == MAX_GESTURE_SCOPES {
            return Err(ClipboardError::GestureTableFull);
        }
        self.gestures.push(GestureRecord {
            client,
            scope,
            paste_expires_at: None,
            activation_expires_at: None,
        });
        let last = self.gestures.len() - 1;
        Ok(&mut self.gestures[last])
    }

    fn take_paste_grant(&mut self, client: ClientId, scope: GestureScope, now: u64) -> bool {
        self.gestures
            .iter_mut()
            .find(|record| record.client == client && record.scope == scope)
            .and_then(|record| record.paste_expires_at.take())
            .is_some_and(|expires_at| now <= expires_at)
    }

    fn has_activation(&self, client: ClientId, scope: GestureScope, now: u64) -> bool {
        self.gestures.iter().any(|record| {
            record.client == client
                && record.scope == scope
                && record.activation_expires_at.is_some_and(|at| now <= at)
        })
    }

    fn authorize(
        &self,
        client: ClientId,
        endpoint_rights: ClipboardRights,
        required: ClipboardRights,
    ) -> Result<(), ClipboardError> {
        let rights = self.client_rights(client)?;
        if rights.attenuate(endpoint_rights).contains(required) {
            Ok(())
        } else {
            Err(ClipboardError::MissingRight)
        }
    }
}

/// Owner handle for the clipboard service. Only the component that starts
/// the service (init) should hold it; it can register and revoke clients.
#[derive(Clone, Debug)]
pub struct ClipboardServiceOwner {
    service: Rc<RefCell<ClipboardService>>,
}

impl ClipboardServiceOwner {
    pub fn new(policy: ClipboardPolicy) -> Self {
        Self {
            service: Rc::new(RefCell::new(ClipboardService {
                policy,
                text: String::new(),
                sequence: 0,
                clients: Vec::new(),
                next_client: 1,
                gestures: Vec::new(),
                stats: ClipboardStats::default(),
            })),
        }
    }

    /// Register a client with at most `rights`, returning its endpoint and
    /// the gesture source for its trusted input path.
    pub fn register_client(
        &self,
        rights: ClipboardRights,
    ) -> Result<(ClipboardEndpoint, GestureSource), ClipboardError> {
        let mut service = self.service.borrow_mut();
        if service.clients.len() == MAX_CLIPBOARD_CLIENTS {
            return Err(ClipboardError::ClientTableFull);
        }
        let id = ClientId(service.next_client);
        service.next_client = service.next_client.wrapping_add(1).max(1);
        service.clients.push(ClientRecord { id, rights });
        drop(service);
        Ok((
            ClipboardEndpoint {
                service: Rc::clone(&self.service),
                client: id,
                rights,
            },
            GestureSource {
                service: Rc::clone(&self.service),
                client: id,
            },
        ))
    }

    /// Revoke a client. Its endpoints and gesture source stop working.
    pub fn revoke_client(&self, client: ClientId) {
        let mut service = self.service.borrow_mut();
        service.clients.retain(|record| record.id != client);
        service.gestures.retain(|record| record.client != client);
    }

    pub fn sequence(&self) -> u64 {
        self.service.borrow().sequence
    }

    pub fn stats(&self) -> ClipboardStats {
        self.service.borrow().stats
    }
}

/// A rights-bearing connection to the clipboard service.
#[derive(Clone, Debug)]
pub struct ClipboardEndpoint {
    service: Rc<RefCell<ClipboardService>>,
    client: ClientId,
    rights: ClipboardRights,
}

impl ClipboardEndpoint {
    pub fn client(&self) -> ClientId {
        self.client
    }

    pub fn rights(&self) -> ClipboardRights {
        self.rights
    }

    /// Derive an endpoint whose rights are the intersection with `mask`.
    #[must_use]
    pub fn attenuate(&self, mask: ClipboardRights) -> Self {
        Self {
            service: Rc::clone(&self.service),
            client: self.client,
            rights: self.rights.attenuate(mask),
        }
    }

    /// Read text, consuming the scope's one-shot paste gesture.
    pub fn read_text(&self, scope: GestureScope, now: u64) -> Result<String, ClipboardError> {
        let mut service = self.service.borrow_mut();
        let result = service
            .authorize(self.client, self.rights, ClipboardRights::READ)
            .and_then(|()| {
                if service.take_paste_grant(self.client, scope, now) {
                    Ok(service.text.clone())
                } else {
                    Err(ClipboardError::NoUserGesture)
                }
            });
        match result {
            Ok(_) => service.stats.reads += 1,
            Err(_) => service.stats.denied_reads += 1,
        }
        result
    }

    /// Replace the clipboard text. Returns the new clipboard sequence number.
    pub fn write_text(
        &self,
        scope: GestureScope,
        text: &str,
        now: u64,
    ) -> Result<u64, ClipboardError> {
        self.replace(scope, text, now)
    }

    pub fn clear(&self, scope: GestureScope, now: u64) -> Result<u64, ClipboardError> {
        self.replace(scope, "", now)
    }

    fn replace(&self, scope: GestureScope, text: &str, now: u64) -> Result<u64, ClipboardError> {
        let mut service = self.service.borrow_mut();
        let result = service
            .authorize(self.client, self.rights, ClipboardRights::WRITE)
            .and_then(|()| {
                if text.len() > MAX_CLIPBOARD_TEXT_BYTES {
                    Err(ClipboardError::TooLarge)
                } else if service.has_activation(self.client, scope, now) {
                    Ok(())
                } else {
                    Err(ClipboardError::NoUserGesture)
                }
            });
        match result {
            Ok(()) => {
                service.text.clear();
                service.text.push_str(text);
                service.sequence = service.sequence.wrapping_add(1);
                service.stats.writes += 1;
                Ok(service.sequence)
            }
            Err(error) => {
                service.stats.denied_writes += 1;
                Err(error)
            }
        }
    }
}

/// Records user gestures for one client. Hand this only to the client's
/// trusted input path, never to the content it hosts.
#[derive(Clone, Debug)]
pub struct GestureSource {
    service: Rc<RefCell<ClipboardService>>,
    client: ClientId,
}

impl GestureSource {
    /// A fresh trusted input event (key press or click) reached `scope`.
    pub fn record_activation(&self, scope: GestureScope, now: u64) -> Result<(), ClipboardError> {
        let mut service = self.service.borrow_mut();
        service.client_rights(self.client)?;
        let window = service.policy.activation_ticks;
        let record = service.gesture_mut(self.client, scope, now)?;
        record.activation_expires_at = Some(now.saturating_add(window));
        Ok(())
    }

    /// The user explicitly asked to paste into `scope`. Authorizes exactly
    /// one read before it expires; it also counts as activation.
    pub fn record_paste(&self, scope: GestureScope, now: u64) -> Result<(), ClipboardError> {
        let mut service = self.service.borrow_mut();
        service.client_rights(self.client)?;
        let policy = service.policy;
        let record = service.gesture_mut(self.client, scope, now)?;
        record.paste_expires_at = Some(now.saturating_add(policy.paste_grant_ticks));
        record.activation_expires_at = Some(now.saturating_add(policy.activation_ticks));
        Ok(())
    }

    /// Drop any pending gestures for `scope`, for example after navigation.
    pub fn forget_scope(&self, scope: GestureScope) {
        self.service
            .borrow_mut()
            .gestures
            .retain(|record| record.client != self.client || record.scope != scope);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    const POLICY: ClipboardPolicy = ClipboardPolicy {
        paste_grant_ticks: 10,
        activation_ticks: 50,
    };
    const TAB_A: GestureScope = GestureScope(1);
    const TAB_B: GestureScope = GestureScope(2);

    fn connected() -> (ClipboardServiceOwner, ClipboardEndpoint, GestureSource) {
        let owner = ClipboardServiceOwner::new(POLICY);
        let (endpoint, gestures) = owner.register_client(ClipboardRights::READ_WRITE).unwrap();
        (owner, endpoint, gestures)
    }

    #[test]
    fn copy_then_paste_round_trips_with_user_gestures() {
        let (owner, endpoint, gestures) = connected();
        gestures.record_activation(TAB_A, 100).unwrap();
        assert_eq!(endpoint.write_text(TAB_A, "なぎ copy", 101), Ok(1));
        gestures.record_paste(TAB_A, 102).unwrap();
        assert_eq!(endpoint.read_text(TAB_A, 103).unwrap(), "なぎ copy");
        assert_eq!(owner.sequence(), 1);
        assert_eq!(owner.stats().reads, 1);
        assert_eq!(owner.stats().writes, 1);
    }

    #[test]
    fn reads_without_a_paste_gesture_are_denied() {
        let (owner, endpoint, gestures) = connected();
        assert_eq!(
            endpoint.read_text(TAB_A, 1),
            Err(ClipboardError::NoUserGesture)
        );
        // Activation (a click) is not consent to read.
        gestures.record_activation(TAB_A, 2).unwrap();
        assert_eq!(
            endpoint.read_text(TAB_A, 3),
            Err(ClipboardError::NoUserGesture)
        );
        assert_eq!(owner.stats().denied_reads, 2);
    }

    #[test]
    fn paste_gesture_is_one_shot_and_expires() {
        let (_owner, endpoint, gestures) = connected();
        gestures.record_paste(TAB_A, 10).unwrap();
        assert!(endpoint.read_text(TAB_A, 11).is_ok());
        assert_eq!(
            endpoint.read_text(TAB_A, 12),
            Err(ClipboardError::NoUserGesture)
        );
        gestures.record_paste(TAB_A, 20).unwrap();
        assert_eq!(
            endpoint.read_text(TAB_A, 20 + POLICY.paste_grant_ticks + 1),
            Err(ClipboardError::NoUserGesture)
        );
    }

    #[test]
    fn gestures_do_not_cross_scopes_or_clients() {
        let owner = ClipboardServiceOwner::new(POLICY);
        let (first, first_gestures) = owner.register_client(ClipboardRights::READ_WRITE).unwrap();
        let (second, _) = owner.register_client(ClipboardRights::READ_WRITE).unwrap();
        first_gestures.record_paste(TAB_A, 1).unwrap();
        assert_eq!(
            first.read_text(TAB_B, 2),
            Err(ClipboardError::NoUserGesture)
        );
        assert_eq!(
            second.read_text(TAB_A, 2),
            Err(ClipboardError::NoUserGesture)
        );
        assert_eq!(
            second.write_text(TAB_A, "x", 2),
            Err(ClipboardError::NoUserGesture)
        );
        assert!(first.read_text(TAB_A, 3).is_ok());
    }

    #[test]
    fn writes_require_recent_activation() {
        let (owner, endpoint, gestures) = connected();
        assert_eq!(
            endpoint.write_text(TAB_A, "silent", 1),
            Err(ClipboardError::NoUserGesture)
        );
        gestures.record_activation(TAB_A, 10).unwrap();
        assert!(endpoint
            .write_text(TAB_A, "ok", 10 + POLICY.activation_ticks)
            .is_ok());
        assert_eq!(
            endpoint.write_text(TAB_A, "late", 11 + POLICY.activation_ticks),
            Err(ClipboardError::NoUserGesture)
        );
        assert_eq!(owner.stats().denied_writes, 2);
    }

    #[test]
    fn attenuation_never_strengthens_rights() {
        let owner = ClipboardServiceOwner::new(POLICY);
        let (read_only, gestures) = owner.register_client(ClipboardRights::READ).unwrap();
        let widened = read_only.attenuate(ClipboardRights::READ_WRITE);
        assert_eq!(widened.rights(), ClipboardRights::READ);
        gestures.record_activation(TAB_A, 1).unwrap();
        assert_eq!(
            widened.write_text(TAB_A, "x", 2),
            Err(ClipboardError::MissingRight)
        );

        let (full, full_gestures) = owner.register_client(ClipboardRights::READ_WRITE).unwrap();
        let write_only = full.attenuate(ClipboardRights::WRITE);
        full_gestures.record_paste(TAB_A, 3).unwrap();
        assert_eq!(
            write_only.read_text(TAB_A, 4),
            Err(ClipboardError::MissingRight)
        );
        // The denied attenuated read did not consume the grant.
        assert!(full.read_text(TAB_A, 5).is_ok());
    }

    #[test]
    fn revoked_clients_lose_all_authority() {
        let (owner, endpoint, gestures) = connected();
        owner.revoke_client(endpoint.client());
        assert_eq!(
            gestures.record_paste(TAB_A, 1),
            Err(ClipboardError::UnknownClient)
        );
        assert_eq!(
            endpoint.read_text(TAB_A, 1),
            Err(ClipboardError::UnknownClient)
        );
    }

    #[test]
    fn oversized_text_is_rejected_without_changing_contents() {
        let (_owner, endpoint, gestures) = connected();
        gestures.record_activation(TAB_A, 1).unwrap();
        endpoint.write_text(TAB_A, "kept", 1).unwrap();
        let oversized = "a".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1);
        assert_eq!(
            endpoint.write_text(TAB_A, &oversized, 2),
            Err(ClipboardError::TooLarge)
        );
        gestures.record_paste(TAB_A, 3).unwrap();
        assert_eq!(endpoint.read_text(TAB_A, 4).unwrap(), "kept");
    }

    #[test]
    fn forgetting_a_scope_cancels_its_pending_paste() {
        let (_owner, endpoint, gestures) = connected();
        gestures.record_paste(TAB_A, 1).unwrap();
        gestures.forget_scope(TAB_A);
        assert_eq!(
            endpoint.read_text(TAB_A, 2),
            Err(ClipboardError::NoUserGesture)
        );
    }

    #[test]
    fn gesture_and_client_tables_are_bounded() {
        let (owner, _endpoint, gestures) = connected();
        for scope in 0..MAX_GESTURE_SCOPES as u64 {
            gestures.record_activation(GestureScope(scope), 0).unwrap();
        }
        assert_eq!(
            gestures.record_activation(GestureScope(999), 1),
            Err(ClipboardError::GestureTableFull)
        );
        // Once earlier gestures expire, their slots are reclaimed.
        assert!(gestures
            .record_activation(GestureScope(999), POLICY.activation_ticks + 1)
            .is_ok());

        for _ in 1..MAX_CLIPBOARD_CLIENTS {
            owner.register_client(ClipboardRights::READ).unwrap();
        }
        assert_eq!(
            owner.register_client(ClipboardRights::READ).map(|_| ()),
            Err(ClipboardError::ClientTableFull)
        );
    }
}
