# Nagi relibc patches

`0001-nagi-portable-header-find.patch` replaces GNU-only `find -printf` in
the pinned relibc header-generation Makefile with portable `find -exec
basename`. `tools/mesa/build.sh` applies this patch with an exact check before
generating the target C headers. It changes only the build host command, not
the generated ABI or target runtime.
