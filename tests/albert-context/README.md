# Albert coherent page context host checks

Run `sh tests/albert-context/check.sh` using the repository's installed toolchain
and existing offline Cargo cache. No new dependency download is required or
performed. This standalone workspace imports the exact production
`user/nagi-albert/src/assistant_context.rs` and its tests. The isolated manifest
avoids resolving unrelated pinned Servo target dependencies for host tests.
It does not compile or execute Servo or a Nagi guest.

`BrowserContextOwner` owns a disabled-by-default sharing policy and the
non-Send/non-Sync epoch from the parent summary boundary. Extraction captures
that epoch before invoking the engine callback, rechecks policy and epoch after
it, and stamps only a coherent success. No automatic retry, model request or
terms acknowledgement occurs. The engine/event adapter must invalidate this
same owner before relevant page/selection/content/tab mutations, including ABA.
Sharing controls invalidate before policy changes. Existing unversioned snapshot
helpers remain compatible with synchronous acceptance consumers.

This helper does not authenticate CallerIdentity or supply a live Desktop
owner. Its test actors and extraction callbacks are synthetic orchestration
fixtures. A production connector must receive actual kernel-stamped sender
metadata on its owned endpoint, resolve the live Supervisor launch record,
bind the authenticated Session and active request, and revalidate lifecycle.
Caller fields in payloads, a user-supplied PID or the foreground Albert identity
cannot serve as the requesting Bar actor. No such endpoint/actor binding is
implemented by this test or by the helper.

Normal Desktop still passes no owner. The existing live Browser API in
m18_acceptance.rs is acceptance-only and pumps events between text evaluations;
it has not been wired to this owner or proven coherent by these host checks.
Real Servo callbacks, coherent multi-field extraction, IPC, guest cancellation
and display remain unverified. Do not turn host success into M23/M30 PASS.

The retained event notifier is `BrowserContextEvents`. It can only invalidate
on the owner's thread and cannot read pages, stamp snapshots or enable sharing.
Its short internal borrow is released between operations, so an engine callback
can revoke during extraction even when an outer caller holds a RefCell borrow
of BrowserContextOwner. Dropping the owner drops its epoch immediately; retained
notifier clones become inert and cannot extend the lifetime of page access.

The guest first-pixel source now separates `start_first_web_pixel` and
`FirstPixelBrowser::step` from the old diverging runner. Step makes one Servo
dispatch call and returns a wake hint; it has no proven wall-clock bound, and
false does not mean shutdown. The old runner still yields on an idle hint.
URL/load/frame callbacks invalidate the retained notifier conservatively. This
subset does not cover all DOM/selection/tab changes, does not expose an extraction
API and leaves sharing disabled. The constructor still has the existing fail-stop
resource setup and owns the first-pixel display surface. It is not normal Desktop
resource/lifecycle/session wiring. The host harness parses/formats lib.rs but
DOES NOT compile its cfg(target_os="nagi") guest body or execute Servo. A real
target check remains required before using the new guest step API; the producer
attempt stopped at absent third_party/cc-nagi source, before Rust compilation.
