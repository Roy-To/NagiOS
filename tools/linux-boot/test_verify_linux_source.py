import hashlib
import io
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import verify_linux_source
from verify_linux_source import SourceVerificationError, verify_archive_matches_directory


class VerifyLinuxSourceTests(unittest.TestCase):
    def make_pair(self, root: Path, archive_mtime: int = 0) -> tuple[Path, Path]:
        source = root / "source"
        (source / "scripts").mkdir(parents=True)
        program = b"#!/bin/sh\nexit 0\n"
        (source / "scripts" / "build.sh").write_bytes(program)
        (source / "scripts" / "build.sh").chmod(0o755)
        (source / "README").write_bytes(b"pinned source\n")
        (source / "link").symlink_to("README")

        archive_path = root / "linux-6.18.63.tar.xz"
        with tarfile.open(archive_path, "w:xz") as archive:
            directory = tarfile.TarInfo("linux-6.18.63/")
            directory.type = tarfile.DIRTYPE
            directory.mode = 0o755
            directory.mtime = archive_mtime
            archive.addfile(directory)
            scripts = tarfile.TarInfo("linux-6.18.63/scripts/")
            scripts.type = tarfile.DIRTYPE
            scripts.mode = 0o755
            scripts.mtime = archive_mtime
            archive.addfile(scripts)
            for name, data, mode in (
                ("linux-6.18.63/README", b"pinned source\n", 0o644),
                ("linux-6.18.63/scripts/build.sh", program, 0o755),
            ):
                member = tarfile.TarInfo(name)
                member.size = len(data)
                member.mode = mode
                member.mtime = archive_mtime
                archive.addfile(member, io.BytesIO(data))
            link = tarfile.TarInfo("linux-6.18.63/link")
            link.type = tarfile.SYMTYPE
            link.linkname = "README"
            link.mtime = archive_mtime
            archive.addfile(link)
        return source, archive_path

    def test_pinned_archive_accepts_matching_extracted_tree(self):
        with tempfile.TemporaryDirectory() as temp:
            source, archive = self.make_pair(Path(temp))
            expected = hashlib.sha256(archive.read_bytes()).hexdigest()
            receipt = verify_archive_matches_directory(source, archive, expected)
            self.assertEqual(receipt["archive_sha256"], expected)
            self.assertEqual(len(receipt["tree_sha256"]), 64)

    def test_pinned_archive_rejects_changed_and_extra_source_files(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source, archive = self.make_pair(root)
            expected = hashlib.sha256(archive.read_bytes()).hexdigest()
            (source / "README").write_bytes(b"changed\n")
            with self.assertRaisesRegex(SourceVerificationError, "differs from pinned archive"):
                verify_archive_matches_directory(source, archive, expected)
            (source / "README").write_bytes(b"pinned source\n")
            (source / "untracked.c").write_text("extra", encoding="utf-8")
            with self.assertRaisesRegex(SourceVerificationError, "differs from pinned archive"):
                verify_archive_matches_directory(source, archive, expected)

    def test_pinned_archive_hash_is_checked_before_tree_use(self):
        with tempfile.TemporaryDirectory() as temp:
            source, archive = self.make_pair(Path(temp))
            with self.assertRaisesRegex(SourceVerificationError, "archive SHA-256"):
                verify_archive_matches_directory(source, archive, "0" * 64)

    def test_archive_path_replacement_cannot_swap_bytes_between_hash_and_parse(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source, archive = self.make_pair(root / "pinned")
            _, replacement = self.make_pair(root / "replacement", archive_mtime=1)
            expected = hashlib.sha256(archive.read_bytes()).hexdigest()
            replacement_bytes = replacement.read_bytes()
            replacement_sha256 = hashlib.sha256(replacement_bytes).hexdigest()
            self.assertNotEqual(expected, replacement_sha256)
            pinned_tree = verify_archive_matches_directory(source, archive, expected)["tree_sha256"]
            replacement_tree = verify_archive_matches_directory(
                source, replacement, replacement_sha256
            )["tree_sha256"]
            self.assertEqual(pinned_tree, replacement_tree)
            original_archive_tree = verify_linux_source._archive_tree

            def replace_path_before_parse(reader):
                replacement.replace(archive)
                return original_archive_tree(reader)

            with mock.patch.object(
                verify_linux_source, "_archive_tree", side_effect=replace_path_before_parse
            ):
                with self.assertRaisesRegex(SourceVerificationError, "changed while verifying"):
                    verify_archive_matches_directory(source, archive, expected)

            self.assertEqual(archive.read_bytes(), replacement_bytes)


if __name__ == "__main__":
    unittest.main()
