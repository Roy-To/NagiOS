import hashlib
import json
import stat
import tempfile
import unittest
from pathlib import Path

import system_verity
from linux_nm_zoffset import generate_zoffset_header


class SystemVerityTests(unittest.TestCase):
    @staticmethod
    def valid_config() -> bytes:
        return b"\n".join(
            f"{name}=y".encode()
            for name in (
                "CONFIG_X86_64", "CONFIG_BLK_DEV_INITRD", "CONFIG_BINFMT_ELF", "CONFIG_PRINTK",
                "CONFIG_SERIAL_8250", "CONFIG_SERIAL_8250_CONSOLE", "CONFIG_BLOCK", "CONFIG_PCI",
                "CONFIG_VIRTIO", "CONFIG_VIRTIO_PCI", "CONFIG_VIRTIO_BLK", "CONFIG_BLK_DEV_DM",
                "CONFIG_DM_INIT", "CONFIG_DM_VERITY", "CONFIG_CRYPTO_SHA256", "CONFIG_EXT4_FS",
                "CONFIG_DEVTMPFS", "CONFIG_PROC_FS", "CONFIG_SYSFS",
            )
        )

    def test_darwin_zoffset_parser_preserves_mixed_efi_entry_offsets(self):
        nm_output = (
            b"0000000000001000 T startup_32\n"
            b"0000000000001200 T efi32_stub_entry\n"
            b"0000000000001400 T efi64_stub_entry\n"
            b"0000000000002000 T efi32_pe_entry\n"
            b"0000000000003000 T kernel_info\n"
            b"0000000000004000 T input_data\n"
            b"0000000000005000 T unrelated_symbol\n"
        )
        header = generate_zoffset_header(nm_output)
        self.assertIn(b"#define ZO_efi32_stub_entry 0x0000000000001200\n", header)
        self.assertIn(b"#define ZO_efi64_stub_entry 0x0000000000001400\n", header)
        self.assertIn(b"#define ZO_startup_32 0x0000000000001000\n", header)
        self.assertNotIn(b"unrelated_symbol", header)

    def test_hash_tree_matches_root_first_v1_layout(self):
        salt = bytes(range(32))
        data = bytes((index % 251 for index in range(129 * 4096)))
        tree, root = system_verity.build_hash_tree(data, salt)
        self.assertEqual(len(tree), 3 * 4096)
        self.assertEqual(root, hashlib.sha256(salt + tree[:4096]).hexdigest())
        leaf_hash = hashlib.sha256(salt + data[:4096]).digest()
        self.assertEqual(tree[4096:4096 + 32], leaf_hash)
        self.assertEqual(tree[4096 + 127 * 32:4096 + 128 * 32],
                         hashlib.sha256(salt + data[127 * 4096:128 * 4096]).digest())
        self.assertEqual(tree[4096 + 128 * 32:4096 + 129 * 32],
                         hashlib.sha256(salt + data[128 * 4096:129 * 4096]).digest())

    def test_hash_tree_rejects_bad_alignment_or_salt(self):
        with self.assertRaisesRegex(system_verity.VerityError, "4096-byte aligned"):
            system_verity.build_hash_tree(b"short", bytes(32))
        with self.assertRaisesRegex(system_verity.VerityError, "salt"):
            system_verity.build_hash_tree(bytes(4096), bytes(31))

    def test_pair_mutates_only_the_critical_data_block_and_reuses_tree(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "source.raw"
            verified = root / "verified.raw"
            tampered = root / "tampered.raw"
            source.write_bytes(bytes((index % 251 for index in range(8 * 4096))))
            source.chmod(0o444)
            receipt = system_verity.create_image_pair(source, verified, tampered)
            self.assertEqual(receipt["verity"]["num_data_blocks"], 8)
            self.assertEqual(verified.stat().st_mode & 0o777, 0o444)
            self.assertEqual(tampered.stat().st_mode & 0o777, 0o444)
            good = verified.read_bytes()
            bad = tampered.read_bytes()
            self.assertEqual(good[8 * 4096:], bad[8 * 4096:])
            changed = [i for i, (left, right) in enumerate(zip(good[:8 * 4096], bad[:8 * 4096]))
                       if left != right]
            self.assertEqual(changed, [system_verity.TAMPER_OFFSET])
            self.assertEqual(hashlib.sha256(good).hexdigest(), receipt["verified"]["sha256"])

    def test_dm_table_and_cmdline_keep_root_and_table_in_one_quoted_argument(self):
        image = {
            "verity": {"num_data_blocks": 16, "hash_start_block": 16,
                       "root_digest": "a" * 64, "salt": "b" * 64,
                       "data_block_size": 4096, "hash_block_size": 4096},
        }
        command = system_verity.kernel_cmdline(image)
        words = system_verity.parse_cmdline(command)
        table = [word.partition("=")[2] for word in words if word.startswith("dm-mod.create=")]
        self.assertEqual(len(table), 1)
        self.assertEqual(table[0], system_verity.dm_mod_create(image))
        self.assertIn("0 128 verity 1 /dev/vda /dev/vda 4096 4096 16 16 sha256", table[0])
        self.assertIn("dm-mod.waitfor=/dev/vda", words)

    def test_cmdline_parser_rejects_unbalanced_quote_and_escape(self):
        with self.assertRaisesRegex(system_verity.VerityError, "unterminated"):
            system_verity.parse_cmdline('dm-mod.create="broken')
        with self.assertRaisesRegex(system_verity.VerityError, "backslash"):
            system_verity.parse_cmdline(r"x=bad\ value")

    def test_kernel_config_requires_builtin_dm_verity_and_guest_filesystems(self):
        config = b"\n".join(
            f"{name}=y".encode()
            for name in (
                "CONFIG_X86_64", "CONFIG_BLK_DEV_INITRD", "CONFIG_BINFMT_ELF", "CONFIG_PRINTK",
                "CONFIG_SERIAL_8250", "CONFIG_SERIAL_8250_CONSOLE", "CONFIG_BLOCK", "CONFIG_PCI",
                "CONFIG_VIRTIO", "CONFIG_VIRTIO_PCI", "CONFIG_VIRTIO_BLK", "CONFIG_BLK_DEV_DM",
                "CONFIG_DM_INIT", "CONFIG_DM_VERITY", "CONFIG_CRYPTO_SHA256", "CONFIG_EXT4_FS",
                "CONFIG_DEVTMPFS", "CONFIG_PROC_FS", "CONFIG_SYSFS",
            )
        )
        self.assertEqual(system_verity.parse_config(config)["CONFIG_DM_VERITY"], "y")
        with self.assertRaisesRegex(system_verity.VerityError, "CONFIG_DM_INIT=y"):
            system_verity.parse_config(config.replace(b"CONFIG_DM_INIT=y", b"# CONFIG_DM_INIT is not set"))

    def test_result_parser_requires_kernel_verity_corruption_and_no_ready(self):
        root = "a" * 64
        clean = f"NAGI_SYSTEM_VERITY_ACCEPTED root={root} filesystem=ext4 readonly=true fixture=true\n"
        self.assertEqual(system_verity.parse_probe_log(clean, "accepted", root)["expected"], "accepted")
        tampered = (
            "device-mapper: verity: 253:0: vda: data block 0 is corrupted\n"
            "NAGI_SYSTEM_VERITY_REJECTED stage=critical_block_read errno=5\n"
        )
        self.assertEqual(system_verity.parse_probe_log(tampered, "rejected", root)["expected"], "rejected")
        with self.assertRaisesRegex(system_verity.VerityError, "corruption report"):
            system_verity.parse_probe_log(
                "NAGI_SYSTEM_VERITY_REJECTED stage=critical_block_read errno=5\n", "rejected", root
            )
        with self.assertRaisesRegex(system_verity.VerityError, "READY"):
            system_verity.parse_probe_log(tampered + "nagi-init: state=ready\n", "rejected", root)

    def test_newc_initramfs_binds_compiled_verity_identity(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            elf = root / "init"
            # Minimal valid ELF identity bytes for this host-tool serialization test.
            elf.write_bytes(
                b"\x7fELF\x02\x01" + bytes(12) + b"\x3e\0" + bytes(64)
                + b"a" * 64 + b"1" + b"b" * 64
            )
            output = root / "initramfs.cpio"
            image = {"verity": {"root_digest": "a" * 64, "num_data_blocks": 1,
                                 "salt": "b" * 64}}
            info = system_verity.make_initramfs(elf, output, image)
            self.assertEqual(info["size_bytes"], output.stat().st_size)
            archive = output.read_bytes()
            self.assertIn(b"run/nagi/dev-system\0", archive)
            self.assertIn(b"a" * 64, archive)
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o444)

    def test_prepare_images_keeps_read_only_private_source(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "source.raw"
            source.write_bytes(bytes((index % 239 for index in range(16 * 4096))))
            source.chmod(0o444)
            output = root / "image-pair"
            receipt = system_verity.prepare_images(source, output)
            private_source = output / receipt["source"]["path"]
            self.assertEqual(private_source.read_bytes(), source.read_bytes())
            self.assertEqual(stat.S_IMODE(private_source.stat().st_mode), 0o444)
            self.assertEqual(receipt["source"]["sha256"], hashlib.sha256(source.read_bytes()).hexdigest())

    def test_run_bundle_recomputes_paired_images_and_binds_artifact_hashes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "source.raw"
            source.write_bytes(bytes((index % 239 for index in range(16 * 4096))))
            source.chmod(0o444)
            bundle_dir = root / "bundle"
            system_verity.prepare_images(source, bundle_dir)
            image = json.loads((bundle_dir / "image.json").read_text())
            kernel = root / "kernel"
            kernel.write_bytes(b"fixed kernel")
            config = root / "kernel.config"
            config.write_bytes(self.valid_config())
            initramfs = root / "initramfs.cpio"
            initramfs.write_bytes(
                b"CPIO" + image["verity"]["root_digest"].encode()
                + str(image["verity"]["num_data_blocks"]).encode()
                + image["verity"]["salt"].encode()
            )
            system_verity.create_run_bundle(bundle_dir, kernel, config, initramfs)
            loaded, paths = system_verity.load_run_bundle(bundle_dir)
            self.assertEqual(loaded["image"]["verity"]["root_digest"], image["verity"]["root_digest"])
            self.assertEqual(paths["source"].name, "system.source.raw")

            bad = paths["tampered"]
            bad.chmod(0o644)
            with bad.open("r+b") as stream:
                stream.seek(4096)
                byte = stream.read(1)
                stream.seek(4096)
                stream.write(bytes([byte[0] ^ 1]))
            with self.assertRaisesRegex(system_verity.VerityError, "SHA-256"):
                system_verity.load_run_bundle(bundle_dir)

    def test_runner_does_not_create_qemu_run_without_board_assignment(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "source.raw"
            source.write_bytes(bytes(16 * 4096))
            source.chmod(0o444)
            bundle_dir = root / "bundle"
            system_verity.prepare_images(source, bundle_dir)
            image = json.loads((bundle_dir / "image.json").read_text())
            kernel = root / "kernel"
            kernel.write_bytes(b"kernel")
            config = root / "kernel.config"
            config.write_bytes(self.valid_config())
            initramfs = root / "initramfs.cpio"
            initramfs.write_bytes(
                b"CPIO" + image["verity"]["root_digest"].encode()
                + str(image["verity"]["num_data_blocks"]).encode()
                + image["verity"]["salt"].encode()
            )
            system_verity.create_run_bundle(bundle_dir, kernel, config, initramfs)
            board = root / "board.md"
            board.write_text("枠1空き。枠2はA予約。\n", encoding="utf-8")
            with self.assertRaisesRegex(system_verity.VerityError, "no current L11-3 assignment"):
                system_verity.run_pair(bundle_dir, board, 1, "no-qemu-executable", 1)
            self.assertFalse((bundle_dir / "qemu-run").exists())

    def test_board_gate_skips_summary_and_reads_current_assignment_event(self):
        summary = "QEMU 枠1：統合予約｜枠2：System RO L11-3（clean/tampered 2起動）"
        assignment = "01:23 JST Dot QEMU割当：System RO L11-3へ枠2を割当"
        self.assertEqual(
            system_verity.current_board_assignment([summary, assignment], 2),
            assignment,
        )

    def test_board_gate_rejects_free_or_returned_slot_with_stale_assignment(self):
        old_assignment = "01:23 JST Dot QEMU割当：System RO L11-3へ枠2を割当"
        with self.assertRaisesRegex(system_verity.VerityError, "no current L11-3 assignment"):
            system_verity.current_board_assignment(
                ["QEMU 枠1：統合予約｜枠2：空き", old_assignment], 2
            )

        with self.assertRaisesRegex(system_verity.VerityError, "already returned or free"):
            system_verity.current_board_assignment(
                [
                    "QEMU 枠1：統合予約｜枠2：System RO L11-3",
                    "01:30 JST Dot QEMU枠2返却：L11-3終了、空き",
                    old_assignment,
                ],
                2,
            )


if __name__ == "__main__":
    unittest.main()
