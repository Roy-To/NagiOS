# M9 Display/Input/First Window Decision

**Milestone:** M9

**Decision:** Use the UEFI GOP framebuffer provisioned by the QEMU VirtIO VGA
device as the initial real scanout, expose it through a bounded display
capability and shared Surface VMO, and implement raw VirtIO keyboard/mouse
input in the kernel. Keep Window Server, focus, composition, clipping, and
window movement in user space.

**Reason:** The loader already validates and preserves a real framebuffer
address, while QEMU already attaches the required VirtIO VGA and input devices.
This adds the missing Nagi device/service boundary without putting window
policy in the kernel or using host rendering/input as a substitute.

**Alternatives considered:**

1. A kernel-owned compositor would reach a visual result sooner but violates
   the kernel boundary.
2. Host screenshots or host input libraries would not prove guest behavior.
3. A complete VirtIO GPU control-queue resource manager is a later refinement;
   the M9 scanout is the real GOP/ VirtIO VGA scanout already initialized by
   UEFI, wrapped by a Nagi DisplayDevice primitive.

**Consequences:** M9 uses a fixed bootstrap Surface VMO and typed bootstrap
capabilities. General VMO creation, process-spawned Window Server services, and
full VirtIO GPU resource management remain follow-up work, but no high-level
window syscall or host escape is introduced.
