"""Extract the x86 kernel offsets consumed by Linux boot/header.S."""

from __future__ import annotations

import re
import sys


_NM_LINE = re.compile(rb"^([0-9a-fA-F]+) [a-zA-Z] ([^\s]+)$")
_ZO_SYMBOL = re.compile(
    rb"(?:startup_32|efi.._stub_entry|efi(?:32)?_pe_entry|input_data|"
    rb"kernel_info|_end|_ehead|_text|_e?data|_e?sbat|z_.*)"
)


def generate_zoffset_header(nm_output: bytes) -> bytes:
    """Return the definitions selected by Linux's sed-zoffset expression."""
    output = bytearray()
    for line in nm_output.splitlines():
        match = _NM_LINE.fullmatch(line)
        if match is None:
            continue
        address, symbol = match.groups()
        if _ZO_SYMBOL.fullmatch(symbol):
            output.extend(b"#define ZO_" + symbol + b" 0x" + address + b"\n")
    return bytes(output)


if __name__ == "__main__":
    sys.stdout.buffer.write(generate_zoffset_header(sys.stdin.buffer.read()))
