# ADR 0072: explicit reference VirtIO queue geometry

Status: local implementation candidate; no release activation.
Date: 2026-11-04.

## Evidence

The byte-identical ordinary native image at source `fd5972b` completes block
requests on QEMU 8.2.2 with a 256-entry ring. QEMU 10.0.13 exposes a 1024-entry
ring. The old block driver accepted any length at least 256 but used fixed
256-entry descriptor/available/used offsets. QMP shows the device looking at
base+16384/base+20480 while the driver writes base+4096/base+8192; kicks are
observed without consumption and M7 times out.

The variable block layout at `4f4ad9d` consumes requests on QEMU 10 and passes
M7. That reveals the next incompatibility: VirtIO Net exposes 1024-entry queues
and the network driver's existing bounded 256-entry contract rejects it. The
layout fixture reaches persistent User Data but its real M13 network operation
fails. This is not formal M30 acceptance.

## Decision

- Size the block driver's rings from queue zero's actual length after device
  reset. Support power-of-two lengths 256/512/1024 within one page-aligned
  32 KiB region. Larger, smaller or malformed lengths fail closed. Use the
  actual length for publication and completion/wrap indexing, and volatile
  DMA accesses. Do not change timeouts, capability boundaries or test outcomes.
- Keep the network driver's existing 256-entry ring and receive-buffer bound.
  Set reference QEMU `rx_queue_size=256,tx_queue_size=256` explicitly so QEMU
  machine-version defaults cannot silently change that hardware contract.
  VirtIO Network remains present and handles actual guest packets.
- Validate each selected RX/TX queue after reset and require its exact supported
  length. The former TX `>=256` check could accept an incompatible layout;
  tightening it prevents that failure. Arbitrary 1024-entry network devices
  remain unsupported by this driver.

The reference machine still uses q35, 4 vCPU, 8 GiB, UEFI and VirtIO Block/
Network/GPU/Sound/RNG. This change does not introduce host networking in place
of the guest stack or turn a fixture result into ordinary session acceptance.

## Validation scope

Kernel tests cover original and larger block layouts, index wrap and invalid
sizes. Guest validation must exercise real disk reads/writes and network traffic
on the reference configuration, including the retained layout/Recovery gates.
Native final linking, successful model inference, Browser/History/voice/stress,
distribution review and formal M30 are separate gates. A model-free image cannot
establish inference. No PR update, publication or merge is part of this local work.
