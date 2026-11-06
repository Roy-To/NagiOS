"""Model-free CLI and output-preservation regressions; not guest acceptance."""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name('check_backend.py')
SPEC = importlib.util.spec_from_file_location('backend_check', SCRIPT)
m = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(m)


class BackendTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.raw = self.root / 'elsewhere/raw'
        self.output = self.root / 'new-output'
        self.qemu = self.root / 'qemu'
        self.backend = self.qemu / 'usr/bin/qemu-system-aarch64'
        self.backend.parent.mkdir(parents=True)
        self.marker = self.root / 'executed'
        program = ('#!/usr/bin/python3\nimport pathlib,sys\n'
                   f'pathlib.Path({str(self.marker)!r}).touch()\n'
                   "el2=any('virtualization=on' in x for x in sys.argv)\n"
                   f'sys.stdout.buffer.write({m.EXPECTED["el2"]!r} if el2 else {m.EXPECTED["el1"]!r})\n'
                   'sys.exit(1 if el2 else 0)\n')
        self.backend.write_text(program)
        self.backend.chmod(0o755)
        elf_hash = hashlib.sha256(b'fixture ELF').hexdigest()
        builds = []
        for label in ('a', 'b'):
            elf = self.raw / f'build-{label}/nagi-aarch64-feasibility-spike'
            elf.parent.mkdir(parents=True)
            elf.write_bytes(b'fixture ELF')
            builds.append({'id': label, 'elf_sha256': elf_hash})
        (self.raw / 'attestation.json').write_text(json.dumps({'builds': builds}))
        for label, serial in m.EXPECTED.items():
            d = self.raw / 'runs' / label
            d.mkdir(parents=True)
            (d / 'serial.log').write_bytes(serial)
            (d / 'result.json').write_text(json.dumps({
                'binary_sha256': elf_hash, 'binary_sha256_after': elf_hash,
                'serial_sha256': hashlib.sha256(serial).hexdigest(),
                'returncode': 0 if label == 'el1' else 1, 'timed_out': False}))

    def cli(self, script=SCRIPT):
        return subprocess.run([sys.executable, '-B', str(script), '--raw-run', str(self.raw),
                               '--output', str(self.output), '--qemu-root', str(self.qemu)],
                              capture_output=True)

    def test_relocated_script_uses_explicit_raw_directory(self):
        relocated = self.root / 'relocated.py'
        relocated.write_bytes(SCRIPT.read_bytes())
        result = self.cli(relocated)
        self.assertEqual(result.returncode, 0, result.stderr)
        record = json.loads((self.output / 'backend-check.json').read_text())
        self.assertEqual(record['raw_run'], str(self.raw))
        self.assertEqual([r['returncode'] for r in record['runs']], [0, 1])
        self.assertEqual(record['outcome'], 'PASS')
        self.assertFalse((self.root / 'run-01').exists())

    def test_existing_directory_is_unchanged_and_qemu_not_executed(self):
        self.output.mkdir()
        (self.output / 'backend-el1.serial.log').write_bytes(b'original evidence')
        result = self.cli()
        self.assertEqual(result.returncode, 2)
        self.assertEqual((self.output / 'backend-el1.serial.log').read_bytes(), b'original evidence')
        self.assertFalse(self.marker.exists())

    def test_existing_file_is_unchanged(self):
        self.output.write_bytes(b'original')
        self.assertEqual(self.cli().returncode, 2)
        self.assertEqual(self.output.read_bytes(), b'original')
        self.assertFalse(self.marker.exists())

    def test_dangling_symlink_is_rejected(self):
        self.output.symlink_to(self.root / 'missing')
        self.assertEqual(self.cli().returncode, 2)
        self.assertTrue(self.output.is_symlink())
        self.assertFalse((self.root / 'missing').exists())
        self.assertFalse(self.marker.exists())

    def test_output_inside_raw_is_rejected(self):
        self.output = self.raw / 'new-output'
        self.assertEqual(self.cli().returncode, 2)
        self.assertFalse(self.output.exists())
        self.assertFalse(self.marker.exists())

    def test_raw_elf_mismatch_rejected_before_output_creation(self):
        (self.raw / 'build-b/nagi-aarch64-feasibility-spike').write_bytes(b'changed')
        self.assertEqual(self.cli().returncode, 2)
        self.assertFalse(self.output.exists())
        self.assertFalse(self.marker.exists())

    def test_missing_cli_paths_are_rejected(self):
        result = subprocess.run([sys.executable, '-B', str(SCRIPT)], capture_output=True)
        self.assertEqual(result.returncode, 2)
        self.assertFalse(self.marker.exists())

    def test_raw_run_with_wrong_exit_is_rejected(self):
        p = self.raw / 'runs/el2/result.json'
        result = json.loads(p.read_text())
        result['returncode'] = 0
        p.write_text(json.dumps(result))
        self.assertEqual(self.cli().returncode, 2)
        self.assertFalse(self.output.exists())
        self.assertFalse(self.marker.exists())

    def test_second_invocation_preserves_first_output(self):
        self.assertEqual(self.cli().returncode, 0)
        original = {p.name: p.read_bytes() for p in self.output.iterdir()}
        self.marker.unlink()
        self.assertEqual(self.cli().returncode, 2)
        self.assertEqual(original, {p.name: p.read_bytes() for p in self.output.iterdir()})
        self.assertFalse(self.marker.exists())

    def test_timeout_preserves_failure_evidence(self):
        with patch.object(m.subprocess, 'run', side_effect=subprocess.TimeoutExpired('qemu', 15, output=b'partial')):
            result = m.check(self.raw, self.output, self.qemu)
        self.assertEqual(result['outcome'], 'FAIL')
        self.assertTrue(all(r['timed_out'] for r in result['runs']))
        self.assertEqual((self.output / 'backend-el1.serial.log').read_bytes(), b'partial')
        self.assertEqual(json.loads((self.output / 'backend-check.json').read_text())['outcome'], 'FAIL')


if __name__ == '__main__':
    unittest.main()
