# Nagi Surfman patch boundary

This directory contains the numbered, reproducible Nagi adapter patches applied
to the pinned Surfman source at revision
`205778f497327c573929c7b471194390e15f331d`.

The adapter selects Surfman's Mesa surfaceless backend for `target_os = "nagi"`,
excludes X11/Wayland dependencies, and resolves EGL symbols from the statically
linked guest Mesa implementation. It does not load a host display or host GL
driver. The guest image must provide the pinned Mesa EGL/Softpipe symbols.

Do not edit `third_party/surfman` directly. `nagi fetch` creates that generated
checkout, applies these patches in numeric order, records a revision/patch/
worktree fingerprint, and refuses to repair an existing mismatched checkout.
