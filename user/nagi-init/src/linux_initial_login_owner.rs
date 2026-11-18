/// Start the login child and accept its first login against the recovered C1
/// runtime while PID 1 retains the system-service launch.
pub(super) trait LinuxInitialLoginOwner<IdentityRuntime> {
    type Attached;
    type Error;

    fn start_and_accept(&mut self, identity: &IdentityRuntime) -> Result<Self::Attached, Self::Error>;
}
