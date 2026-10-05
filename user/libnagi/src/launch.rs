//! Supervisor launch registry (ADR 0046).
//!
//! The Supervisor is the only component that binds a kernel Process ID to an
//! application identity:
//!
//! - **Manifests.** An application may be launched only if a manifest
//!   declares it. The manifest's `app=` identifier is the sole source of its
//!   `AppId`, and its `grant=` lines name the capabilities it may exercise.
//! - **Launches.** Each launch binds `ProcessId -> (AppId, AppSessionId,
//!   NodeId, WorkspaceId)` until the process exits.
//! - **Grants.** A grant is effective only while a live launch holds that
//!   exact application session. Exiting revokes it, and a payload can never
//!   name or strengthen it.
//!
//! - **Consent (ADR 0051).** A manifest grant is only a *request*. It
//!   becomes effective once an authenticated, unlocked user decides `Allow`,
//!   or `AllowOnce` for one live session. The default is `Ask`, and `Deny`
//!   overrides the manifest. Neither the launched process, Developer Mode,
//!   nor the Owner role can supply a decision on the user's behalf.
//!
//! The registry is bounded, allocation-free, and has no kernel dependency, so
//! its policy is host-testable.

use crate::security::Session;
use nagi_model::{AppId, AppSessionId, NodeId, WorkspaceId};

pub const MAX_APP_MANIFESTS: usize = 8;
pub const MAX_LIVE_LAUNCHES: usize = 4;
pub const MAX_APP_GRANTS: usize = 8;
pub const MAX_APP_IDENTIFIER: usize = 64;
pub const MAX_GRANT_NAME: usize = 32;
pub const MAX_APP_MANIFEST_BYTES: usize = 1024;
/// Recorded user decisions, one per (application, capability).
pub const MAX_CONSENT_DECISIONS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchError {
    InvalidManifest,
    DuplicateManifest,
    ManifestTableFull,
    /// A manifest for this AppId with different grants is already registered.
    ConflictingManifest,
    UnknownApplication,
    ProcessAlreadyLaunched,
    SessionAlreadyLive,
    LaunchTableFull,
    InitProcess,
    /// No authenticated, unlocked user session is present to decide.
    ConsentUnavailable,
    /// An `AllowOnce` decision named a session that is not live.
    SessionNotLive,
    ConsentTableFull,
}

/// A user's decision for one (application, capability) pair (spec §23).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrantDecision {
    /// Withdraw any earlier decision; use of the capability needs a prompt.
    Ask,
    Allow,
    /// Allow only the named live session; dropped when that session exits.
    AllowOnce(AppSessionId),
    Deny,
}

/// Why a capability is or is not effective for a session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrantCheck {
    Granted,
    /// No live launch holds this application session.
    NotLive,
    /// The application's signed manifest does not request the capability.
    NotDeclared,
    /// Requested, but the user has not decided (or `AllowOnce` names
    /// another session). A trusted OS dialog would ask; until then it fails
    /// closed.
    ConsentRequired,
    Denied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Name<const N: usize> {
    bytes: [u8; N],
    length: u8,
}

impl<const N: usize> Name<N> {
    const EMPTY: Self = Self {
        bytes: [0; N],
        length: 0,
    };

    /// Accept lowercase reverse-DNS style identifiers and capability names:
    /// ASCII letters, digits, `.`, `-` and `_`, starting with a letter.
    fn parse(value: &[u8]) -> Result<Self, LaunchError> {
        let valid = !value.is_empty()
            && value.len() <= N
            && value.len() <= u8::MAX as usize
            && value[0].is_ascii_lowercase()
            && value.iter().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'-' | b'_')
            });
        if !valid {
            return Err(LaunchError::InvalidManifest);
        }
        let mut name = Self::EMPTY;
        name.bytes[..value.len()].copy_from_slice(value);
        name.length = value.len() as u8;
        Ok(name)
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length as usize]
    }
}

