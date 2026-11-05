use super::*;
use crate::{m19_files_client as client, m19_search_service as service, supervisor};
use nagi_search_ipc::*;

fn setup() -> (Desktop, UserDataVolume, Vec<u32>) {
    libnagi::set_readiness(true);
    let (mut desktop, mut volume, surface, _) = existing_account(Locale::EnUs);
    unlock(&mut desktop, &mut volume);
    desktop.desktop_focus = Some(DesktopFocus::Application(2));
    desktop.focused[2] = true;
    desktop.files_search_query[0] = b'x';
    desktop.files_search_query_len = 1;
    (desktop, volume, surface)
}
fn status() -> libnagi::ProcessExitStatus {
    libnagi::ProcessExitStatus {
        process_id: 7,
        kind: libnagi::PROCESS_EXIT_KIND_EXITED,
        code: 0,
        fault_vector: 0,
    }
}
fn request() -> libnagi::ChannelReceiveResult {
    let mut m = libnagi::ChannelReceiveResult::default();
    m.sender_process_id = 7;
    m.protocol_id = PROTOCOL_ID;
    m.version = PROTOCOL_VERSION;
    m.opcode = OPCODE_SEARCH;
    m.request_id = 1;
    m.payload_len = encode_request(
        SearchRequest {
            kind: KindFilter::File,
            text: "x",
        },
        &mut m.payload,
    )
    .unwrap() as u32;
    m
}
fn relay() -> libnagi::ChannelReceiveResult {
    let mut m = request();
    m.opcode = OPCODE_RESULT_RELAY;
    let mut results = SearchResults::status_only(ResultStatus::Ok);
    results.count = 1;
    results.visible_total = 1;
    results.ids[0] = 42;
    m.payload_len = encode_results(&results, &mut m.payload).unwrap() as u32;
    m
}

#[test]
fn delayed_request_and_relay_progress_then_clean_reap() {
    let (mut desktop, mut volume, mut surface) = setup();
    let user = desktop.signed_in_session().unwrap();
    let generation = desktop.files_search_generation();
    let launched = supervisor::host_fixture::launch();
    libnagi::script_files(
        vec![None, Some(request()), None, Some(relay())],
        vec![None, Some(status())],
    );
    libnagi::script_modal(vec![None; 6], false);
    let mut progress = || {
        client::wait_progress(
            &mut desktop,
            1,
            2,
            &mut surface,
            &mut volume,
            &user,
            generation,
            100,
        )
    };
    let results = service::exchange_files_one(
        77,
        7,
        |ep, msg| client::receive_responsive(ep, msg, &mut progress),
        |access, text| {
            assert_eq!(access.app_id, Some(launched.record.app_id));
            assert_eq!(text, "x");
            Some(vec![nagi_model::ObjectId(42)])
        },
    )
    .unwrap();
    assert_eq!(results.ids(), &[42]);
    assert!(supervisor::exited_cleanly(
        &client::reap_responsive(&launched, &mut progress).unwrap()
    ));
    assert!(supervisor::resolve(7).is_none());
    assert_eq!(libnagi::modal_counts().0, 6);
    assert_eq!(libnagi::files_counts(), (4, 2, 2, vec![77]));
}

#[test]
fn lock_or_escape_invalidates_before_late_reply_is_consumed() {
    for event in [
        key(62),
        key(libnagi::INPUT_KEY_ESCAPE),
        key(30),
        key(libnagi::INPUT_KEY_ENTER),
    ]
    .into_iter()
    .filter(|event| cfg!(feature = "m20-model-service") || event.code != 62)
    {
        let (mut desktop, mut volume, mut surface) = setup();
        let user = desktop.signed_in_session().unwrap();
        let generation = desktop.files_search_generation();
        let launched = supervisor::host_fixture::launch();
        libnagi::script_files(vec![None, Some(relay())], vec![None, None, Some(status())]);
        libnagi::script_modal(vec![None, Some(event)], false);
        let mut progress = || {
            client::wait_progress(
                &mut desktop,
                1,
                2,
                &mut surface,
                &mut volume,
                &user,
                generation,
                100,
            )
        };
        assert!(client::receive_responsive(
            77,
            &mut libnagi::ChannelReceiveResult::default(),
            &mut progress
        )
        .is_none());
        assert_eq!(libnagi::files_counts().0, 1, "late reply was not adopted");
        let mut cleanup = None;
        client::abort_and_reap(launched, 1, &mut cleanup);
        assert_eq!(cleanup, Some(7));
        assert!(supervisor::resolve(7).is_none());
        assert_eq!(libnagi::files_counts().3, vec![77]);
        client::poll_cleanup(&mut cleanup);
        assert_eq!(cleanup, Some(7));
        client::poll_cleanup(&mut cleanup);
        assert_eq!(cleanup, None);
        assert!(!desktop.files_search_complete);
    }
}

