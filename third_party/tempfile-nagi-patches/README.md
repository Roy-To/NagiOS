# Nagi tempfile patches

These ordered patches adapt the pinned `tempfile` 3.27.0 registry source to
Nagi's real std/VFS filesystem API. The Nagi target uses the std-only backend
and does not compile the Unix `rustix` backend, whose host-oriented libc ABI is
not part of the M17 target contract.
