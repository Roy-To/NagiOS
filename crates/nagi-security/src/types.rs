use nagi_model::{AppId, ObjectId, UserId};

pub const MAX_CAPABILITY_ID_BYTES: usize = 63;
/// Bounded 0.1 scope value; longer DNS names fail closed during parsing.
pub const MAX_DOMAIN_NAME_BYTES: usize = 127;
pub const MAX_LOCALIZATION_KEY_BYTES: usize = 95;
pub const MAX_DEVICE_CLASS_BYTES: usize = 31;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentifierError {
    Empty,
    TooLong,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BoundedName<const N: usize> {
    bytes: [u8; N],
    length: u8,
}

impl<const N: usize> BoundedName<N> {
    const EMPTY: Self = Self {
        bytes: [0; N],
        length: 0,
    };

    fn parse(value: &[u8], validator: fn(u8) -> bool) -> Result<Self, IdentifierError> {
        if value.is_empty() {
            return Err(IdentifierError::Empty);
        }
        if value.len() > N {
            return Err(IdentifierError::TooLong);
        }
        if !value.iter().copied().all(validator) {
            return Err(IdentifierError::Invalid);
        }
        let mut name = Self::EMPTY;
        name.bytes[..value.len()].copy_from_slice(value);
        name.length = value.len() as u8;
        Ok(name)
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.length)]
    }
}

fn capability_byte_is_valid(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-')
}

fn localization_byte_is_valid(byte: u8) -> bool {
    capability_byte_is_valid(byte)
}

