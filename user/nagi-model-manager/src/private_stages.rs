//! Fixed in-process diagnostics. No observer return value conveys authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivateInstalledStage {
    RetainedTakeBegin,
    RetainedTakeDone,
    SelectionVerifyBegin,
    SelectionVerifyDone,
    RuntimeVerifyBegin,
    RuntimeVerifyDone,
    RuntimeProofBegin,
    RuntimeProofDone,
    BackendLoadBegin,
    BackendLoadDone,
}
pub type PrivateInstalledStageObserver = fn(PrivateInstalledStage);

pub(crate) fn observe(
    observer: Option<PrivateInstalledStageObserver>,
    stage: PrivateInstalledStage,
) {
    #[cfg(feature = "private-installed-acceptance-diagnostics")]
    if let Some(observer) = observer {
        observer(stage);
    }
    #[cfg(not(feature = "private-installed-acceptance-diagnostics"))]
    let _ = (observer, stage);
}