/// Supervisor-owned declaration of one launchable application.
///
/// Text format, one `key=value` per line:
///
/// ```text
/// app=org.nagi.example
/// grant=files.search
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppManifest {
    identifier: Name<MAX_APP_IDENTIFIER>,
    app_id: AppId,
    grants: [Name<MAX_GRANT_NAME>; MAX_APP_GRANTS],
    grant_count: usize,
}

impl AppManifest {
    pub fn parse(text: &[u8]) -> Result<Self, LaunchError> {
        if text.is_empty() || text.len() > MAX_APP_MANIFEST_BYTES {
            return Err(LaunchError::InvalidManifest);
        }
        let mut identifier = None;
        let mut grants = [Name::EMPTY; MAX_APP_GRANTS];
        let mut grant_count = 0;
        for line in text.split(|byte| *byte == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() || line.starts_with(b"#") {
                continue;
            }
            let Some(separator) = line.iter().position(|byte| *byte == b'=') else {
                return Err(LaunchError::InvalidManifest);
            };
            let (key, value) = (&line[..separator], &line[separator + 1..]);
            match key {
                b"app" if identifier.is_none() => identifier = Some(Name::parse(value)?),
                b"grant" => {
                    let grant = Name::parse(value)?;
                    if grants[..grant_count].contains(&grant) {
                        return Err(LaunchError::InvalidManifest);
                    }
                    if grant_count == MAX_APP_GRANTS {
                        return Err(LaunchError::InvalidManifest);
                    }
                    grants[grant_count] = grant;
                    grant_count += 1;
                }
                _ => return Err(LaunchError::InvalidManifest),
            }
        }
        let identifier = identifier.ok_or(LaunchError::InvalidManifest)?;
        Ok(Self {
            app_id: AppId::from_identifier(identifier.as_bytes()),
            identifier,
            grants,
            grant_count,
        })
    }

    /// Build a declaration from an already-verified source, such as a signed
    /// M16 package manifest (ADR 0049). The same naming rules apply as for
    /// text manifests.
    pub fn from_declaration<'a>(
        identifier: &[u8],
        grants: impl IntoIterator<Item = &'a [u8]>,
    ) -> Result<Self, LaunchError> {
        let identifier = Name::<MAX_APP_IDENTIFIER>::parse(identifier)?;
        let mut manifest = Self {
            app_id: AppId::from_identifier(identifier.as_bytes()),
            identifier,
            grants: [Name::EMPTY; MAX_APP_GRANTS],
            grant_count: 0,
        };
        for grant in grants {
            let grant = Name::parse(grant)?;
            if manifest.grants[..manifest.grant_count].contains(&grant)
                || manifest.grant_count == MAX_APP_GRANTS
            {
                return Err(LaunchError::InvalidManifest);
            }
            manifest.grants[manifest.grant_count] = grant;
            manifest.grant_count += 1;
        }
        Ok(manifest)
    }

    pub const fn app_id(&self) -> AppId {
        self.app_id
    }

    pub fn identifier(&self) -> &[u8] {
        self.identifier.as_bytes()
    }

    pub fn grants(&self, capability: &[u8]) -> bool {
        self.grants[..self.grant_count]
            .iter()
            .any(|grant| grant.as_bytes() == capability)
    }
}

/// Identity bound to a live process by the Supervisor at launch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaunchRecord {
    pub process_id: u32,
    pub app_id: AppId,
    pub app_session_id: AppSessionId,
    pub node_id: NodeId,
    pub workspace_id: Option<WorkspaceId>,
}

/// Where the Supervisor places a launch. The session is chosen by the
/// Supervisor: a new one, or a restored logical session, because workspaces
/// reference sessions across boots. It never comes from the launched process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaunchPlacement {
    pub app_session_id: AppSessionId,
    pub node_id: NodeId,
    pub workspace_id: Option<WorkspaceId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Consent {
    app_id: AppId,
    capability: Name<MAX_GRANT_NAME>,
    decision: GrantDecision,
}

pub struct LaunchRegistry {
    manifests: [Option<AppManifest>; MAX_APP_MANIFESTS],
    launches: [Option<LaunchRecord>; MAX_LIVE_LAUNCHES],
    consents: [Option<Consent>; MAX_CONSENT_DECISIONS],
}