fn label_byte_is_valid(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_')
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityId(BoundedName<MAX_CAPABILITY_ID_BYTES>);

impl CapabilityId {
    pub fn parse(value: &[u8]) -> Result<Self, IdentifierError> {
        let name = BoundedName::parse(value, capability_byte_is_valid)?;
        if value.first() == Some(&b'.')
            || value.last() == Some(&b'.')
            || value.windows(2).any(|pair| pair == b"..")
            || !value.contains(&b'.')
        {
            return Err(IdentifierError::Invalid);
        }
        Ok(Self(name))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Classify a registered capability as resource permission, platform
    /// participation, or system authority. This classification grants no
    /// access by itself.
    pub fn kind(self) -> Option<CapabilityKind> {
        self.risk_class()?;
        match self.as_bytes() {
            b"context.publish"
            | b"context.read"
            | b"actions.register"
            | b"search.provider"
            | b"workspace.participate"
            | b"ai.model.invoke"
            | b"ai.action.execute"
            | b"activity.read"
            | b"wayback.checkpoint"
            | b"wayback.restore" => Some(CapabilityKind::Platform),
            b"process.execute"
            | b"device.access"
            | b"system.settings.write"
            | b"accessibility.control"
            | b"background.execute"
            | b"system.packages.install"
            | b"system.credentials.access"
            | b"system.devices.manage"
            | b"system.administration" => Some(CapabilityKind::System),
            _ => Some(CapabilityKind::ResourcePermission),
        }
    }

    /// Return the built-in Nagi 0.1 risk class. Unregistered identifiers are
    /// rejected by policy evaluation even if a stale policy row exists.
    pub fn risk_class(self) -> Option<RiskClass> {
        match self.as_bytes() {
            b"files.read"
            | b"files.directory.read"
            | b"network.access"
            | b"network.connect"
            | b"audio.output"
            | b"notifications.send"
            | b"ai.model.invoke"
            | b"activity.read"
            | b"wayback.checkpoint"
            | b"context.publish"
            | b"context.read"
            | b"actions.register"
            | b"search.provider"
            | b"workspace.participate" => Some(RiskClass::Routine),
            b"files.write"
            | b"files.directory.write"
            | b"network.write"
            | b"network.listen"
            | b"microphone.capture"
            | b"camera.capture"
            | b"screen.capture"
            | b"clipboard.read"
            | b"clipboard.write"
            | b"location.read"
            | b"contacts.read"
            | b"contacts.write"
            | b"calendar.read"
            | b"calendar.write"
            | b"mail.read"
            | b"device.access" => Some(RiskClass::Sensitive),
            b"files.delete" | b"mail.send" | b"calendar.invite" | b"wayback.restore" => {
                Some(RiskClass::Destructive)
            }
            b"process.execute"
            | b"system.settings.write"
            | b"accessibility.control"
            | b"background.execute"
            | b"ai.action.execute"
            | b"system.packages.install"
            | b"system.credentials.access"
            | b"system.devices.manage"
            | b"system.administration" => Some(RiskClass::Privileged),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainName(BoundedName<MAX_DOMAIN_NAME_BYTES>);

impl DomainName {
    pub fn parse(value: &[u8]) -> Result<Self, IdentifierError> {
        let name = BoundedName::parse(
            value,
            |byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.'),
        )?;
        let mut label_length = 0_usize;
        let mut label_first = 0_u8;
        let mut previous = 0_u8;
        for byte in value.iter().copied().chain(core::iter::once(b'.')) {
            if byte == b'.' {
                if label_length == 0 || label_length > 63 || label_first == b'-' || previous == b'-'
                {
                    return Err(IdentifierError::Invalid);
                }
                label_length = 0;
                label_first = 0;
            } else {
                if label_length == 0 {
                    label_first = byte;
                }
                label_length += 1;
                previous = byte;
            }
        }
        Ok(Self(name))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn is_within(self, grant: Self) -> bool {
        let requested = self.as_bytes();
        let granted = grant.as_bytes();
        requested == granted
            || (requested.len() > granted.len()
                && requested[requested.len() - granted.len() - 1] == b'.'
                && requested.ends_with(granted))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalizationKey(BoundedName<MAX_LOCALIZATION_KEY_BYTES>);

impl LocalizationKey {
    pub fn parse(value: &[u8]) -> Result<Self, IdentifierError> {
        let name = BoundedName::parse(value, localization_byte_is_valid)?;
        if value.first() == Some(&b'.')
            || value.last() == Some(&b'.')
            || value.windows(2).any(|pair| pair == b"..")
            || !value.contains(&b'.')
        {
            return Err(IdentifierError::Invalid);
        }
        Ok(Self(name))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceClassId(BoundedName<MAX_DEVICE_CLASS_BYTES>);

impl DeviceClassId {
    pub fn parse(value: &[u8]) -> Result<Self, IdentifierError> {
        let name = BoundedName::parse(value, label_byte_is_valid)?;
        if value.first() == Some(&b'-')
            || value.last() == Some(&b'-')
            || value.first() == Some(&b'_')
            || value.last() == Some(&b'_')
        {
            return Err(IdentifierError::Invalid);
        }
        Ok(Self(name))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityScope {
    Any,
    Object(ObjectId),
    Directory(ObjectId),
    /// A resolved object and the directory tree that a trusted resource
    /// service attested contains it. Callers must not derive this relationship
    /// from caller-supplied paths or IDs.
    ObjectWithinDirectory {
        root_directory: ObjectId,
        object: ObjectId,
    },
    Domain(DomainName),
    Localhost,
    DeviceClass(DeviceClassId),
    Device {
        class: DeviceClassId,
        id: u64,
    },
}

impl CapabilityScope {
    /// Whether this requested scope is contained by `grant`.
    pub fn is_within(self, grant: Self) -> bool {
        match (self, grant) {
            (_, Self::Any) => true,
            (Self::Any, _) => false,
            (Self::Object(requested), Self::Object(granted))
            | (Self::Directory(requested), Self::Directory(granted)) => requested == granted,
            (
                Self::ObjectWithinDirectory {
                    root_directory: requested_root,
                    object: _,
                },
                Self::Directory(granted_root),
            ) => requested_root == granted_root,
            (
                Self::ObjectWithinDirectory {
                    object: requested_object,
                    ..
                },
                Self::Object(granted_object),
            ) => requested_object == granted_object,
            (
                Self::ObjectWithinDirectory {
                    root_directory: requested_root,
                    object: requested_object,
                },
                Self::ObjectWithinDirectory {
                    root_directory: granted_root,
                    object: granted_object,
                },
            ) => requested_root == granted_root && requested_object == granted_object,
            (Self::Domain(requested), Self::Domain(granted)) => requested.is_within(granted),
            (Self::Localhost, Self::Localhost) => true,
            (Self::DeviceClass(requested), Self::DeviceClass(granted)) => requested == granted,
            (Self::Device { class, .. }, Self::DeviceClass(granted)) => class == granted,
            (
                Self::Device {
                    class: requested_class,
                    id: requested_id,
                },
                Self::Device {
                    class: granted_class,
                    id: granted_id,
                },
            ) => requested_class == granted_class && requested_id == granted_id,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityScopeTemplate {
    Any,
    SelectedObject,
    SelectedDirectory,
    Domain(DomainName),
    Localhost,
    DeviceClass(DeviceClassId),
}

impl CapabilityScopeTemplate {
    pub fn parse(value: &[u8]) -> Result<Self, IdentifierError> {
        match value {
            b"any" => Ok(Self::Any),
            b"selected-object" => Ok(Self::SelectedObject),
            b"selected-directory" => Ok(Self::SelectedDirectory),
            b"localhost" => Ok(Self::Localhost),
            _ if value.starts_with(b"domain:") => {
                DomainName::parse(&value[b"domain:".len()..]).map(Self::Domain)
            }
            _ if value.starts_with(b"device-class:") => {
                DeviceClassId::parse(&value[b"device-class:".len()..]).map(Self::DeviceClass)
            }
            _ => Err(IdentifierError::Invalid),
        }
    }

    pub fn permits(self, requested: CapabilityScope) -> bool {
        match self {
            Self::Any => true,
            Self::SelectedObject => matches!(
                requested,
                CapabilityScope::Object(_) | CapabilityScope::ObjectWithinDirectory { .. }
            ),
            Self::SelectedDirectory => matches!(
                requested,
                CapabilityScope::Directory(_) | CapabilityScope::ObjectWithinDirectory { .. }
            ),
            Self::Domain(domain) => match requested {
                CapabilityScope::Domain(requested_domain) => requested_domain.is_within(domain),
                _ => false,
            },
            Self::Localhost => matches!(requested, CapabilityScope::Localhost),
            Self::DeviceClass(class) => match requested {
                CapabilityScope::DeviceClass(requested_class) => requested_class == class,
                CapabilityScope::Device {
                    class: requested_class,
                    ..
                } => requested_class == class,
                _ => false,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityRequirement {
    Required,
    Optional,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityDeclaration {
    pub capability: CapabilityId,
    pub requirement: CapabilityRequirement,
    pub scope: CapabilityScopeTemplate,
    pub reason: LocalizationKey,
}

impl CapabilityDeclaration {
    /// Parse `capability|required|scope-template|localization.reason`.
    pub fn parse(value: &[u8]) -> Result<Self, IdentifierError> {
        let mut fields = value.split(|byte| *byte == b'|');
        let capability = CapabilityId::parse(fields.next().ok_or(IdentifierError::Invalid)?)?;
        let requirement = match fields.next() {
            Some(b"required") => CapabilityRequirement::Required,
            Some(b"optional") => CapabilityRequirement::Optional,
            _ => return Err(IdentifierError::Invalid),
        };
        let scope = CapabilityScopeTemplate::parse(fields.next().ok_or(IdentifierError::Invalid)?)?;
        let reason = LocalizationKey::parse(fields.next().ok_or(IdentifierError::Invalid)?)?;
        if fields.next().is_some() {
            return Err(IdentifierError::Invalid);
        }
        Ok(Self {
            capability,
            requirement,
            scope,
            reason,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Actor {
    User(UserId),
    SystemService(SystemServiceId),
    FirstPartyApp(AppId),
    ThirdPartyApp(AppId),
    AiAgent(AppId),
    BackgroundAutomation(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemServiceId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DelegationId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionCorrelationId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RiskClass {
    Routine,
    Sensitive,
    Destructive,
    Privileged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityKind {
    ResourcePermission,
    Platform,
    System,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationKind {
    UserDirect,
    Application,
    AiSuggestion,
    AiDelegated,
    Automation,
    SystemService,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvaluationContext {
    pub invocation: InvocationKind,
    pub foreground: bool,
    pub user_initiated: bool,
    pub now: u64,
    pub delegation_id: Option<DelegationId>,
    pub correlation_id: ActionCorrelationId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyRequest {
    pub actor: Actor,
    pub capability: CapabilityId,
    pub scope: CapabilityScope,
    pub context: EvaluationContext,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionGrant {
    pub actor: Actor,
    pub capability: CapabilityId,
    pub scope: CapabilityScope,
    pub decision: PermissionDecision,
    pub allow_background: bool,
}

impl PermissionGrant {
    pub const fn key(self) -> crate::PermissionKey {
        crate::PermissionKey {
            actor: self.actor,
            capability: self.capability,
            scope: self.scope,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DelegationGrant {
    pub id: DelegationId,
    pub user: UserId,
    pub agent: Actor,
    pub capability: CapabilityId,
    pub scope: CapabilityScope,
    pub expires_at: u64,
    pub allow_background: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionReason {
    AllowedByPolicy,
    ExplicitlyDenied,
    UserConfirmationRequired,
    NoMatchingPolicy,
    ScopeNotGranted,
    BackgroundNotAllowed,
    UnknownCapability,
    SuggestionCannotExecute,
    DelegationRequired,
    DelegationNotFound,
    DelegationMismatch,
    DelegationExpired,
    DelegationDoesNotAllowBackground,
    UserAuthorityNotGranted,
    HighRiskConfirmationRequired,
    ActorContextMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Evaluation {
    pub decision: PermissionDecision,
    pub reason: DecisionReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuditEvent {
    pub actor: Actor,
    pub capability: CapabilityId,
    pub requested_scope: CapabilityScope,
    pub decision: PermissionDecision,
    pub reason: DecisionReason,
    pub timestamp: u64,
    pub correlation_id: ActionCorrelationId,
}

impl AuditEvent {
    pub const fn from_evaluation(request: PolicyRequest, evaluation: Evaluation) -> Self {
        Self {
            actor: request.actor,
            capability: request.capability,
            requested_scope: request.scope,
            decision: evaluation.decision,
            reason: evaluation.reason,
            timestamp: request.context.now,
            correlation_id: request.context.correlation_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CapabilityId, CapabilityKind, CapabilityScope, CapabilityScopeTemplate, DomainName,
        IdentifierError,
    };
    use nagi_model::ObjectId;

    #[test]
    fn domain_scopes_match_only_exact_domains_or_label_boundaries() {
        let grant = DomainName::parse(b"example.com").expect("domain");
        let child = DomainName::parse(b"api.example.com").expect("child domain");
        let suffix_lookalike = DomainName::parse(b"notexample.com").expect("other domain");
        assert!(child.is_within(grant));
        assert!(!suffix_lookalike.is_within(grant));

        let broad_request = CapabilityScope::Any;
        let narrow_grant = CapabilityScope::Domain(grant);
        assert!(!broad_request.is_within(narrow_grant));
        assert!(CapabilityScope::Object(ObjectId(1)).is_within(CapabilityScope::Any));
        assert!(CapabilityScope::Localhost.is_within(CapabilityScope::Any));
        assert!(!CapabilityScope::Domain(grant).is_within(CapabilityScope::Localhost));
        assert!(CapabilityScopeTemplate::parse(b"localhost")
            .expect("localhost template")
            .permits(CapabilityScope::Localhost));
        assert!(CapabilityScopeTemplate::SelectedDirectory.permits(
            CapabilityScope::ObjectWithinDirectory {
                root_directory: ObjectId(5),
                object: ObjectId(6),
            }
        ));
    }

    #[test]
    fn domain_names_are_bounded_and_canonical_lowercase_ascii() {
        assert_eq!(
            DomainName::parse(b"Example.com"),
            Err(IdentifierError::Invalid)
        );
        let oversized = [b'a'; super::MAX_DOMAIN_NAME_BYTES + 1];
        assert_eq!(DomainName::parse(&oversized), Err(IdentifierError::TooLong));
    }

    #[test]
    fn capability_kind_separates_resource_platform_and_system_authority() {
        assert_eq!(
            CapabilityId::parse(b"files.read")
                .expect("resource capability")
                .kind(),
            Some(CapabilityKind::ResourcePermission)
        );
        assert_eq!(
            CapabilityId::parse(b"actions.register")
                .expect("platform capability")
                .kind(),
            Some(CapabilityKind::Platform)
        );
        assert_eq!(
            CapabilityId::parse(b"system.administration")
                .expect("system capability")
                .kind(),
            Some(CapabilityKind::System)
        );
        assert_eq!(
            CapabilityId::parse(b"vendor.unknown")
                .expect("unknown capability")
                .kind(),
            None
        );
    }
}
