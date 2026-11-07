"""Launcher routing fixtures: fake Cargo never compiles a dependency."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class BootstrapRouting(unittest.TestCase):
    def test_posix_routing_arguments_and_exit_codes(self):
        commands = [[], ["--help"], ["doctor", "--unexpected"], ["fetch"],
                    ["dev", "fingerprint"], ["legal", "check"], ["build"],
                    ["test", "--acceptance", "--host-only"], ["lint"],
                    ["image"], ["m17"], ["unknown-command"],
                    ["build", "space value", "", "'quoted'", "$literal", "`literal`"]]
        with tempfile.TemporaryDirectory(prefix="launcher-routing-") as temporary:
            temp = Path(temporary)
            repo = temp / "repo with spaces"
            repo.mkdir()
            for name in ["nagi", "rust-toolchain.toml"]:
                shutil.copyfile(ROOT / name, repo / name)
            tools = temp / "bin"
            tools.mkdir()
            cargo = tools / "cargo"
            cargo.write_text("#!/usr/bin/python3\nimport json,os,sys\n"
                             "open(os.environ['ROUTING_LOG'],'w').write(json.dumps(sys.argv[1:]))\n"
                             "sys.exit(int(os.environ['ROUTING_EXIT']))\n")
            cargo.chmod(0o700)
            rustup = tools / "rustup"
            rustup.write_text("#!/bin/sh\nexit 99\n")
            rustup.chmod(0o700)
            log = temp / "args.json"
            for code in [0, 2, 4, 17, 101]:
                for args in commands:
                    with self.subTest(code=code, args=args):
                        env = dict(os.environ, PATH=str(tools) + ":/usr/bin:/bin",
                                   ROUTING_LOG=str(log), ROUTING_EXIT=str(code))
                        result = subprocess.run(["/bin/sh", str(repo / "nagi"), *args],
                                                env=env, cwd=temp, capture_output=True, timeout=5)
                        self.assertEqual(result.returncode, code, result.stderr)
                        self.assertEqual(json.loads(log.read_text()),
                                         ["run", "--locked", "--manifest-path",
                                          str(repo / "tools/nagi-bootstrap/Cargo.toml"),
                                          "-p", "nagi-bootstrap", "--", *args])

    def test_powershell_static_routing_and_exit_contract(self):
        source = (ROOT / "nagi.ps1").read_text()
        self.assertNotIn("$bootstrapMode", source)
        self.assertIn("$manifestPath = Join-Path $repositoryRoot 'tools\\nagi-bootstrap\\Cargo.toml'", source)
        self.assertIn("$packageName = 'nagi-bootstrap'", source)
        self.assertNotIn("else { 'nagi-cli' }", source)
        self.assertIn("-- @Arguments", source)
        self.assertIn("$childExitCode = $LASTEXITCODE", source)
        self.assertIn("exit $childExitCode", source)

    def test_bootstrap_remains_independent_and_reuses_cli(self):
        bootstrap = (ROOT / "tools/nagi-bootstrap/Cargo.toml").read_text()
        self.assertTrue(bootstrap.startswith("[workspace]"))
        self.assertNotIn("[patch.crates-io]", bootstrap)
        self.assertIn('path = "../nagi-cli/src/main.rs"', bootstrap)
        self.assertIn('path = "../nagi-cli/src/lib.rs"', bootstrap)


if __name__ == "__main__":
    unittest.main()
