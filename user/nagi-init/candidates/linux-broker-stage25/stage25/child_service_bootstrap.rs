//! Child-side AF_UNIX endpoint locator passed through the existing argv path.
//!
//! The socket address is a transport locator only. It carries no PID, session,
//! route, grant, consent, or endpoint generation and must never be accepted as
//! authority. PID 1 authenticates the accepted stream against its spawn-owned
//! process-instance proof and rechecks all live owners before an effect.

use std::ffi::OsString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

const ARG_PREFIX: &str = "--nagi-service-endpoint-v1=";
const SOCKET_ROOT: &[u8] = b"/run/nagi/ipc/";
const LINUX_SUN_PATH_MAX: usize = 108;
const MAX_SOCKET_LEAF: usize = 80;

/// Parsed, bounded child bootstrap input. This value is only a locator; a
/// child cannot authenticate itself by supplying or echoing it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildServiceBootstrap {
    argument: String,
    socket_path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointBootstrapError {
    MissingEndpoint,
    UnsupportedVersion,
    DuplicateEndpoint,
    InvalidPath,
    NonPathListener,
    ListenerAddress,
}

impl ChildServiceBootstrap {
    /// Read only the listener address from the same PID-1-owned UnixListener
    /// that is retained in `PendingSpawnOwnedEndpoint`. Arbitrary caller text
    /// is not an input to this constructor.
    pub(crate) fn from_listener(listener: &UnixListener) -> Result<Self, EndpointBootstrapError> {
        let address = listener
            .local_addr()
            .map_err(|_| EndpointBootstrapError::ListenerAddress)?;
        let path = address
            .as_pathname()
            .ok_or(EndpointBootstrapError::NonPathListener)?;
        Self::from_path(path)
    }

    fn from_path(path: &Path) -> Result<Self, EndpointBootstrapError> {
        let bytes = path.as_os_str().as_bytes();
        validate_socket_path(bytes)?;
        let path_text =
            std::str::from_utf8(bytes).map_err(|_| EndpointBootstrapError::InvalidPath)?;
        Ok(Self {
            argument: format!("{ARG_PREFIX}{path_text}"),
            socket_path: PathBuf::from(path),
        })
    }

    /// Parse the one supported argv item. Duplicate, malformed, and future
    /// endpoint flags fail closed instead of silently selecting another path.
    pub fn from_args(args: &[OsString]) -> Result<Option<Self>, EndpointBootstrapError> {
        let mut selected: Option<Self> = None;
        for argument in args {
            let bytes = argument.as_os_str().as_bytes();
            if bytes.starts_with(b"--nagi-service-endpoint")
                && !bytes.starts_with(ARG_PREFIX.as_bytes())
            {
                return Err(EndpointBootstrapError::UnsupportedVersion);
            }
            let Some(path) = bytes.strip_prefix(ARG_PREFIX.as_bytes()) else {
                continue;
            };
            validate_socket_path(path)?;
            if selected.is_some() {
                return Err(EndpointBootstrapError::DuplicateEndpoint);
            }
            let path_text =
                std::str::from_utf8(path).map_err(|_| EndpointBootstrapError::InvalidPath)?;
            selected = Some(Self {
                argument: argument.to_string_lossy().into_owned(),
                socket_path: PathBuf::from(path_text),
            });
        }
        Ok(selected)
    }

    /// Require the bootstrap argument for a service process. Non-service apps
    /// can use `from_args()` and remain disconnected when it returns `None`.
    pub fn required_from_args(args: &[OsString]) -> Result<Self, EndpointBootstrapError> {
        Self::from_args(args)?.ok_or(EndpointBootstrapError::MissingEndpoint)
    }

    /// Connect to PID 1's advertised listener. The resulting stream is not an
    /// authentication proof; the parent validates kernel peer-instance data.
    pub fn connect(&self) -> io::Result<UnixStream> {
        UnixStream::connect(&self.socket_path)
    }

    pub(crate) fn argument(&self) -> &str {
        &self.argument
    }

    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

fn validate_socket_path(path: &[u8]) -> Result<(), EndpointBootstrapError> {
    if path.is_empty()
        || path.len() >= LINUX_SUN_PATH_MAX
        || !path.starts_with(SOCKET_ROOT)
        || path.contains(&0)
    {
        return Err(EndpointBootstrapError::InvalidPath);
    }
    let leaf = &path[SOCKET_ROOT.len()..];
    if leaf.len() < 6
        || leaf.len() > MAX_SOCKET_LEAF
        || !leaf.ends_with(b".sock")
        || leaf == b".sock"
        || leaf.windows(2).any(|pair| pair == b"..")
        || !leaf
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(EndpointBootstrapError::InvalidPath);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arg(path: &str) -> OsString {
        OsString::from(format!("{ARG_PREFIX}{path}"))
    }

    #[test]
    fn canonical_listener_locator_round_trips_without_authority_fields() {
        let parsed = ChildServiceBootstrap::from_args(&[
            OsString::from("--mode=service"),
            arg("/run/nagi/ipc/7f2a3c4d.sock"),
        ])
        .expect("valid bootstrap")
        .expect("service endpoint");
        assert_eq!(
            parsed.argument(),
            "--nagi-service-endpoint-v1=/run/nagi/ipc/7f2a3c4d.sock"
        );
        assert_eq!(
            parsed.socket_path(),
            Path::new("/run/nagi/ipc/7f2a3c4d.sock")
        );
        assert!(!parsed.argument().contains("pid="));
        assert!(!parsed.argument().contains("session="));
        assert!(!parsed.argument().contains("grant="));
    }

    #[test]
    fn malformed_duplicate_and_unknown_version_arguments_are_rejected() {
        assert_eq!(
            ChildServiceBootstrap::required_from_args(&[OsString::from("--mode=service")]),
            Err(EndpointBootstrapError::MissingEndpoint)
        );
        assert_eq!(
            ChildServiceBootstrap::from_args(&[arg("/tmp/foreign.sock")]),
            Err(EndpointBootstrapError::InvalidPath)
        );
        assert_eq!(
            ChildServiceBootstrap::from_args(&[
                arg("/run/nagi/ipc/one.sock"),
                arg("/run/nagi/ipc/two.sock"),
            ]),
            Err(EndpointBootstrapError::DuplicateEndpoint)
        );
        assert_eq!(
            ChildServiceBootstrap::from_args(&[OsString::from(
                "--nagi-service-endpoint-v2=/run/nagi/ipc/one.sock"
            )]),
            Err(EndpointBootstrapError::UnsupportedVersion)
        );
    }

    #[test]
    fn traversal_nul_and_oversized_paths_are_rejected() {
        for path in [
            "/run/nagi/ipc/../other.sock".to_owned(),
            "/run/nagi/ipc/a\0b.sock".to_owned(),
            format!("/run/nagi/ipc/{}.sock", "a".repeat(100)),
        ] {
            assert_eq!(
                ChildServiceBootstrap::from_args(&[arg(&path)]),
                Err(EndpointBootstrapError::InvalidPath)
            );
        }
    }
}
