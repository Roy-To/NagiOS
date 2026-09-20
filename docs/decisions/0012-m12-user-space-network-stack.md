# ADR 0012: Keep network protocols in user space

## Status

Accepted for M12.

## Decision

The kernel provides only a capability-checked raw VirtIO Net device boundary.
`user/nagi-net` owns the protocol stack in user space and uses the pinned
smoltcp 0.12.0 implementation (`d2d647090d544b1e7c142571da9d55f7280f664b`)
through a Nagi `Device` adapter. It implements ARP, IPv4, ICMP, UDP, DHCP,
DNS, TCP, and the bounded HTTP client without host sockets.

## Rationale

This preserves the Nagi kernel boundary: low-level execution, memory, IPC,
and device authority remain in the kernel while policy and protocol evolution
stay restartable and testable in services. It also prevents a POSIX/Linux
socket layer from becoming a hidden production dependency.

## Consequences

- The M12 ABI adds raw-frame send/receive syscalls and a separate network
  capability; it does not add `connect`, `read`, DNS, or HTTP kernel syscalls.
- The Nagi socket-facing API is bounded and poll-oriented; smoltcp owns packet
  parsing, routing, neighbor discovery, retransmission, and protocol state.
- QEMU's user-net backend is treated as the external network, while all packet
  construction, checksums, ARP, TCP state, and HTTP parsing are performed in
  the guest.
