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
- Share the bounded split-ring layout/publication/completion implementation
  between block and network. Read and validate RX/TX lengths independently after
  reset. Both support power-of-two lengths 256/512/1024; unsupported lengths
  fail closed. Network ring length does not expand its 256 receive-buffer pool.
- Modern input explicitly programs and reads back its bounded event queue
  length before enabling it. Supplying separate ring addresses without setting
  queue length left QEMU 10 at 1024 while software wrapped at 64, duplicating or
  losing input after enough events. Legacy input requires its exact supported
  64-entry layout; it cannot silently clamp a device-owned larger ring.
- Publish only those 256 RX descriptor heads into the actual available ring.
  Recycle returned heads only after checking their buffer bounds. Include the
  10-byte VirtIO header in each buffer stride, and notify RX after DRIVER_OK.

A local attempt to pin QEMU `rx_queue_size=256,tx_queue_size=256` did not restore
this contract: QMP still reported 1024-entry device rings. That attempt is
superseded by actual device-sized handling; its launcher properties are removed.
The failed attempt and M12 queue error remain in source-bound evidence. No
configuration-only success is claimed.

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