impl Default for LaunchRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LaunchRegistry {
    pub const fn new() -> Self {
        Self {
            manifests: [None; MAX_APP_MANIFESTS],
            launches: [None; MAX_LIVE_LAUNCHES],
            consents: [None; MAX_CONSENT_DECISIONS],
        }
    }

    pub fn register_manifest(&mut self, manifest: AppManifest) -> Result<AppId, LaunchError> {
        if self
            .manifests
            .iter()
            .flatten()
            .any(|existing| existing.app_id == manifest.app_id)
        {
            return Err(LaunchError::DuplicateManifest);
        }
        let slot = self
            .manifests
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(LaunchError::ManifestTableFull)?;
        *slot = Some(manifest);
        Ok(manifest.app_id)
    }

    /// Register `manifest`, or accept it if an identical declaration for the
    /// same application is already registered. One application may ship
    /// several packages, but they must agree on its grants.
    pub fn register_or_match(&mut self, manifest: AppManifest) -> Result<AppId, LaunchError> {
        match self.manifest(manifest.app_id) {
            Some(existing) if *existing == manifest => Ok(manifest.app_id),
            Some(_) => Err(LaunchError::ConflictingManifest),
            None => self.register_manifest(manifest),
        }
    }

    fn manifest(&self, app_id: AppId) -> Option<&AppManifest> {
        self.manifests
            .iter()
            .flatten()
            .find(|manifest| manifest.app_id == app_id)
    }

    /// Check every launch precondition that does not depend on the Process
    /// ID. The Supervisor calls this before spawning, because a spawned
    /// process cannot be withdrawn if recording its launch then fails.
    pub fn check_launch(
        &self,
        app_id: AppId,
        placement: LaunchPlacement,
    ) -> Result<(), LaunchError> {
        if self.manifest(app_id).is_none() {
            return Err(LaunchError::UnknownApplication);
        }
        let mut live = self.launches.iter().flatten();
        if live.any(|record| {
            record.app_id == app_id && record.app_session_id == placement.app_session_id
        }) {
            return Err(LaunchError::SessionAlreadyLive);
        }
        if self.launches.iter().all(Option::is_some) {
            return Err(LaunchError::LaunchTableFull);
        }
        Ok(())
    }

    /// Bind a freshly spawned process to a declared application. PID 1
    /// (init) cannot be relabeled, and one application session is live in at
    /// most one process.
    pub fn record_launch(
        &mut self,
        process_id: u32,
        app_id: AppId,
        placement: LaunchPlacement,
    ) -> Result<LaunchRecord, LaunchError> {
        if process_id <= 1 {
            return Err(LaunchError::InitProcess);
        }
        if self
            .launches
            .iter()
            .flatten()
            .any(|record| record.process_id == process_id)
        {
            return Err(LaunchError::ProcessAlreadyLaunched);
        }
        self.check_launch(app_id, placement)?;
        let slot = self
            .launches
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(LaunchError::LaunchTableFull)?;
        let record = LaunchRecord {
            process_id,
            app_id,
            app_session_id: placement.app_session_id,
            node_id: placement.node_id,
            workspace_id: placement.workspace_id,
        };
        *slot = Some(record);
        Ok(record)
    }

    /// The identity of a kernel-stamped sender, if it is a live launch.
    pub fn resolve(&self, process_id: u32) -> Option<LaunchRecord> {
        self.launches
            .iter()
            .flatten()
            .copied()
            .find(|record| record.process_id == process_id)
    }

    fn is_live(&self, app_id: AppId, app_session_id: AppSessionId) -> bool {
        self.launches
            .iter()
            .flatten()
            .any(|record| record.app_id == app_id && record.app_session_id == app_session_id)
    }

    fn consent(&self, app_id: AppId, capability: &[u8]) -> Option<GrantDecision> {
        self.consents
            .iter()
            .flatten()
            .find(|consent| consent.app_id == app_id && consent.capability.as_bytes() == capability)
            .map(|consent| consent.decision)
    }

