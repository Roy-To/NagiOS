#![no_std]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppId(pub u64);

impl AppId {
    /// Derive the stable logical identity used by packages and SDK clients.
    /// The identifier is a model-level naming rule, not an authority grant.
    pub const fn from_identifier(identifier: &[u8]) -> Self {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut index = 0;
        while index < identifier.len() {
            hash ^= identifier[index] as u64;
            hash = hash.wrapping_mul(0x1000_0000_01b3);
            index += 1;
        }
        Self(hash)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppSessionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionInstanceId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresentationClass {
    Compact,
    Medium,
    Expanded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationContext {
    pub logical_width: u16,
    pub logical_height: u16,
    pub dpi: u16,
    pub touch: bool,
    pub keyboard: bool,
    pub pointer: bool,
    pub class: PresentationClass,
}

impl PresentationContext {
    pub const fn compact(width: u16, height: u16) -> Self {
        Self {
            logical_width: width,
            logical_height: height,
            dpi: 96,
            touch: false,
            keyboard: true,
            pointer: true,
            class: PresentationClass::Compact,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationSurface {
    pub surface_id: SurfaceId,
    pub node_id: NodeId,
    pub context: PresentationContext,
}

#[cfg(test)]
mod tests {
    use super::{AppId, AppSessionId, PresentationClass, PresentationContext};

    #[test]
    fn presentation_is_separate_from_logical_application_identity() {
        let context = PresentationContext::compact(320, 200);
        assert_eq!(context.class, PresentationClass::Compact);
        assert_eq!(AppId(7), AppId(7));
        assert_eq!(AppSessionId(9), AppSessionId(9));
    }

    #[test]
    fn identifier_identity_is_stable() {
        assert_eq!(
            AppId::from_identifier(b"com.example.hello-nagi"),
            AppId::from_identifier(b"com.example.hello-nagi")
        );
        assert_ne!(
            AppId::from_identifier(b"com.example.hello-nagi"),
            AppId::from_identifier(b"com.example.other")
        );
    }
}
