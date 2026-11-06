#!/usr/bin/env python3
"""Packaging rejection boundaries; no runtime evidence is produced."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("packaging", HERE / "verify-self-contained-records.py")
packaging = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(packaging)
objects = packaging.objects


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.current = {"source.rs": b"100644 blob source", "old-record.json": b"100644 blob record"}
        self.prior = {p: b"100644 blob historical" for p in packaging.SIDE_FILES}
        self.actual = {**self.current, **self.prior,
                       **{p: b"100644 blob packaging" for p in packaging.PACKAGING}}
        self.manifest = json.loads((HERE / "review-dependencies.json").read_text())

    def test_exact_overlay(self):
        packaging.check_tree(self.actual, self.current, self.prior)

    def test_preserved_files_cannot_change(self):
        for path in self.current.keys() | self.prior.keys():
            with self.subTest(path=path):
                changed = dict(self.actual, **{path: b"100644 blob changed"})
                with self.assertRaises(ValueError):
                    packaging.check_tree(changed, self.current, self.prior)

    def test_preserved_files_cannot_disappear(self):
        for path in self.current.keys() | self.prior.keys():
            with self.subTest(path=path):
                changed = dict(self.actual)
                del changed[path]
                with self.assertRaises(ValueError):
                    packaging.check_tree(changed, self.current, self.prior)

    def test_extra_path_rejected(self):
        self.actual["unexpected.txt"] = b"100644 blob extra"
        with self.assertRaises(ValueError):
            packaging.check_tree(self.actual, self.current, self.prior)

    def test_packaging_mode_and_missing_rejected(self):
        for path in packaging.PACKAGING:
            for replacement in (None, b"120000 blob link", b"100755 blob executable"):
                with self.subTest(path=path, replacement=replacement):
                    changed = dict(self.actual)
                    if replacement is None:
                        del changed[path]
                    else:
                        changed[path] = replacement
                    with self.assertRaises(ValueError):
                        packaging.check_tree(changed, self.current, self.prior)

    def test_overlapping_inputs_rejected(self):
        with self.assertRaises(ValueError):
            packaging.check_tree(self.actual, {**self.current, **self.prior}, self.prior)

    def test_real_manifest_and_bundle(self):
        objects.validate_manifest(self.manifest)
        objects.validate_bundle((HERE / "review-dependencies.bundle").read_bytes())

    def test_bundle_truncation_and_same_size_corruption(self):
        data = (HERE / "review-dependencies.bundle").read_bytes()
        for bad in (data[:-1], bytes([data[0] ^ 1]) + data[1:]):
            with self.assertRaises(ValueError):
                objects.validate_bundle(bad)

    def test_manifest_identity_and_anchor_changes(self):
        for key in ("schema", "bundle", "bytes", "sha256", *objects.ANCHORS):
            with self.subTest(key=key):
                changed = copy.deepcopy(self.manifest)
                changed[key] = "changed"
                with self.assertRaises(ValueError):
                    objects.validate_manifest(changed)

    def test_manifest_unknown_field_rejected(self):
        self.manifest["allow_missing"] = True
        with self.assertRaises(ValueError):
            objects.validate_manifest(self.manifest)


if __name__ == "__main__":
    unittest.main()
