pub(crate) mod codec;
pub(crate) mod panel;
pub(crate) mod preferences;

#[cfg(all(target_os = "linux", linux_task24_settings))]
pub(crate) mod password;

#[cfg(any(target_os = "linux", all(test, target_os = "macos")))]
pub(crate) mod runtime;
