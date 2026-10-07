const COMMANDS: &str = include_str!("commands.rs");
const DESKTOP: &str = include_str!("../../../user/nagi-init/src/desktop.rs");
const M19_RUNTIME: &str = include_str!("../../../user/nagi-init/src/m19_runtime.rs");
const LOGIN_SCREEN: &str = include_str!("../../../user/nagi-init/src/login_screen.rs");
const INIT_CARGO: &str = include_str!("../../../user/nagi-init/Cargo.toml");

fn compact(source: &str) -> String {
    source.chars().filter(|ch| !ch.is_whitespace()).collect()
}

#[test]
fn login_acceptance_changes_the_password_and_checks_it_after_restart() {
    let commands = compact(COMMANDS);
    let start = commands
        .find("fnrun_login_acceptance(")
        .expect("login acceptance runner");
    let end = commands[start..]
        .find("///Anacceptanceevent")
        .map(|offset| start + offset)
        .expect("login acceptance runner end");
    let login = &commands[start..end];

    assert!(login.contains("Some(\"desktop-password-change-acceptance\")"));
    assert!(login.contains("\"password-change\""));
    assert!(login.contains("\"verify-password\""));
    for event in [
        "qmp_typed_keys(\"wrong1\",\"tab\")",
        "qmp_typed_keys(\"nagi1\",\"tab\")",
        "qmp_typed_keys(\"nagi2\",\"tab\")",
        "qmp_typed_keys(\"nagi2\",\"ret\")",
        "qmp_typed_keys(\"nagi1\",\"ret\")",
    ] {
        assert!(login.contains(event), "missing QMP input {event}");
    }
    for marker in [
        "Nagi password change READY",
        "Nagi password change REJECTED current",
        "Nagi password change PASS",
        "Nagi login unlock REJECTED",
        "Nagi login unlocked PASS",
    ] {
        assert!(
            login.contains(&compact(marker)),
            "missing guest marker {marker}"
        );
    }

    let change = login.find("\"password-change\"").unwrap();
    let verify = login.find("\"verify-password\"").unwrap();
    assert!(
        change < verify,
        "credential verification must follow the change"
    );
    let change_phase = &login[change..verify];
    assert!(change_phase.contains("NagipasswordchangeREJECTEDcurrent"));
    assert!(change_phase.contains("NagipasswordchangePASS"));
    for marker in [
        "Nagi M19 signed-in desktop Files query PASS",
        "Nagi M19 signed-in desktop non-UTF-8 filename isolation PASS",
        "Nagi M19 signed-in desktop Files ObjectId initial persist PASS",
        "Nagi M19 signed-in desktop SearchService ready PASS",
        "Nagi M19 signed-in desktop Files UI Search PASS",
    ] {
        assert!(
            change_phase.contains(&compact(marker)),
            "password-change boot must verify `{marker}`"
        );
    }
    let verify_phase = &login[verify..];
    assert!(verify_phase.contains("NagiloginunlockREJECTED"));
    assert!(verify_phase.contains("NagiloginunlockedPASS"));
    for marker in [
        "Nagi M19 signed-in desktop Files query PASS",
        "Nagi M19 signed-in desktop non-UTF-8 filename isolation PASS",
        "Nagi M19 signed-in desktop Files ObjectId restore PASS",
        "Nagi M19 signed-in desktop SearchService ready PASS",
    ] {
        assert!(
            verify_phase.contains(&compact(marker)),
            "password verification boot must verify `{marker}`"
        );
    }

    assert!(login.contains("qmp_typed_keys(\"runtime\",\"ret\")"));
    assert!(login.contains("@screenshot:files-search.png"));
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
