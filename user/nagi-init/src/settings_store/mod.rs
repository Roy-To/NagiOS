pub(crate) mod codec;
pub(crate) mod panel;
pub(crate) mod preferences;

#[cfg(target_os = "linux")]
pub(crate) mod password;

#[cfg(any(target_os = "linux", all(test, target_os = "macos")))]
pub(crate) mod runtime;
