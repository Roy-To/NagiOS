"""Static contracts, plus optional isolated host-only executed fixtures.

Default mode uses Python only. --runtime uses explicitly supplied installed
Rust/Cargo with private inputs, outputs and copied verified dependency archives.
No real target compiler, wrapper, Mesa archives or fonts are executed or changed.
Run: python3 -B user/nagi-init/tests/build_inputs_host.py [--source BUILD_RS]
"""
from pathlib import Path
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
import time
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser()
parser.add_argument('--source', type=Path, default=ROOT / 'user/nagi-init/build.rs')
parser.add_argument('--runtime', action='store_true')
parser.add_argument('--rustc', type=Path)
parser.add_argument('--cargo', type=Path)
parser.add_argument('--cache-root', type=Path)
parser.add_argument('--evidence', type=Path)
args, unittest_args = parser.parse_known_args()
SOURCE = args.source.read_text(encoding='utf-8')
WRAPPER = (ROOT / 'tools/nagi-target-cc.sh').read_text(encoding='utf-8')


class BuildInputContracts(unittest.TestCase):
    def test_wrapper_environment_inputs_are_declared_to_cargo(self):
        inputs = set(re.findall(r'\$\{(NAGI_[A-Z0-9_]+)', WRAPPER))
        declared = set(re.findall(r'cargo:rerun-if-env-changed=([A-Z_a-z0-9]+)', SOURCE))
        self.assertEqual(inputs - declared, set())

    def test_both_mesa_archives_have_explicit_file_tracking(self):
        mesa = SOURCE[SOURCE.index('let mesa_build ='):SOURCE.index('let app_directory =')]
        for name in ['libnagi_mesa.a', 'libnagi_mesa_roots.a']:
            self.assertTrue(name in mesa, 'untracked Mesa archive: ' + name)
        self.assertRegex(mesa, r'cargo:rerun-if-changed=\{\}.*?mesa_archive\.display\(\)')
        self.assertRegex(mesa, r'(?s)cargo:rerun-if-changed=\{\}[^;]+libnagi_mesa_roots\.a')

    def test_wrapper_and_header_directories_are_tracked_before_native_work(self):
        host_guard = SOURCE.index('CARGO_CFG_TARGET_OS')
        self.assertTrue('track_target_cc_inputs();' in SOURCE, 'wrapper input tracker absent')
        call = SOURCE.index('track_target_cc_inputs();', host_guard)
        first_wrapper_call = SOURCE.index('.arg(&target_cc_wrapper)')
        self.assertLess(call, first_wrapper_call)
        helper = SOURCE[SOURCE.index('fn track_target_cc_inputs()'):]
        for entry in ['tools/nagi-target-cc.sh', 'tools/mesa/nagi-headers',
                      'NAGI_RELIBC_HEADERS', 'NAGI_CXX_HEADERS', 'relibc-target', 'include']:
            with self.subTest(input=entry):
                self.assertIn(entry, helper)
        self.assertIn('cargo:rerun-if-changed={}', helper)

    def test_selected_compiler_and_search_environment_are_tracked(self):
        self.assertTrue('track_configured_compiler_input(&compiler);' in SOURCE,
                        'configured compiler path tracker absent')
        self.assertIn('cargo:rerun-if-env-changed=PATH', SOURCE)
        self.assertIn('cargo:rerun-if-env-changed=ProgramFiles', SOURCE)
        helper = SOURCE[SOURCE.index('fn track_configured_compiler_input('):]
        self.assertIn('cargo:rerun-if-changed={}', helper)
        self.assertNotIn('Command::new', helper[:helper.index('/// Generate')])


