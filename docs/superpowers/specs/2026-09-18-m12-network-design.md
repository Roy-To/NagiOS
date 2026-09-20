# M12 Networking Design

## Goal

Deliver a real network path for the Nagi 0.1 reference VM without moving
network policy or high-level protocols into the kernel. The M12 acceptance
must generate an actual guest-owned HTTP request and validate the response
received through the guest's own Ethernet, IPv4, and TCP implementation.

## Boundaries

- The kernel owns only the low-level VirtIO Net device, bounded DMA buffers,
  and capability-checked raw-frame send/receive operations.
- `user/nagi-net` owns Ethernet framing, ARP, IPv4, ICMP, UDP, DHCP, DNS, TCP,
  and the bounded HTTP client used by the acceptance app.
- `libnagi` exposes only raw network device wrappers and does not implement
  TCP, DNS, or HTTP.
- No host socket is used as a guest socket and no host response is copied into
  the guest. QEMU user-net is the configured external network backend; the
  acceptance fixture is an ordinary endpoint reachable at the QEMU gateway
  address.
- The initial reference path uses a bounded static IPv4 configuration for the
  HTTP acceptance while the DHCP/DNS packet paths are independently covered by
  deterministic user-space tests. This keeps the first real TCP path
  diagnosable while retaining the protocol deliverables.

## Guest network contract

- QEMU presents a legacy-compatible `virtio-net-pci` device so the current
  bootstrap driver can use the same bounded legacy queue model as VirtIO
  Block. The device is explicitly configured with `disable-modern=on`.
- The guest interface uses `10.0.2.15/24`, gateway `10.0.2.2`, and the
  acceptance HTTP endpoint `10.0.2.2:18080`.
- Ethernet frames are capped at 1536 bytes. RX and TX queues are bounded and
  serialized by kernel locks; malformed or oversized frames are dropped.
- Net capabilities are bootstrapped as a separate capability and are never
  inferred from a path, port, or user-supplied object ID.

## Protocol subset

`nagi-net` implements the protocol fields needed for the reference VM and
rejects malformed input:

1. Ethernet II with IPv4 and ARP EtherTypes;
2. ARP request/reply and bounded neighbor cache;
3. IPv4 header checksum, source/destination and protocol validation;
4. ICMP echo request/reply;
5. UDP checksum and bounded datagrams, including DHCP and DNS message helpers;
6. TCP three-way handshake, sequence/acknowledgement validation, payload
   receive, FIN/RST handling, and bounded retransmission timeout;
7. HTTP/1.0 request construction and response status/header/payload parsing.

The API is poll-oriented and allocation-free for the preview. It exposes
object-like `Socket` handles in user space and returns explicit timeout,
malformed-packet, checksum, and connection-state errors.

## Acceptance

The M12 command builds the guest image and runs QEMU with VirtIO Net. The
PowerShell and Git Bash acceptance scripts start a temporary host HTTP fixture
that is reachable through QEMU user-net as `10.0.2.2:18080`, launch the guest,
and require ordered guest markers for network setup, ARP, TCP SYN/SYN-ACK,
HTTP response parsing, and final M12 PASS. The fixture process is cleaned up
by the script and is not a substitute for guest networking.

## Non-goals

- No Linux socket or host networking dependency in the production guest.
- No high-level network syscalls in the kernel.
- No TLS, IPv6, congestion-control optimization, or general-purpose browser
  networking in M12; those remain later scope.
