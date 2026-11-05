"""Check the workflow's artifact paths against M27's real evidence layout."""

from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
UPLOAD = WORKFLOW.split("      - name: Upload target acceptance reports and logs\n", 1)[1]
UPLOAD = UPLOAD.split("\n  nagi-m20-inference:", 1)[0]


def artifact_patterns():
    block = UPLOAD.split("          path: |\n", 1)[1]
    patterns = []
    for line in block.splitlines():
        if not line.startswith("            "):
            break
        patterns.append(line.strip())
    return patterns


class M27ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def write(self, relative, contents=b"fixture\n"):
        target = self.root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(contents)
        return relative

    def selected(self):
        return {
            str(path.relative_to(self.root))
            for pattern in artifact_patterns()
            for path in self.root.glob(pattern)
            if path.is_file()
        }

    def test_failed_promotion_and_preceding_boot_logs_are_retained(self):
        evidence = "out/evidence/m27-ab-rollback-1793854380240094000"
        required = {
            self.write(f"{evidence}/readiness-promotion/{boot}.log")
            for boot in ("trial-boot", "promotion-boot", "confirmed-boot")
        }
        required.add(self.write(f"{evidence}/rollback-boot.log"))
        required.add(self.write(f"{evidence}/gpt/healthy-system-b-readiness.log"))
        required.add(self.write("out/logs/m17-runner-resources.log"))
        self.assertTrue(required <= self.selected(), required - self.selected())
        self.assertIn("        if: always()", UPLOAD)

    def test_retry_serial_and_reason_survive_without_uploading_vars_or_disks(self):
        promotion = "out/evidence/m27-ab-rollback-run/readiness-promotion/promotion-boot.log"
        required = {
            self.write(promotion),
            self.write(promotion + ".pre-guest-timeout-1"),
            self.write(promotion + ".pre-guest-retry-1.txt"),
        }
        forbidden = {
            self.write(promotion + ".ovmf-vars.pre-guest-timeout-1", b"\x00vars"),
            self.write(promotion + ".ovmf-vars.pre-guest-retry-source-1", b"\x00vars"),
            self.write("out/evidence/m27-ab-rollback-run/readiness-promotion/OVMF_VARS.fd"),
            self.write("out/evidence/m27-ab-rollback-run/system.raw"),
            self.write("out/evidence/m27-ab-rollback-run/user.qcow2"),
            self.write("out/evidence/unrelated-run/private.log"),
        }
        self.assertEqual(self.selected(), required)
        self.assertTrue(forbidden.isdisjoint(self.selected()))

    def test_absent_m27_evidence_preserves_existing_target_diagnostics(self):
        existing = self.write("out/logs/m17-bootstrap.log")
        self.assertEqual(self.selected(), {existing})


if __name__ == "__main__":
    unittest.main()
