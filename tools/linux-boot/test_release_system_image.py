#!/usr/bin/env python3
"""Host checks for the release System raw and unsigned metadata pair."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from release_system_image import (
    DATA_BLOCK_SIZE,
    IMAGE_NAME,
    METADATA_NAME,
    MIN_DATA_BLOCKS,
    VerityError,
    create_image,
    describe_image,
    encode_metadata,
    verify_image,
)
from system_verity import build_hash_tree


class ReleaseSystemImageTests(unittest.TestCase):
    def test_matches_reference_tree_and_rejects_corruption(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / IMAGE_NAME
            blocks = MIN_DATA_BLOCKS + 1
            data_bytes = blocks * DATA_BLOCK_SIZE
            with path.open("wb") as stream:
                stream.write(b"System contents")
                stream.truncate(data_bytes)
            salt = bytes(range(32))
            with path.open("r+b") as stream:
                metadata = describe_image(stream, blocks, salt, generate=True)
            image = path.read_bytes()
            tree, digest = build_hash_tree(image[:data_bytes], salt)
            self.assertEqual(image[data_bytes:], tree)
            self.assertEqual(metadata["verity"]["root_digest"], digest)
            self.assertEqual(metadata["verity"]["hash_start_block"], blocks)
            self.assertEqual(metadata["system"]["sha256"], hashlib.sha256(image).hexdigest())
            payload = encode_metadata(metadata)
            verify_image(path, payload)
            invalid = json.loads(payload)
            invalid["verity"]["hash_start_block"] += 1
            with self.assertRaisesRegex(VerityError, "metadata does not match"):
                verify_image(path, encode_metadata(invalid))

            for offset in (42, data_bytes, data_bytes + len(tree) - 1):
                with self.subTest(offset=offset), path.open("r+b") as stream:
                    original = image[offset : offset + 1]
                    stream.seek(offset)
                    stream.write(bytes([original[0] ^ 1]))
                    stream.flush()
                    with self.assertRaisesRegex(VerityError, "tree mismatch"):
                        verify_image(path, payload)
                    stream.seek(offset)
                    stream.write(original)
            with path.open("ab") as stream:
                stream.write(b"unexpected trailing bytes")
            with self.assertRaisesRegex(VerityError, "raw length"):
                verify_image(path, payload)

    @unittest.skipUnless(shutil.which("mke2fs"), "mke2fs is required")
    def test_creates_ext4_and_verity_above_development_fixture_limit(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            staging = root / "staging"
            staging.mkdir()
            (staging / "example").write_bytes(b"release System test file\n")
            output = root / "result"
            blocks = (129 * 1024 * 1024) // DATA_BLOCK_SIZE
            create_image(staging, output, blocks, "mke2fs")
            image = output / IMAGE_NAME
            metadata = (output / METADATA_NAME).read_bytes()
            verify_image(image, metadata)
            with image.open("rb") as stream:
                stream.seek(1024 + 56)
                self.assertEqual(stream.read(2), b"\x53\xef")
            self.assertFalse(image.stat().st_mode & 0o222)
            self.assertEqual(json.loads(metadata)["verity"]["num_data_blocks"], blocks)
            self.assertEqual(set(os.listdir(output)), {IMAGE_NAME, METADATA_NAME})
            with self.assertRaises(FileExistsError):
                create_image(staging, output, blocks, "mke2fs")

            debugfs = shutil.which("debugfs")
            if debugfs:
                result = subprocess.run(
                    [debugfs, "-R", "cat /example", str(image)], check=True, capture_output=True
                )
                self.assertEqual(result.stdout, (staging / "example").read_bytes())

    def test_cleans_up_after_formatter_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            staging = root / "staging"
            staging.mkdir()
            output = root / "result"
            with self.assertRaises(FileNotFoundError):
                create_image(staging, output, MIN_DATA_BLOCKS, str(root / "missing-mke2fs"))
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
