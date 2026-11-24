import struct
import unittest
from unittest.mock import Mock, call, patch

from task24_rfb_input import RfbError, RfbKeyboard


class RfbKeyboardTests(unittest.TestCase):
    def test_connect_and_send_one_press_and_release(self):
        connection = Mock()
        connection.getpeername.return_value = ("127.0.0.1", 5903)
        header = struct.pack(">HH16xI", 640, 480, 4)
        connection.recv.side_effect = [
            b"RFB ", b"003.008\n", b"\x02", b"\x02\x01", bytes(4), header, b"test"
        ]
        with patch("task24_rfb_input.socket.create_connection", return_value=connection) as connect:
            keyboard = RfbKeyboard.connect_local(5903, timeout=1)
        connect.assert_called_once_with(("127.0.0.1", 5903), timeout=1)
        keyboard.tap(ord("a"))
        self.assertEqual(connection.sendall.call_args_list, [
            call(b"RFB 003.008\n"),
            call(b"\x01"),
            call(b"\x01"),
            call(bytes.fromhex("04 01 00 00 00 00 00 61 04 00 00 00 00 00 00 61")),
        ])
        with self.assertRaisesRegex(RfbError, "unsupported"):
            keyboard.tap(0xFFFF)
        self.assertEqual(connection.sendall.call_count, 4)

    def test_raw_frame_capture_uses_the_negotiated_display_and_returns_ppm(self):
        connection = Mock()
        keyboard = RfbKeyboard(connection)
        keyboard.display_size = (2, 1)
        connection.recv.side_effect = [
            struct.pack(">BBH", 0, 0, 1),
            struct.pack(">HHHHi", 0, 0, 2, 1, 0),
            bytes.fromhex("00 ff 00 00 00 00 00 ff"),
        ]
        self.assertEqual(keyboard.capture_ppm(), b"P6\n2 1\n255\n" + bytes.fromhex("ff 00 00 00 00 ff"))
        self.assertTrue(connection.sendall.call_args.args[0].endswith(struct.pack(">BBHHHH", 3, 0, 0, 0, 2, 1)))

    def test_unexpected_security_is_rejected_and_connection_closed(self):
        connection = Mock()
        connection.getpeername.return_value = ("127.0.0.1", 5903)
        connection.recv.side_effect = [b"RFB 003.008\n", b"\x01", b"\x02"]
        with patch("task24_rfb_input.socket.create_connection", return_value=connection):
            with self.assertRaisesRegex(RfbError, "security type"):
                RfbKeyboard.connect_local(5903, timeout=1)
        connection.sendall.assert_called_once_with(b"RFB 003.008\n")
        connection.close.assert_called_once()

    def test_remote_peer_is_rejected_before_handshake(self):
        connection = Mock()
        connection.getpeername.return_value = ("::1", 5903, 0, 0)
        with patch("task24_rfb_input.socket.create_connection", return_value=connection):
            with self.assertRaisesRegex(RfbError, "exact IPv4 loopback endpoint"):
                RfbKeyboard.connect_local(5903, timeout=1)
        connection.sendall.assert_not_called()
        connection.close.assert_called_once()

    def test_fixture_text_restricts_ascii_and_sends_key_transitions(self):
        connection = Mock()
        keyboard = RfbKeyboard(connection)
        with patch("task24_rfb_input.time.sleep"):
            keyboard.type_fixture_text("a1-z")
        self.assertEqual(connection.sendall.call_count, 4)
        with self.assertRaisesRegex(RfbError, "lower-case ASCII"):
            keyboard.type_fixture_text("A")

    def test_partial_initialization_reports_disconnect(self):
        connection = Mock()
        connection.recv.side_effect = [b"RFB ", b""]
        with self.assertRaisesRegex(RfbError, "disconnected"):
            RfbKeyboard(connection).initialize()
        connection.sendall.assert_not_called()


if __name__ == "__main__":
    unittest.main()