class TinyFontOracleFixtures(unittest.TestCase):
    """An independent digest oracle, not execution of the Rust build script."""
    def test_positive_font_manifest_fixture(self):
        with tempfile.TemporaryDirectory(prefix='nagi-font-oracle-') as tmp:
            font = Path(tmp) / 'Fixture.ttf'
            font.write_bytes(b'abc')
            pin = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
            manifest = Path(tmp) / 'manifest.tsv'
            manifest.write_text('/system/fonts/Fixture.ttf\tFixture.ttf\t3\t' + pin + '\n')
            fields = manifest.read_text().strip().split('\t')
            self.assertEqual(len(font.read_bytes()), int(fields[2]))
            self.assertEqual(hashlib.sha256(font.read_bytes()).hexdigest(), fields[3])

    def test_same_size_single_byte_corruption_fails_digest_oracle(self):
        with tempfile.TemporaryDirectory(prefix='nagi-font-oracle-') as tmp:
            font = Path(tmp) / 'Fixture.ttf'
            expected = b'abc'
            font.write_bytes(b'abd')
            self.assertEqual(len(font.read_bytes()), len(expected))
            self.assertEqual(sum(a != b for a, b in zip(font.read_bytes(), expected)), 1)
            self.assertNotEqual(hashlib.sha256(font.read_bytes()).digest(),
                                hashlib.sha256(expected).digest())


