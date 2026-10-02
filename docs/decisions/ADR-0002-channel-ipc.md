# ADR-0002: Channel IPC

**Status:** Accepted

Channels are the primary IPC primitive, with transferable attenuated handles
and VMO/shared memory for large data.

The channel core records the `ProcessId` from the kernel `Process` passed to
`ChannelPair::send` when it queues a message, then returns it as receive
metadata separate from the caller-controlled header and payload. This core
metadata identifies that process instance; mapping it to an application/session
or granting user-space policy authority belongs to a trusted supervisor. A
caller-supplied identity field is never an authority source.

The bootstrap runtime exposes Channel create, send, nonblocking receive, and
handle close through the user ABI. It also exposes a `WAIT`-authorized
readability wait, which the `libnagi` blocking receive wrapper retries around
nonblocking receive. Its bounded manager still runs one
shared-address-space init Process (PID 1), so every message is from that
bootstrap Process and the API does not authenticate an application or session.
The user path exists to exercise message transfer and capability attenuation;
it is not a production service boundary. Authentication requires isolated
processes and a trusted supervisor binding kernel Process IDs, endpoint
delivery, and launch records to application/session policy. Event, Timer,
Process-exit, and service/socket readiness waits remain unavailable in the
bootstrap user ABI.