#[test]
#[cfg(feature = "m20-model-service")]
fn lock_during_reap_revokes_and_defers_exit_collection() {
    let (mut desktop, mut volume, mut surface) = setup();
    let user = desktop.signed_in_session().unwrap();
    let generation = desktop.files_search_generation();
    let launched = supervisor::host_fixture::launch();
    libnagi::script_files(vec![], vec![None, None, Some(status())]);
    libnagi::script_modal(vec![None, Some(key(62))], false);
    assert!(client::reap_responsive(&launched, || client::wait_progress(
        &mut desktop,
        1,
        2,
        &mut surface,
        &mut volume,
        &user,
        generation,
        100
    ))
    .is_none());
    assert!(desktop.session.is_none());
    let mut cleanup = None;
    client::abort_and_reap(launched, 1, &mut cleanup);
    assert_eq!(cleanup, Some(7));
    assert!(supervisor::resolve(7).is_none());
    client::poll_cleanup(&mut cleanup);
    assert_eq!(cleanup, None);
}

#[test]
fn timeout_and_display_failure_stop_without_consuming_queued_reply() {
    for timeout in [true, false] {
        let (mut desktop, mut volume, mut surface) = setup();
        let user = desktop.signed_in_session().unwrap();
        let generation = desktop.files_search_generation();
        libnagi::script_files(vec![Some(relay())], vec![]);
        libnagi::script_modal(
            vec![Some(InputEvent {
                event_type: libnagi::INPUT_EVENT_REL,
                code: libnagi::INPUT_REL_X,
                value: 1,
            })],
            !timeout,
        );
        if timeout {
            libnagi::set_ticks(client::WAIT_TICKS);
        }
        assert!(client::receive_responsive(
            77,
            &mut libnagi::ChannelReceiveResult::default(),
            || client::wait_progress(
                &mut desktop,
                1,
                2,
                &mut surface,
                &mut volume,
                &user,
                generation,
                100
            )
        )
        .is_none());
        assert_eq!(libnagi::files_counts().0, 0);
    }
}

#[test]
fn wrong_sender_and_modified_relay_are_rejected() {
    for wrong_sender in [true, false] {
        let _launch = supervisor::host_fixture::launch();
        let mut late = relay();
        if wrong_sender {
            late.sender_process_id = 99;
        } else {
            late.payload[4] ^= 1;
        }
        libnagi::script_files(vec![Some(request()), Some(late)], vec![]);
        assert!(service::exchange_files_one(
            77,
            7,
            |ep, msg| client::receive_responsive(ep, msg, || true),
            |_, _| Some(vec![nagi_model::ObjectId(42)])
        )
        .is_none());
    }
}

#[test]
#[cfg(feature = "m20-model-service")]
fn search_wait_drains_model_reply_with_idle_and_continuous_input() {
    for idle in [true, false] {
        let (mut desktop, mut volume, mut surface) = pending_model_with_consent();
        desktop.consent = None;
        assert!(desktop
            .model_services
            .as_ref()
            .unwrap()
            .host_test_complete_request());
        let user = desktop.signed_in_session().unwrap();
        let generation = desktop.files_search_generation();
        libnagi::script_files(vec![None, Some(relay())], vec![]);
        libnagi::script_modal(vec![if idle { None } else { Some(key(61)) }; 2], false);
        assert!(client::receive_responsive(
            77,
            &mut libnagi::ChannelReceiveResult::default(),
            || client::wait_progress(
                &mut desktop,
                1,
                2,
                &mut surface,
                &mut volume,
                &user,
                generation,
                100
            )
        )
        .is_some());
        assert_eq!(
            desktop.bar.controller.status(),
            session_ui::Status::Complete
        );
        assert_eq!(
            desktop.bar.controller.output(),
            "scripted close/reopen reply"
        );
        assert_eq!(libnagi::modal_counts().0, 2);
    }
}

#[test]
fn reap_timeout_returns_without_blocking_and_can_collect_later() {
    let (mut desktop, mut volume, mut surface) = setup();
    let user = desktop.signed_in_session().unwrap();
    let generation = desktop.files_search_generation();
    let launched = supervisor::host_fixture::launch();
    libnagi::script_files(vec![], vec![None, None, Some(status())]);
    libnagi::script_modal(vec![None], false);
    let mut iterations = 0;
    assert!(client::reap_responsive(&launched, || {
        iterations += 1;
        if iterations == 2 {
            libnagi::set_ticks(client::WAIT_TICKS);
        }
        client::wait_progress(
            &mut desktop,
            1,
            2,
            &mut surface,
            &mut volume,
            &user,
            generation,
            100,
        )
    })
    .is_none());
    assert_eq!(libnagi::files_counts().1, 1);
    let mut cleanup = None;
    client::abort_and_reap(launched, 1, &mut cleanup);
    assert_eq!(cleanup, Some(7));
    client::poll_cleanup(&mut cleanup);
    assert_eq!(cleanup, None);
}
