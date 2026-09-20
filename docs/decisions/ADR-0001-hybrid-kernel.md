# ADR-0001: Capability-Based Hybrid Kernel

**Status:** Accepted

Nagi uses a capability-based hybrid kernel. The kernel remains responsible for
low-level execution, memory, IPC, and authority while high-level policy stays
in user-space services.

