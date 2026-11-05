# Staged caller-free Bar summary transport

Status: staged integration, no release activation. Existing action@1 is unchanged.

The signed Supervisor launch remains the source of Bar identity. A separate
bar-summary@1 protocol (ID 0x4241, version 1) carries only one summary tag byte:
OS offer, child request, and negative terminal. Header metadata must be exact,
request ID nonzero, and handles absent. Child checks kernel-stamped init sender
1; init checks the real Supervisor receiver and full live launch record. No
identity, Session, prompt, page, terms or model data is supplied by this payload.

The Desktop's explicit gesture supplies a non-wrapping request nonce, claimed
once even across endpoint replacement or send failure. One exchange expires
within 500 trusted boot ticks (not a millisecond or dispatch bound). A receipt
is consumed once; current Desktop binding, live record, intent and time are
rechecked before actor conversion. A negative terminal burns the receipt and
cannot be interpreted as model success. Caller conversion remains separate from
Browser Context Authority, sharing, terms, model admission and service checks.

Retirement compares/removes the launch record under its existing lock before
closing the endpoint, without waiting for process exit. The owned retirement
token retains close/wait failures. The explicit waiting reap API belongs outside
the latency-sensitive UI step and is not a nonblocking poll. The existing
registry spinlock time and validation/use atomicity are not guaranteed here.

The one-shot child source and grant-free manifest are not signed or packaged into
normal runtime. Normal activation still needs Browser surface/input ownership,
a signed package, lifecycle coordinator with a bounded reaper resource, real
endpoint dispatch and positive model-result/lifetime integration. Channel closure
or a payload report never proves process exit. No absent ownership or
OS provenance is synthesized. Existing M23/M30 acceptance gates remain intact.
