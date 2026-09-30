# Nagi llama.cpp patches

This directory is the tracked patch boundary for the pinned llama.cpp source in
`third_party/sources.lock`. Number `.patch` files to define application order.
The fetch tool keeps the upstream checkout pristine and builds an ignored,
fingerprinted checkout under `out/cache/llama-cpp-nagi` when patches exist.

No compatibility patch is checked in yet. The Nagi target build currently
fails on upstream C++ exception syntax with the target's `-fno-exceptions`
setting; a patch must preserve parser, allocation, and I/O error handling
before it is added here.
