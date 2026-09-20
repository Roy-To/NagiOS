# Nagi FreeType source patches

This directory contains the numbered Nagi patches applied to the exact
`freetype-sys 0.23.0` registry source declared in `third_party/sources.lock`.
The source is generated under `third_party/freetype-sys/`; do not edit that
checkout directly. The patch keeps bundled FreeType/libpng target builds on
the pinned `libz-sys` include metadata instead of a host zlib include path.
