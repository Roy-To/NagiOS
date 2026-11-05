#!/usr/bin/env python3
"""Supplemental direct-QEMU check; read a raw receipt, write a new directory only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

EXPECTED = {
    'el1': b'SPIKE ARM64 EL1 PL011 PASS\nSPIKE SVC VECTOR ERET PASS\nSPIKE GENERIC COUNTER PASS\nSPIKE COMPLETE PASS\n',
    'el2': b'SPIKE FAIL exception-level\n',
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check(raw_run, output, qemu_root):
    # Refuse existing directories, files and dangling symlinks before any execution.
    if os.path.lexists(output):
        raise ValueError('output already exists; choose a new directory')
    raw_run = raw_run.resolve(strict=True)
    output = output.absolute()
    if output.resolve().is_relative_to(raw_run):
        raise ValueError('output must be outside the raw receipt directory')
    qemu_root = qemu_root.resolve(strict=True)
    backend = qemu_root / 'usr/bin/qemu-system-aarch64'
    receipt = raw_run / 'attestation.json'
    attestation = json.loads(receipt.read_text())
    elf = raw_run / 'build-a/nagi-aarch64-feasibility-spike'
    elf_hash = sha(elf)
    for label in ('a', 'b'):
        built = raw_run / f'build-{label}/nagi-aarch64-feasibility-spike'
        entries = [b for b in attestation['builds'] if b['id'] == label]
        if len(entries) != 1 or sha(built) != entries[0]['elf_sha256'] or sha(built) != elf_hash:
            raise ValueError('raw build ELF binding mismatch')
    expected = {}
    for label in ('el1', 'el2'):
        directory = raw_run / 'runs' / label
        result = json.loads((directory / 'result.json').read_text())
        serial = (directory / 'serial.log').read_bytes()
        if (result['binary_sha256'] != elf_hash or result['binary_sha256_after'] != elf_hash
                or result['serial_sha256'] != hashlib.sha256(serial).hexdigest()
                or serial != EXPECTED[label]
                or result.get('returncode') != (0 if label == 'el1' else 1)
                or result.get('timed_out') is not False):
            raise ValueError('raw run ELF/serial binding mismatch')
        expected[label] = serial
    backend_before = sha(backend)
    # Atomic creation is the ownership boundary. A race cannot reuse old output.
    output.mkdir()
    record = {'raw_run': str(raw_run), 'raw_attestation_sha256': sha(receipt),
              'backend_path': str(backend), 'backend_sha256_before': backend_before,
              'elf_sha256': elf_hash, 'runs': []}
    env = {'PATH': '/usr/bin:/bin',
           'LD_LIBRARY_PATH': str(qemu_root / 'usr/lib/x86_64-linux-gnu'),
           'QEMU_MODULE_DIR': str(qemu_root / 'usr/lib/x86_64-linux-gnu/qemu')}
    for label, mode in (('el1', 'off'), ('el2', 'on')):
        command = [str(backend), '-L', str(qemu_root / 'usr/share/qemu'),
                   '-machine', f'virt,gic-version=2,virtualization={mode}',
                   '-cpu', 'cortex-a53', '-accel', 'tcg', '-smp', '1', '-m', '128M',
                   '-display', 'none', '-monitor', 'none', '-serial', 'stdio',
                   '-no-reboot', '-nic', 'none', '-semihosting-config',
                   'enable=on,target=native', '-kernel', str(elf)]
        timed_out, error = False, None
        try:
            result = subprocess.run(command, env=env, capture_output=True, timeout=15)
            serial, stderr, code = result.stdout, result.stderr, result.returncode
        except subprocess.TimeoutExpired as exc:
            serial, stderr, code = exc.stdout or b'', exc.stderr or b'', None
            timed_out = True
        except OSError as exc:
            serial, stderr, code = b'', b'', None
            error = str(exc)
        # Exclusive file creation also prevents an accidental overwrite within this run.
        for kind, data in (('serial', serial), ('stderr', stderr)):
            with (output / f'backend-{label}.{kind}.log').open('xb') as stream:
                stream.write(data)
        passed = (not timed_out and error is None and code == (0 if label == 'el1' else 1)
                  and serial == expected[label] and stderr == b'')
        record['runs'].append({'label': label, 'command': command, 'returncode': code,
                              'timed_out': timed_out, 'error': error,
                              'serial_matches_primary': serial == expected[label],
                              'serial_sha256': hashlib.sha256(serial).hexdigest(),
                              'acceptance': 'PASS' if passed else 'FAIL'})
    record['backend_sha256_after'] = sha(backend)
    record['elf_sha256_after'] = sha(elf)
    record['raw_attestation_sha256_after'] = sha(receipt)
    passed = (all(r['acceptance'] == 'PASS' for r in record['runs'])
              and backend_before == record['backend_sha256_after']
              and elf_hash == record['elf_sha256_after']
              and record['raw_attestation_sha256'] == record['raw_attestation_sha256_after'])
    record['outcome'] = 'PASS' if passed else 'FAIL'
    with (output / 'backend-check.json').open('x') as stream:
        json.dump(record, stream, indent=2)
        stream.write('\n')
    return record


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--raw-run', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--qemu-root', type=Path, required=True,
                        help='extracted package root containing usr/bin/qemu-system-aarch64')
    args = parser.parse_args(argv)
    try:
        record = check(args.raw_run, args.output, args.qemu_root)
    except (OSError, ValueError, KeyError, TypeError) as exc:
        parser.exit(2, f'backend check rejected: {exc}\n')
    print(json.dumps(record, indent=2))
    return 0 if record['outcome'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
