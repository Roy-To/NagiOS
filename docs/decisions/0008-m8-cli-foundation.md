# M8 CLI Foundation Decision

M8 keeps the existing one-shot default init image for `nagi run` and builds a
feature-selected shell init image for `nagi shell`. This preserves the accepted
M5-M7 regression path while allowing the real serial shell to remain resident
after the M7 persistence check.

The bounded bootstrap user image window is expanded from eight to sixteen 4 KiB
pages. The shell's parser, VFS command buffers, and diagnostic snapshot buffers
must fit in that fixed window; the kernel still validates every ELF segment and
rejects images that exceed the limit. This is a bounded M8 evolution, not a
general process loader or demand-paged address space.

The kernel exposes only low-level console input and fixed process/memory/log
snapshots. File semantics, command parsing, and shell policy remain in user
space.

