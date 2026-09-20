# Nagi Servo libc patch boundary

Servo's pinned workspace requires libc 0.2.189 because some locked
dependencies require a newer libc API than the M13 libc 0.2.174 package. This
directory patches the exact crates.io 0.2.189 source with the existing Nagi
user-space POSIX ABI surface; it does not replace the M13 libc source.

`third_party/libc-servo/` is generated from the locked archive, patched in
numeric order, fingerprinted, and never repaired in place. The patch removes
foreign C-library link directives for Nagi and exposes the Nagi ABI bindings
needed by Rust std and Servo's user-space runtime.
