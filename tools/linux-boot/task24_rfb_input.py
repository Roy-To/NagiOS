"""Task24 keyboard transport for a dedicated localhost QEMU RFB display."""

from __future__ import annotations

import socket
import struct
import time


class RfbError(ValueError):
    """The server did not offer the expected local RFB contract."""


class RfbKeyboard:
    """RFB 3.8 keyboard client. The caller owns the display and boot deadline."""

    def __init__(self, connection: socket.socket):
        self.connection = connection
        self.display_size = (0, 0)

    @classmethod
    def connect_local(cls, port: int, timeout: float) -> RfbKeyboard:
        if isinstance(port, bool) or not isinstance(port, int) or not 1 <= port <= 65535:
            raise RfbError("RFB port is outside the TCP port range")
        if timeout <= 0:
            raise RfbError("RFB timeout must be positive")
        connection = socket.create_connection(("127.0.0.1", port), timeout=timeout)
        keyboard = cls(connection)
        try:
            peer = connection.getpeername()
            if not isinstance(peer, tuple) or len(peer) < 2 or peer[0] != "127.0.0.1" or peer[1] != port:
                raise RfbError("RFB peer is not the exact IPv4 loopback endpoint")
            keyboard.initialize()
        except BaseException:
            connection.close()
            raise
        return keyboard

    def _read_exact(self, size: int) -> bytes:
        result = bytearray()
        while len(result) < size:
            chunk = self.connection.recv(size - len(result))
            if not chunk:
                raise RfbError("RFB peer disconnected")
            result.extend(chunk)
        return bytes(result)

    def initialize(self) -> None:
        version = b"RFB 003.008\n"
        if self._read_exact(len(version)) != version:
            raise RfbError("expected RFB 3.8")
        self.connection.sendall(version)
        count = self._read_exact(1)[0]
        if count == 0 or 1 not in self._read_exact(count):
            raise RfbError("expected local RFB security type None")
        self.connection.sendall(b"\x01")
        if self._read_exact(4) != b"\x00" * 4:
            raise RfbError("RFB security negotiation failed")
        self.connection.sendall(b"\x01")  # Shared connection.
        header = self._read_exact(24)
        width, height = struct.unpack_from(">HH", header)
        name_length = struct.unpack_from(">I", header, 20)[0]
        if not width or not height or name_length > 1024 * 1024:
            raise RfbError("invalid RFB display initialization")
        self._read_exact(name_length)
        self.display_size = width, height

    def capture_ppm(self) -> bytes:
        """Capture one complete raw RGB frame from the already verified local peer."""
        width, height = self.display_size
        if not width or not height or width * height > 2_097_152:
            raise RfbError("RFB capture dimensions exceed the local evidence limit")
        # SetPixelFormat: 32-bit big-endian RGB 0x00RRGGBB. SetEncodings: raw only.
        self.connection.sendall(
            struct.pack(">BBBBBBBBHHHBBB3x", 0, 0, 0, 0, 32, 24, 1, 1, 255, 255, 255, 16, 8, 0)
            + struct.pack(">BBHi", 2, 0, 1, 0)
            + struct.pack(">BBHHHH", 3, 0, 0, 0, width, height)
        )
        kind, _, count = struct.unpack(">BBH", self._read_exact(4))
        if kind != 0 or not 1 <= count <= 256:
            raise RfbError("expected a bounded RFB framebuffer update")
        rgb = bytearray(3 * width * height)
        coverage = bytearray(width * height)
        remaining_area = width * height
        for _ in range(count):
            x, y, w, h, encoding = struct.unpack(">HHHHi", self._read_exact(12))
            if (encoding != 0 or not w or not h or x + w > width or y + h > height
                    or w * h > remaining_area):
                raise RfbError("unexpected RFB raw rectangle")
            remaining_area -= w * h
            raw = self._read_exact(w * h * 4)
            for row in range(h):
                offset = (y + row) * width + x
                source = raw[4 * w * row:4 * w * (row + 1)]
                rgb[3 * offset:3 * (offset + w):3] = source[1::4]
                rgb[3 * offset + 1:3 * (offset + w):3] = source[2::4]
                rgb[3 * offset + 2:3 * (offset + w):3] = source[3::4]
                coverage[offset:offset + w] = b"\x01" * w
        if 0 in coverage:
            raise RfbError("RFB framebuffer update was incomplete")
        return f"P6\n{width} {height}\n255\n".encode("ascii") + rgb

    def tap(self, keysym: int) -> None:
        """Send a key press and release. ASCII keysyms and UI navigation only."""
        if keysym not in (0xFF08, 0xFF09, 0xFF0D, 0xFF1B) and not 32 <= keysym <= 126:
            raise RfbError("unsupported Task24 keysym")
        self.connection.sendall(struct.pack(">BBHIBBHI", 4, 1, 0, keysym, 4, 0, 0, keysym))

    def type_fixture_text(self, value: str, *, delay_seconds: float = 0.04) -> None:
        """Type the fixture's lower-case ASCII alphabet, digits, spaces and hyphens."""
        if not isinstance(value, str) or any(
            not ("a" <= char <= "z" or "0" <= char <= "9" or char in " -")
            for char in value
        ):
            raise RfbError("fixture text must use lower-case ASCII letters, digits, spaces, or hyphens")
        if not 0 <= delay_seconds <= 1:
            raise RfbError("fixture key delay is outside the supported range")
        for char in value:
            self.tap(ord(char))
            if delay_seconds:
                time.sleep(delay_seconds)

    def close(self) -> None:
        self.connection.close()