@unittest.skipUnless(args.runtime, 'host compilation requires --runtime and explicit installed tools')
class ExecutedHostFixtures(unittest.TestCase):
    """Execute the actual build.rs, using only temporary fake native inputs.

    Cargo freshness tests forward its real rerun directives, discard all native
    link directives, and compile an ordinary host observer binary. They prove
    host Cargo output freshness, not actual Mesa/product target relinking.
    """
    @classmethod
    def setUpClass(cls):
        assert args.rustc and args.cargo and args.cache_root
        cls.temporary = tempfile.TemporaryDirectory(prefix='nagi-build-input-host-')
        cls.observations = []
        cls.root = Path(cls.temporary.name)
        cls.home = cls.root / 'cargo-home'
        cls.env = {k: v for k, v in os.environ.items()
                   if not k.startswith(('NAGI_', 'CARGO_FEATURE_', 'CARGO_TARGET_', 'CARGO_ENCODED_'))}
        cls.env.update(CARGO_HOME=str(cls.home), CARGO_NET_OFFLINE='true',
                       RUSTC=str(args.rustc), RUSTDOC=str(args.rustc.with_name('rustdoc')))
        cls.packages = cls.copy_dependencies()
        cls.repo = cls.root / 'fake-repo'
        cls.manifest_dir = cls.repo / 'user/nagi-init'
        cls.manifest_dir.mkdir(parents=True)
        (cls.manifest_dir / 'Cargo.toml').write_text(
            '[workspace]\n[package]\nname="build-rs-host-driver"\nversion="0.1.0"\n'
            'edition="2021"\nbuild=false\n[dependencies]\n'
            'sha2={version="=0.10.9",default-features=false}\n'
            '[[bin]]\nname="driver"\npath=' + json.dumps(str(args.source.resolve())) + '\n')
        cls.write_lock(cls.manifest_dir, 'build-rs-host-driver', ['sha2'])
        driver_target = cls.root / 'driver-target'
        driver_env = dict(cls.env, CARGO_TARGET_DIR=str(driver_target))
        result = subprocess.run([str(args.cargo), 'build', '--locked', '--offline',
                                 '--manifest-path', str(cls.manifest_dir / 'Cargo.toml')],
                                env=driver_env, capture_output=True, text=True)
        assert result.returncode == 0, result.stderr
        cls.driver = driver_target / 'debug/driver'
        cls.fonts = cls.root / 'fonts'
        cls.fonts.mkdir()
        cls.compiler = cls.root / 'fake-compiler'
        cls.compiler.write_text('#!/bin/sh\nexit 0\n')
        cls.compiler.chmod(0o700)
        cls.wrapper = cls.repo / 'tools/nagi-target-cc.sh'
        cls.wrapper.parent.mkdir(parents=True)
        cls.wrapper.write_text('#!/bin/sh\nexit 0\n')
        cls.wrapper.chmod(0o700)
        for name in ['tools/mesa/nagi-cxx-runtime.cpp', 'tools/mesa/nagi-libcpp-abi.cpp',
                     'tools/mesa/nagi-libcpp-sort.cpp', 'tests/apps/m13_posix.c',
                     'tests/apps/m13_relibc.c']:
            path = cls.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('temporary fixture; never compiled as native code\n')
        (cls.repo / 'tools/mesa/nagi-headers').mkdir()
        cls.headers = cls.root / 'headers'
        cls.headers.mkdir()
        (cls.headers / 'pthread.h').write_text('header A')
        cls.cxx = cls.root / 'cxx'
        cls.cxx.mkdir()
        (cls.cxx / 'cstddef').write_text('C++ header A')
        cls.mesa = cls.root / 'mesa'
        (cls.mesa / 'mesa-build').mkdir(parents=True)
        (cls.mesa / 'libnagi_mesa.a').write_text('archive A')
        (cls.mesa / 'libnagi_mesa_roots.a').write_text('roots A')

    @classmethod
    def tearDownClass(cls):
        if args.evidence:
            args.evidence.write_text(json.dumps(cls.observations, indent=2) + '\n')
        cls.temporary.cleanup()

    @classmethod
    def copy_dependencies(cls):
        packages = tomllib.loads((ROOT / 'Cargo.lock').read_text())['package']
        selected = {}
        todo = [('sha2', '0.10.9')]
        while todo:
            name, version = todo.pop()
            entry = dict(next(p for p in packages if p['name'] == name and
                              (version is None or p['version'] == version)))
            key = (name, entry['version'])
            if key in selected:
                continue
            selected[key] = entry
            for dependency in entry.get('dependencies', []):
                fields = dependency.split()
                todo.append((fields[0], fields[1] if len(fields) > 1 else None))
        registry = 'index.crates.io-1949cf8c6b5b557f'
        for (name, version), entry in selected.items():
            relative = f'{name[:2]}/{name[2:4]}/{name}'
            index = args.cache_root / 'index' / registry / '.cache' / relative
            if 'checksum' not in entry:
                # The product workspace patches libc; the host-only observer
                # resolves the same version from its already-cached registry archive.
                row = next(json.loads(row) for row in index.read_bytes().split(b'\0')
                           if row.startswith(b'{') and json.loads(row)['vers'] == version)
                entry.update(source='registry+https://github.com/rust-lang/crates.io-index',
                             checksum=row['cksum'])
            archive = args.cache_root / 'cache' / registry / f'{name}-{version}.crate'
            assert hashlib.sha256(archive.read_bytes()).hexdigest() == entry['checksum']
            for original, destination in [
                (archive, cls.home / 'registry/cache' / registry / archive.name),
                (index, cls.home / 'registry/index' / registry / '.cache' / relative),
            ]:
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(original, destination)
        shutil.copyfile(args.cache_root / 'index' / registry / 'config.json',
                        cls.home / 'registry/index' / registry / 'config.json')
        return list(selected.values())

    @classmethod
    def write_lock(cls, directory, name, dependencies):
        entries = [{'name': name, 'version': '0.1.0', 'dependencies': dependencies}]
        entries += cls.packages if dependencies else []
        text = 'version = 4\n'
        for entry in entries:
            text += '\n[[package]]\n'
            for key, value in entry.items():
                text += key + ' = ' + json.dumps(value) + '\n'
        (directory / 'Cargo.lock').write_text(text)

    def setUp(self):
        self.fonts.joinpath('Fixture.ttf').write_bytes(b'abc')
        self.pin = hashlib.sha256(b'abc').hexdigest()
        self.fonts.joinpath('manifest.tsv').write_text(
            f'/system/fonts/Fixture.ttf\tFixture.ttf\t3\t{self.pin}\n')
        lock = self.repo / 'third_party/fonts.lock'
        lock.parent.mkdir(exist_ok=True)
        lock.write_text('format_version = 1\n[fonts.fixture]\nfile_name="Fixture.ttf"\n'
                        'guest_path="/system/fonts/Fixture.ttf"\nsize_bytes=3\n'
                        f'sha256="{self.pin}"\n')
        self.out = Path(tempfile.mkdtemp(prefix='out-', dir=self.root))
        self.run_env = dict(self.env, OUT_DIR=str(self.out), CARGO_FEATURE_M17_SERVO='1',
                            CARGO_CFG_TARGET_OS='linux', CARGO_MANIFEST_DIR=str(self.manifest_dir),
                            NAGI_FONT_DIR=str(self.fonts), NAGI_TARGET_CLANG=str(self.compiler),
                            NAGI_MESA_BUILD=str(self.mesa / 'mesa-build'),
                            NAGI_RELIBC_HEADERS=str(self.headers), NAGI_CXX_HEADERS=str(self.cxx))

    def stage(self):
        result = subprocess.run([str(self.driver)], env=self.run_env, capture_output=True, text=True)
        staged = self.out / 'system-fonts/Fixture.ttf'
        self.observations.append({'test': self.id(), 'action': 'execute actual build.rs host driver',
                                  'rc': result.returncode, 'stderr': result.stderr[:1500],
                                  'staged_sha256': hashlib.sha256(staged.read_bytes()).hexdigest()
                                  if staged.exists() else None})
        return result

    def test_font_positive_executes_actual_build_script(self):
        result = self.stage()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.out / 'system-fonts/Fixture.ttf').read_bytes(), b'abc')

    def test_font_same_size_corruption_is_rejected(self):
        self.fonts.joinpath('Fixture.ttf').write_bytes(b'abd')
        self.assertNotEqual(self.stage().returncode, 0, 'same-size corrupted font accepted')

    def test_font_and_manifest_digest_cannot_override_source_pin(self):
        self.fonts.joinpath('Fixture.ttf').write_bytes(b'abd')
        self.fonts.joinpath('manifest.tsv').write_text(
            '/system/fonts/Fixture.ttf\tFixture.ttf\t3\t' + hashlib.sha256(b'abd').hexdigest() + '\n')
        self.assertNotEqual(self.stage().returncode, 0, 'forged manifest digest accepted')

    def test_font_duplicate_manifest_entry_is_rejected(self):
        manifest = self.fonts / 'manifest.tsv'
        manifest.write_text(manifest.read_text() * 2)
        self.assertNotEqual(self.stage().returncode, 0, 'duplicate manifest entry accepted')

    def test_font_missing_manifest_entry_is_rejected(self):
        self.fonts.joinpath('manifest.tsv').write_text('')
        self.assertNotEqual(self.stage().returncode, 0, 'missing pinned font accepted')

    def test_actual_repository_lock_scalar_format_is_understood(self):
        (self.repo / 'third_party/fonts.lock').write_bytes((ROOT / 'third_party/fonts.lock').read_bytes())
        self.fonts.joinpath('manifest.tsv').write_text('')
        result = self.stage()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('font manifest omits pinned files', result.stderr)

    def cargo_freshness(self, mutation, environment_change=None):
        consumer = Path(tempfile.mkdtemp(prefix='consumer-', dir=self.root))
        (consumer / 'src').mkdir()
        (consumer / 'Cargo.toml').write_text(
            '[workspace]\n[package]\nname="input-observer"\nversion="0.1.0"\nedition="2021"\n')
        (consumer / 'src/main.rs').write_text(
            'fn main(){print!("{}",include_str!(concat!(env!("OUT_DIR"),"/observed.txt")));}\n')
        # Execute the real script but never pass native link directives into the
        # ordinary host observer. Only its real Cargo rerun contracts are forwarded.
        bridge = '''use std::{env,fs,process::Command};
fn main(){
let r=Command::new(env::var("FIXTURE_DRIVER").unwrap())
 .env("CARGO_CFG_TARGET_OS","nagi")
 .env("CARGO_MANIFEST_DIR",env::var("FIXTURE_MANIFEST").unwrap()).output().unwrap();
assert!(r.status.success(),"{}",String::from_utf8_lossy(&r.stderr));
for line in String::from_utf8(r.stdout).unwrap().lines(){
 if line.starts_with("cargo:rerun-if-"){println!("{}",line);}
}
let mut value=String::new();
for file in ["FIXTURE_ARCHIVE","FIXTURE_ROOTS","FIXTURE_WRAPPER","FIXTURE_HEADER","FIXTURE_CXX_HEADER","FIXTURE_COMPILER","FIXTURE_LOCK"]{
 value.push_str(&fs::read_to_string(env::var(file).unwrap()).unwrap());
}
for key in ["NAGI_RELIBC_HEADERS","NAGI_CXX_HEADERS","NAGI_TARGET","NAGI_TARGET_LD"]{
 value.push_str(&env::var(key).unwrap_or_default());
}
fs::write(std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("observed.txt"),value).unwrap();
}
'''
        (consumer / 'build.rs').write_text(bridge)
        self.write_lock(consumer, 'input-observer', [])
        env = dict(self.run_env, CARGO_TARGET_DIR=str(consumer / 'target'),
                   FIXTURE_DRIVER=str(self.driver), FIXTURE_MANIFEST=str(self.manifest_dir),
                   FIXTURE_ARCHIVE=str(self.mesa / 'libnagi_mesa.a'),
                   FIXTURE_ROOTS=str(self.mesa / 'libnagi_mesa_roots.a'),
                   FIXTURE_WRAPPER=str(self.wrapper), FIXTURE_HEADER=str(self.headers / 'pthread.h'),
                   FIXTURE_CXX_HEADER=str(self.cxx / 'cstddef'),
                   FIXTURE_LOCK=str(self.repo / 'third_party/fonts.lock'),
                   FIXTURE_COMPILER=str(self.compiler))
        command = [str(args.cargo), 'run', '--quiet', '--locked', '--offline',
                   '--manifest-path', str(consumer / 'Cargo.toml')]
        def run():
            result = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout
        before = run()
        self.assertEqual(run(), before, 'unchanged fixture should stay stable')
        time.sleep(0.05)
        mutation()
        if environment_change:
            env.update(environment_change)
        after = run()
        self.observations.append({'test': self.id(), 'action': 'ordinary Cargo host observer',
                                  'before_sha256': hashlib.sha256(before.encode()).hexdigest(),
                                  'after_sha256': hashlib.sha256(after.encode()).hexdigest(),
                                  'refreshed': before != after, 'rc': 0})
        self.assertNotEqual(after, before, 'Cargo reused stale host observer output')

    def test_cargo_mesa_archive_refresh(self):
        archive = self.mesa / 'libnagi_mesa.a'
        self.cargo_freshness(lambda: archive.write_text(archive.read_text() + ' changed'))

    def test_cargo_mesa_roots_archive_refresh(self):
        archive = self.mesa / 'libnagi_mesa_roots.a'
        self.cargo_freshness(lambda: archive.write_text(archive.read_text() + ' changed'))

    def test_cargo_wrapper_refresh(self):
        self.cargo_freshness(lambda: self.wrapper.write_text(self.wrapper.read_text() + '# changed\n'))

    def test_cargo_header_refresh(self):
        header = self.headers / 'pthread.h'
        self.cargo_freshness(lambda: header.write_text(header.read_text() + ' changed'))

    def test_cargo_cxx_header_refresh(self):
        header = self.cxx / 'cstddef'
        self.cargo_freshness(lambda: header.write_text(header.read_text() + ' changed'))

    def test_cargo_font_source_lock_refresh(self):
        lock = self.repo / 'third_party/fonts.lock'
        self.cargo_freshness(lambda: lock.write_text(lock.read_text() + '\n# pin metadata change\n'))

    def test_cargo_configured_compiler_refresh(self):
        self.cargo_freshness(lambda: self.compiler.write_text(self.compiler.read_text() + '# changed\n'))

    def test_cargo_target_environment_refresh(self):
        self.cargo_freshness(lambda: None, {'NAGI_TARGET': 'fixture-other-target'})

    def test_cargo_linker_environment_refresh(self):
        self.cargo_freshness(lambda: None, {'NAGI_TARGET_LD': 'fixture-other-linker'})

    def test_cargo_relibc_headers_environment_refresh(self):
        self.cargo_freshness(lambda: None, {'NAGI_RELIBC_HEADERS': str(self.cxx)})


if __name__ == '__main__':
    unittest.main(argv=[__file__, *unittest_args])
