const DESKTOP: &str = include_str!("../../../user/nagi-init/src/desktop.rs");
const M19_RUNTIME: &str = include_str!("../../../user/nagi-init/src/m19_runtime.rs");
const LOGIN_SCREEN: &str = include_str!("../../../user/nagi-init/src/login_screen.rs");
const INIT_CARGO: &str = include_str!("../../../user/nagi-init/Cargo.toml");

fn compact(source: &str) -> String {
    source.chars().filter(|ch| !ch.is_whitespace()).collect()
}

#[test]
fn files_search_is_wired_to_the_desktop() {
    let ui_search = compact(DESKTOP);
    assert!(ui_search.contains("fnhandle_files_search_key("));
    assert!(ui_search.contains("fntake_files_search_request("));
    assert!(ui_search.contains("desktop.files.search.label"));
    assert!(compact(M19_RUNTIME).contains("fnsearch_file_titles("));
}

#[test]
fn password_change_acceptance_does_not_finish_on_sign_in_or_cancel() {
    let desktop = compact(DESKTOP);
    let ready_start = desktop
        .find("pubfnacceptance_ready(")
        .expect("desktop acceptance readiness gate");
    let ready = &desktop[ready_start..];
    assert!(ready.contains("self.password_change_succeeded"));

    let init_cargo = compact(INIT_CARGO);
    assert!(
        init_cargo.contains("desktop-password-change-acceptance=[\"desktop-login-acceptance\"]")
    );

    let login_screen = compact(LOGIN_SCREEN);
    assert!(login_screen.contains("ChangeAction::Cancel=>{"));
    assert!(login_screen.contains("ChangeOutcome::PasswordChanged"));
    assert!(login_screen.contains("NagipasswordchangePASS"));
}
