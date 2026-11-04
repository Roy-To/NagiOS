# Ordinary model-service and summary admission checks

This standalone workspace compiles exact production leaves. Host tests exercise
orchestration/admission only; no Nagi syscall, model, VM or native inference is
run. The actual SessionModelService wrapper's host tests use panic/abort-only
syscall, artifact and native entry hooks. Reaching one fails the suite rather
than providing fake output. Nagi-target builds use the real implementations;
those hooks exist only under cfg(test) on non-Nagi targets.

    cargo test --locked --offline --manifest-path tests/model-session-service/Cargo.toml
    cargo clippy --locked --offline --manifest-path tests/model-session-service/Cargo.toml --all-targets -- -D warnings
    cargo check --locked --offline --manifest-path tests/model-session-service/Cargo.toml --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,alloc
    cargo clippy --locked --offline --manifest-path tests/model-session-service/Cargo.toml --features granite-acceptance --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,alloc -- -D warnings

The 20 host tests comprise 11 transferred bridge/session-access cases, 5 actual
wrapper rejection/admission/cancellation cases, 1 synchronous typed-error versus
cancellation precedence case, and 3 tests of Albert's actual sharing policy.
The real LazyModelService fixture uses synthetic artifacts and a test backend.
No production license is acknowledged: the real wrapper explicitly tests
LicenseAcknowledgementRequired after enabling only synthetic context sharing.
Host fixture Session and caller values do not establish OS invocation provenance.

NAGI_BAR_APP_ID is shared through nagi-ai and re-exported by Albert. It identifies
the requesting assistant, not the foreground browser. The summary entry checks
that AppId, full logical CallerIdentity equality, the bound live Session and
Context Authority/public Browser policy. Ordinary external Some(AppId) model
requests remain denied. Trusted system instructions and untrusted page data stay
separate; size/output caps, request checks and cancellation precedence remain.

These checks do not connect the summary method to the worker/UI, prove its
lock/sign-out propagation or stale Session/request display filtering, or prove
post-acquisition page/navigation/sharing freshness. A copied Session cannot
observe a later lock. A freely constructed CallerIdentity is not OS provenance.
The future transport must enforce those properties before ordinary use. No
end-to-end M23 or M30 PASS follows from this harness. Target checks prove compile
coverage only; init still reports unused summary-entry warnings because it is
not connected. Full ELF linking and guest/model execution are separate checks.
