#![no_std]

mod generated;

pub mod app_contract;

pub use generated::*;

pub use nagi_model::{
    AppId, AppSessionId, ExecutionInstanceId, NodeId, ObjectId, PresentationClass,
    PresentationContext, PresentationSurface, SurfaceId, TransactionId, UserId, WorkspaceId,
};

pub const HELLO_APP_ID: AppId = AppId::from_identifier(b"com.example.hello-nagi");
pub const HELLO_MANIFEST: &[u8] =
    b"id=com.example.hello-nagi\nname=Hello Nagi\nversion=0.1.0\nentry=hello.napp\nsurfaces=compact,expanded\n";

/// The M16 sample executable is a bounded NAPP user-space payload. It is
/// interpreted by the Package Service, never by the kernel or the host OS.
pub const HELLO_EXECUTABLE: &[u8] = b"NAPP\x01\x01\x21\x00Hello from out-of-tree Nagi app\r\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Application {
    pub app_id: AppId,
    pub session_id: AppSessionId,
}

impl Application {
    pub const fn new(app_id: AppId, session_id: AppSessionId) -> Self {
        Self { app_id, session_id }
    }

    pub const fn presentation(&self, context: PresentationContext) -> PresentationSurface {
        PresentationSurface {
            surface_id: SurfaceId(self.session_id.0),
            node_id: NodeId(1),
            context,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppSessionId, Application, PresentationClass, PresentationContext, HELLO_APP_ID};

    #[test]
    fn keeps_app_session_and_surface_identity_distinct() {
        let app = Application::new(HELLO_APP_ID, AppSessionId(7));
        let surface = app.presentation(PresentationContext::compact(320, 200));
        assert_eq!(surface.context.class, PresentationClass::Compact);
        assert_eq!(surface.surface_id.0, app.session_id.0);
        assert_eq!(surface.node_id.0, 1);
    }
}
