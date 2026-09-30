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

The current bootstrap runtime does not expose this channel core through user
syscalls and still runs one shared-address-space init process. This metadata
therefore does not by itself authenticate an application or session; that
requires process isolation and a trusted user-space supervisor binding.
