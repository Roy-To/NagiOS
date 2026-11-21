use nagi_model::AppId;

/// Run package-side effects only after the caller's expected identity matches
/// the identity parsed from the package. The callback may install, inspect,
/// authenticate, or spawn; none of it is entered on an AppId mismatch.
pub(crate) fn with_expected_app_id<T>(
    actual: AppId,
    expected: AppId,
    after_match: impl FnOnce() -> T,
) -> Result<T, AppId> {
    if actual != expected {
        return Err(actual);
    }
    Ok(after_match())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    use nagi_package::PackageView;

    #[test]
    fn wrong_expected_app_id_skips_install_inspect_provider_and_spawn() {
        let path = std::env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must name the signed Linux package input");
        let bytes = std::fs::read(path).expect("read signed package input");
        let package = PackageView::parse(&bytes).expect("parse signed package input");
        let actual = package.manifest().app_id();
        let wrong_expected = AppId(actual.0.wrapping_add(1));

        let install_calls = Cell::new(0);
        let inspect_calls = Cell::new(0);
        let provider_calls = Cell::new(0);
        let spawn_calls = Cell::new(0);
        let result = with_expected_app_id(actual, wrong_expected, || {
            install_calls.set(install_calls.get() + 1);
            inspect_calls.set(inspect_calls.get() + 1);
            provider_calls.set(provider_calls.get() + 1);
            spawn_calls.set(spawn_calls.get() + 1);
        });

        assert_eq!(result, Err(actual));
        assert_eq!(install_calls.get(), 0);
        assert_eq!(inspect_calls.get(), 0);
        assert_eq!(provider_calls.get(), 0);
        assert_eq!(spawn_calls.get(), 0);
    }
}
