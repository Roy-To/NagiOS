# C1 review focus

Target code commit: `ab0117d` — C1 authenticated caller and Session lease contract.

Please review this commit for high-impact correctness or security issues, with focus on:

- Whether `AuthenticatedInteractiveCaller` is produced only after real OS peer authentication and is never built from request-supplied IDs; whether `AuthenticatedSystemServicePrincipal` remains a separate OS-credential plus endpoint-ownership path.
- Whether the typed C1 APIs prevent Session issuance, logout, or profile access through a plain `TrustedCallerContext`; test-only implementations must not imply a production proof.
- Whether a durable active Session can ever recreate a missing in-process lease, and whether logout, expiry, recovery, service drop, and failed persistence preserve the required revoke ordering.
- Whether `LiveSnapshot` is safely limited to nonblocking connection maintenance: it does not read durable current state and must never authorize protected work. Protected operations still need full `check_live` revalidation.
- Whether lease generation allocation, expiry at `now >= expires_at`, clock failure handling, and process restart make stale connection state fail closed without introducing persisted or reusable lease generations.
- Whether `LocalAccountBinding` remains trusted integration input: C1 persists User/Profile and Session lifecycle; the account-to-User/Profile mapping source and its persistence remain with its owner.

Scope is C1 contract and adapter changes only. C2 wire/IDL, C3 launch/FD/credential application, E Resource/Revision schema, and A root Cargo/CI/image remain with their owners. No auth method, expiry duration, or external monotonic anchor is selected here. Host focused tests passed; Linux Target acceptance and actual C2/C3 wiring remain unverified.