    /// Record `user`'s decision for `app_id`'s use of `capability`. Only an
    /// authenticated, unlocked session can decide; the registry never
    /// infers consent. `Ask` withdraws an earlier decision.
    pub fn record_decision(
        &mut self,
        user: &Session,
        app_id: AppId,
        capability: &[u8],
        decision: GrantDecision,
    ) -> Result<(), LaunchError> {
        if user.token() == 0 || user.is_locked() {
            return Err(LaunchError::ConsentUnavailable);
        }
        let capability = Name::<MAX_GRANT_NAME>::parse(capability)?;
        if let GrantDecision::AllowOnce(session) = decision {
            if !self.is_live(app_id, session) {
                return Err(LaunchError::SessionNotLive);
            }
        }
        let existing = self.consents.iter().position(|slot| {
            slot.is_some_and(|consent| consent.app_id == app_id && consent.capability == capability)
        });
        if decision == GrantDecision::Ask {
            if let Some(index) = existing {
                self.consents[index] = None;
            }
            return Ok(());
        }
        let index = existing
            .or_else(|| self.consents.iter().position(Option::is_none))
            .ok_or(LaunchError::ConsentTableFull)?;
        self.consents[index] = Some(Consent {
            app_id,
            capability,
            decision,
        });
        Ok(())
    }

    /// Whether the live application session `(app_id, app_session_id)` may
    /// exercise `capability`: the session is live, its manifest requests the
    /// capability, and the user allowed it. Identities that no live launch
    /// holds, including exited or forged sessions, have no grants.
    pub fn check_grant(
        &self,
        app_id: AppId,
        app_session_id: AppSessionId,
        capability: &[u8],
    ) -> GrantCheck {
        if !self.is_live(app_id, app_session_id) {
            return GrantCheck::NotLive;
        }
        if !self
            .manifest(app_id)
            .is_some_and(|manifest| manifest.grants(capability))
        {
            return GrantCheck::NotDeclared;
        }
        match self.consent(app_id, capability) {
            Some(GrantDecision::Allow) => GrantCheck::Granted,
            Some(GrantDecision::AllowOnce(session)) if session == app_session_id => {
                GrantCheck::Granted
            }
            Some(GrantDecision::Deny) => GrantCheck::Denied,
            _ => GrantCheck::ConsentRequired,
        }
    }

    pub fn has_grant(
        &self,
        app_id: AppId,
        app_session_id: AppSessionId,
        capability: &[u8],
    ) -> bool {
        self.check_grant(app_id, app_session_id, capability) == GrantCheck::Granted
    }

    /// Remove an exited process's launch, revoking its session's grants.
    pub fn record_exit(&mut self, process_id: u32) -> Option<LaunchRecord> {
        let slot = self
            .launches
            .iter_mut()
            .find(|slot| slot.is_some_and(|record| record.process_id == process_id))?;
        let record = slot.take()?;
        // An `AllowOnce` decision ends with the session it was given to.
        for slot in &mut self.consents {
            if slot.is_some_and(|consent| {
                consent.app_id == record.app_id
                    && consent.decision == GrantDecision::AllowOnce(record.app_session_id)
            }) {
                *slot = None;
            }
        }
        Some(record)
    }

    pub fn live_launches(&self) -> usize {
        self.launches.iter().flatten().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{AccountStore, Role};

    fn user() -> Session {
        let mut accounts = AccountStore::new();
        accounts
            .add_account(b"user", Role::Standard, b"user-pass")
            .expect("account");
        accounts.authenticate(b"user", b"user-pass").expect("login")
    }

    const PLACEMENT: LaunchPlacement = LaunchPlacement {
        app_session_id: AppSessionId(0x51),
        node_id: NodeId(0x52),
        workspace_id: Some(WorkspaceId(0x53)),
    };

    fn registry_with(text: &[u8]) -> (LaunchRegistry, AppId) {
        let mut registry = LaunchRegistry::new();
        let app = registry
            .register_manifest(AppManifest::parse(text).expect("manifest"))
            .expect("register");
        (registry, app)
    }

    #[test]
    fn manifest_identity_is_derived_from_its_identifier() {
        let manifest = AppManifest::parse(
            b"# search client\r\napp=org.nagi.example\ngrant=files.search\ngrant=files.move\n",
        )
        .expect("manifest");
        assert_eq!(
            manifest.app_id(),
            AppId::from_identifier(b"org.nagi.example")
        );
        assert_eq!(manifest.identifier(), b"org.nagi.example");
        assert!(manifest.grants(b"files.search"));
        assert!(manifest.grants(b"files.move"));
        assert!(!manifest.grants(b"files.copy"));
        assert!(!manifest.grants(b"files"));
    }

    #[test]
    fn malformed_manifests_are_rejected() {
        for text in [
            &b""[..],
            b"grant=files.search\n",
            b"app=org.nagi.a\napp=org.nagi.b\n",
            b"app=Org.Nagi\n",
            b"app=org.nagi/../x\n",
            b"app=org.nagi.a\ngrant=files.search\ngrant=files.search\n",
            b"app=org.nagi.a\nowner=root\n",
            b"app=org.nagi.a\ngrant\n",
            b"app=org.nagi.a\ngrant=\n",
        ] {
            assert_eq!(AppManifest::parse(text), Err(LaunchError::InvalidManifest));
        }
        let mut many = [0_u8; 512];
        let mut length = 0;
        for line in [
            &b"app=org.nagi.a\n"[..],
            b"grant=g.a\n",
            b"grant=g.b\n",
            b"grant=g.c\n",
            b"grant=g.d\n",
            b"grant=g.e\n",
            b"grant=g.f\n",
            b"grant=g.g\n",
            b"grant=g.h\n",
            b"grant=g.i\n",
        ] {
            many[length..length + line.len()].copy_from_slice(line);
            length += line.len();
        }
        assert_eq!(
            AppManifest::parse(&many[..length]),
            Err(LaunchError::InvalidManifest)
        );
    }

    #[test]
    fn only_declared_apps_launch_and_grants_follow_live_sessions() {
        let (mut registry, app) = registry_with(b"app=org.nagi.example\ngrant=files.search\n");
        let unknown = AppId::from_identifier(b"org.nagi.unknown");
        assert_eq!(
            registry.record_launch(2, unknown, PLACEMENT),
            Err(LaunchError::UnknownApplication)
        );
        assert_eq!(
            registry.record_launch(1, app, PLACEMENT),
            Err(LaunchError::InitProcess)
        );
        // No live session: no grant, even for the declared app.
        assert!(!registry.has_grant(app, PLACEMENT.app_session_id, b"files.search"));
        registry
            .record_decision(&user(), app, b"files.search", GrantDecision::Allow)
            .expect("consent");
        assert_eq!(
            registry.check_grant(app, PLACEMENT.app_session_id, b"files.search"),
            GrantCheck::NotLive
        );

        let record = registry.record_launch(2, app, PLACEMENT).expect("launch");
        assert_eq!(registry.resolve(2), Some(record));
        assert_eq!(registry.resolve(3), None);
        assert!(registry.has_grant(app, PLACEMENT.app_session_id, b"files.search"));
        assert!(!registry.has_grant(app, PLACEMENT.app_session_id, b"files.move"));
        assert!(!registry.has_grant(app, AppSessionId(0x99), b"files.search"));
        assert!(!registry.has_grant(unknown, PLACEMENT.app_session_id, b"files.search"));

        assert_eq!(
            registry.record_launch(
                2,
                app,
                LaunchPlacement {
                    app_session_id: AppSessionId(0x60),
                    ..PLACEMENT
                }
            ),
            Err(LaunchError::ProcessAlreadyLaunched)
        );
        assert_eq!(
            registry.record_launch(3, app, PLACEMENT),
            Err(LaunchError::SessionAlreadyLive)
        );
        assert_eq!(
            registry.check_launch(app, PLACEMENT),
            Err(LaunchError::SessionAlreadyLive)
        );
        assert_eq!(
            registry.check_launch(unknown, PLACEMENT),
            Err(LaunchError::UnknownApplication)
        );

        assert_eq!(registry.record_exit(2), Some(record));
        assert_eq!(registry.resolve(2), None);
        assert!(!registry.has_grant(app, PLACEMENT.app_session_id, b"files.search"));
        assert_eq!(registry.record_exit(2), None);
        // The session can be restored by a later launch.
        assert!(registry.record_launch(2, app, PLACEMENT).is_ok());
    }

    #[test]
    fn manifest_and_launch_tables_are_bounded() {
        let mut registry = LaunchRegistry::new();
        let manifest = AppManifest::parse(b"app=org.nagi.example\n").unwrap();
        registry.register_manifest(manifest).unwrap();
        assert_eq!(
            registry.register_manifest(manifest),
            Err(LaunchError::DuplicateManifest)
        );
        let names: [&[u8]; MAX_APP_MANIFESTS - 1] = [
            b"app=a.b\n",
            b"app=a.c\n",
            b"app=a.d\n",
            b"app=a.e\n",
            b"app=a.f\n",
            b"app=a.g\n",
            b"app=a.h\n",
        ];
        for text in names {
            registry
                .register_manifest(AppManifest::parse(text).unwrap())
                .unwrap();
        }
        assert_eq!(
            registry.register_manifest(AppManifest::parse(b"app=a.z\n").unwrap()),
            Err(LaunchError::ManifestTableFull)
        );
        let app = manifest.app_id();
        for index in 0..MAX_LIVE_LAUNCHES {
            registry
                .record_launch(
                    2 + index as u32,
                    app,
                    LaunchPlacement {
                        app_session_id: AppSessionId(index as u64),
                        ..PLACEMENT
                    },
                )
                .unwrap();
        }
        assert_eq!(registry.live_launches(), MAX_LIVE_LAUNCHES);
        assert_eq!(
            registry.check_launch(
                app,
                LaunchPlacement {
                    app_session_id: AppSessionId(0x77),
                    ..PLACEMENT
                }
            ),
            Err(LaunchError::LaunchTableFull)
        );
        assert_eq!(
            registry.record_launch(99, app, PLACEMENT),
            Err(LaunchError::LaunchTableFull)
        );
    }

    #[test]
    fn package_declarations_match_text_manifests_and_must_agree() {
        let declared = AppManifest::from_declaration(
            b"org.nagi.example",
            [&b"files.search"[..], &b"search.query"[..]],
        )
        .expect("declaration");
        let text =
            AppManifest::parse(b"app=org.nagi.example\ngrant=files.search\ngrant=search.query\n")
                .expect("manifest");
        assert_eq!(declared, text);
        assert_eq!(
            AppManifest::from_declaration(b"Org", core::iter::empty()),
            Err(LaunchError::InvalidManifest)
        );
        assert_eq!(
            AppManifest::from_declaration(b"org.a", [&b"x.y"[..], &b"x.y"[..]]),
            Err(LaunchError::InvalidManifest)
        );

        let mut registry = LaunchRegistry::new();
        let app = registry.register_or_match(declared).expect("first package");
        assert_eq!(registry.register_or_match(declared), Ok(app));
        let widened = AppManifest::from_declaration(
            b"org.nagi.example",
            [&b"files.search"[..], &b"files.delete"[..]],
        )
        .expect("declaration");
        assert_eq!(
            registry.register_or_match(widened),
            Err(LaunchError::ConflictingManifest)
        );
    }

    #[test]
    fn manifest_grants_need_user_consent() {
        let (mut registry, app) =
            registry_with(b"app=org.nagi.example\ngrant=files.search\ngrant=files.move\n");
        let other = LaunchPlacement {
            app_session_id: AppSessionId(0x61),
            ..PLACEMENT
        };
        registry.record_launch(2, app, PLACEMENT).expect("launch");
        registry
            .record_launch(3, app, other)
            .expect("second launch");
        let session = PLACEMENT.app_session_id;
        // Requested but undecided: fail closed.
        assert_eq!(
            registry.check_grant(app, session, b"files.search"),
            GrantCheck::ConsentRequired
        );
        assert_eq!(
            registry.check_grant(app, session, b"files.copy"),
            GrantCheck::NotDeclared
        );

        // Only an authenticated, unlocked user decides.
        let mut locked = user();
        locked.lock();
        assert_eq!(
            registry.record_decision(&locked, app, b"files.search", GrantDecision::Allow),
            Err(LaunchError::ConsentUnavailable)
        );
        assert_eq!(
            registry.record_decision(&user(), app, b"Files", GrantDecision::Allow),
            Err(LaunchError::InvalidManifest)
        );
        // Consenting to an undeclared capability does not create it.
        registry
            .record_decision(&user(), app, b"files.copy", GrantDecision::Allow)
            .expect("decision");
        assert_eq!(
            registry.check_grant(app, session, b"files.copy"),
            GrantCheck::NotDeclared
        );

        // Allow once covers exactly one live session and ends with it.
        assert_eq!(
            registry.record_decision(
                &user(),
                app,
                b"files.search",
                GrantDecision::AllowOnce(AppSessionId(0x99))
            ),
            Err(LaunchError::SessionNotLive)
        );
        registry
            .record_decision(
                &user(),
                app,
                b"files.search",
                GrantDecision::AllowOnce(session),
            )
            .expect("allow once");
        assert!(registry.has_grant(app, session, b"files.search"));
        assert_eq!(
            registry.check_grant(app, other.app_session_id, b"files.search"),
            GrantCheck::ConsentRequired
        );
        assert!(!registry.has_grant(app, session, b"files.move"));
        registry.record_exit(2).expect("exit");
        registry.record_launch(4, app, PLACEMENT).expect("relaunch");
        assert_eq!(
            registry.check_grant(app, session, b"files.search"),
            GrantCheck::ConsentRequired
        );

        // Deny overrides the manifest; Allow covers every session; Ask
        // withdraws the decision.
        registry
            .record_decision(&user(), app, b"files.search", GrantDecision::Deny)
            .expect("deny");
        assert_eq!(
            registry.check_grant(app, session, b"files.search"),
            GrantCheck::Denied
        );
        registry
            .record_decision(&user(), app, b"files.search", GrantDecision::Allow)
            .expect("allow");
        assert!(registry.has_grant(app, session, b"files.search"));
        assert!(registry.has_grant(app, other.app_session_id, b"files.search"));
        registry
            .record_decision(&user(), app, b"files.search", GrantDecision::Ask)
            .expect("ask");
        assert_eq!(
            registry.check_grant(app, other.app_session_id, b"files.search"),
            GrantCheck::ConsentRequired
        );
    }

    #[test]
    fn developer_mode_and_owner_do_not_imply_consent() {
        let (mut registry, app) = registry_with(b"app=org.nagi.example\ngrant=files.search\n");
        let mut accounts = AccountStore::new();
        accounts
            .add_account(b"owner", Role::Owner, b"owner-pass")
            .expect("owner");
        let mut owner = accounts
            .authenticate(b"owner", b"owner-pass")
            .expect("login");
        assert!(accounts.enable_developer_mode(&mut owner));
        registry.record_launch(2, app, PLACEMENT).expect("launch");
        assert_eq!(
            registry.check_grant(app, PLACEMENT.app_session_id, b"files.search"),
            GrantCheck::ConsentRequired
        );
    }

    #[test]
    fn consent_table_is_bounded() {
        let (mut registry, app) = registry_with(b"app=org.nagi.example\n");
        let names = b"abcdefghijklmnopq";
        for name in &names[..MAX_CONSENT_DECISIONS] {
            let capability = [b'g', b'.', *name];
            registry
                .record_decision(&user(), app, &capability, GrantDecision::Deny)
                .expect("decision");
        }
        // Replacing an existing decision needs no new slot.
        registry
            .record_decision(&user(), app, b"g.a", GrantDecision::Allow)
            .expect("replace");
        assert_eq!(
            registry.record_decision(&user(), app, b"g.z", GrantDecision::Allow),
            Err(LaunchError::ConsentTableFull)
        );
        registry
            .record_decision(&user(), app, b"g.b", GrantDecision::Ask)
            .expect("withdraw");
        assert!(registry
            .record_decision(&user(), app, b"g.z", GrantDecision::Allow)
            .is_ok());
    }
}
