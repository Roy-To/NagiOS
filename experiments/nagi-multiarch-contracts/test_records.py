import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

VERIFY_PATH = Path(__file__).resolve().parents[2] / "docs/porting/multiarch/verify-records.py"
SPEC = importlib.util.spec_from_file_location("verify_records", VERIFY_PATH)
verify_records = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verify_records)

RecordsError = verify_records.RecordsError
HISTORICAL_BASE = verify_records.HISTORICAL_BASE


def run(root, *args, check=True):
    return subprocess.run(["git", *args], cwd=root, check=check, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)


def digest(data):
    return hashlib.sha256(data).hexdigest()


class RecordBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="pr57-records-", dir=os.environ.get("TMPDIR"))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        run(self.root, "init", "-q")
        run(self.root, "config", "user.name", "fixture")
        run(self.root, "config", "user.email", "fixture@example.invalid")
        (self.root / "README.md").write_text("base\n")
        run(self.root, "add", "README.md")
        run(self.root, "commit", "-qm", "main base")
        self.main = run(self.root, "rev-parse", "HEAD").stdout.strip()
        source = self.root / "experiments/nagi-multiarch-contracts/audit.py"
        source.parent.mkdir(parents=True)
        source.write_text("pass\n")
        run(self.root, "add", "experiments/nagi-multiarch-contracts/audit.py")
        run(self.root, "commit", "-qm", "source candidate")
        self.source = run(self.root, "rev-parse", "HEAD").stdout.strip()
        (self.root / "docs/porting/multiarch/state.json").parent.mkdir(parents=True)
        (self.root / "docs/porting/multiarch/state.json").write_text("{}\n")
        run(self.root, "add", "docs/porting/multiarch/state.json")
        run(self.root, "commit", "-qm", "records only")
        self.records = run(self.root, "rev-parse", "HEAD").stdout.strip()

    def test_records_only_child_is_accepted(self):
        self.assertEqual(verify_records.validate_record_commit(self.root, self.source, self.records), ["docs/porting/multiarch/state.json"])
        self.assertTrue(verify_records.validate_owned_scope(self.root, self.main, self.records))

    def test_source_change_in_records_child_is_rejected(self):
        path = self.root / "experiments/nagi-multiarch-contracts/audit.py"
        path.write_text("changed\n")
        run(self.root, "add", "experiments/nagi-multiarch-contracts/audit.py")
        run(self.root, "commit", "-qm", "unexpected source change")
        head = run(self.root, "rev-parse", "HEAD").stdout.strip()
        with self.assertRaisesRegex(RecordsError, "records-only child"):
            verify_records.validate_record_commit(self.root, self.source, head)

    def test_non_ancestor_source_is_rejected(self):
        run(self.root, "checkout", "--orphan", "unrelated")
        (self.root / "elsewhere.txt").write_text("unrelated\n")
        run(self.root, "add", "elsewhere.txt")
        run(self.root, "commit", "-qm", "unrelated")
        head = run(self.root, "rev-parse", "HEAD").stdout.strip()
        with self.assertRaisesRegex(RecordsError, "ancestor"):
            verify_records.validate_record_commit(self.root, self.source, head)

    def test_owned_scope_rejects_nonowned_path(self):
        path = self.root / "kernel/src/main.rs"
        path.parent.mkdir(parents=True)
        path.write_text("kernel change\n")
        run(self.root, "add", "kernel/src/main.rs")
        run(self.root, "commit", "-qm", "outside scope")
        head = run(self.root, "rev-parse", "HEAD").stdout.strip()
        with self.assertRaisesRegex(RecordsError, "outside"):
            verify_records.validate_owned_scope(self.root, self.main, head)

    def test_historical_base_is_never_current(self):
        with self.assertRaisesRegex(RecordsError, "historical"):
            verify_records.validate_current_main(self.root, HISTORICAL_BASE, self.main)

    def test_inventory_base_must_equal_explicit_main(self):
        with self.assertRaisesRegex(RecordsError, "equal"):
            verify_records.validate_current_main(self.root, self.source, self.main)

    def test_tracked_dirty_is_rejected(self):
        (self.root / "README.md").write_text("dirty\n")
        with self.assertRaisesRegex(RecordsError, "dirty"):
            verify_records.validate_clean_record_worktree(self.root)

    def test_guard_untracked_and_other_untracked_are_separate(self):
        (self.root / ".codex").mkdir()
        (self.root / ".codex/config").write_text("local\n")
        result = verify_records.validate_clean_record_worktree(self.root)
        self.assertTrue(result["tracked_clean"])
        self.assertEqual(result["local_guard_untracked_roots"], [".codex"])
        self.assertEqual(result["local_guard_untracked_count"], 1)
        (self.root / "outside.txt").write_text("unexpected\n")
        with self.assertRaisesRegex(RecordsError, "unexpected untracked"):
            verify_records.validate_clean_record_worktree(self.root)


class VerifyEndToEndTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="pr57-verify-e2e-", dir=os.environ.get("TMPDIR"))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        run(self.root, "init", "-q")
        run(self.root, "config", "user.name", "fixture")
        run(self.root, "config", "user.email", "fixture@example.invalid")
        self.create_base()
        run(self.root, "add", "-A")
        run(self.root, "commit", "-qm", "old main")
        self.old_main = run(self.root, "rev-parse", "HEAD").stdout.strip()
        (self.root / "README.md").write_text("current main\n")
        run(self.root, "add", "README.md")
        run(self.root, "commit", "-qm", "current main")
        self.main = run(self.root, "rev-parse", "HEAD").stdout.strip()
        self.source = self.main
        self.tree = self.tree_sha(self.source)
        self.write_records()
        self.records = run(self.root, "rev-parse", "HEAD").stdout.strip()

    def create_base(self):
        (self.root / "README.md").write_text("old main\n")
        scripts = [
            "experiments/nagi-multiarch-contracts/audit.py",
            "experiments/nagi-multiarch-contracts/test_audit.py",
            "experiments/nagi-multiarch-contracts/test_records.py",
            "experiments/nagi-aarch64-spike/run.py",
            "experiments/nagi-aarch64-spike/attest.py",
            "experiments/nagi-aarch64-spike/verify_attestation.py",
            "experiments/nagi-aarch64-spike/test_runner.py",
            "experiments/nagi-aarch64-spike/test_attestation.py",
            "docs/porting/multiarch/verify-records.py",
        ]
        for name in scripts:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("pass\n")
        for name in verify_records.AUTHORITY_PATHS:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("authority source\n")
        lib = self.root / "crates/fixture/src/lib.rs"
        lib.parent.mkdir(parents=True, exist_ok=True)
        lib.write_text("pub fn fixture() {}\n")
        registry = self.root / ".dev/workstreams.json"
        registry.parent.mkdir(parents=True, exist_ok=True)
        registry.write_text(json.dumps({"workstreams": []}) + "\n")
        schema = self.root / ".dev/schemas/workstream-state.schema.json"
        schema.parent.mkdir(parents=True)
        schema.write_text(json.dumps({"type": "object"}) + "\n")
        doc = self.root / "docs/porting/multiarch"
        doc.mkdir(parents=True, exist_ok=True)
        (doc / "state.json").write_text(json.dumps({"last_verified": {"commit_sha": "0" * 40}}) + "\n")
        (doc / "registration-proposal.json").write_text(json.dumps({"registration_status": "PROPOSED_UNREGISTERED"}) + "\n")
        (doc / "validation-evidence.json").write_text("{}\n")
        evidence = doc / "evidence"
        evidence.mkdir(parents=True)
        (evidence / "legacy.txt").write_text("legacy evidence\n")
        (evidence / "HISTORICAL.json").write_text("{}\n")

    def tree_sha(self, commit):
        return run(self.root, "rev-parse", f"{commit}^{{tree}}").stdout.strip()

    def write_json(self, path, value):
        path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")

    def file_hash(self, path):
        return digest(path.read_bytes())

    def build_index_entries(self, evidence):
        entries = []
        for sha, state in sorted(((self.old_main, "SUPERSEDED"), (self.main, "CURRENT"))):
            directory = evidence / ("main-" + sha)
            files = []
            names = ["source-inventory.json", "authority-sources.json", "baseline-drift.json", "current-status.json"]
            status = json.loads((directory / "current-status.json").read_text())
            validation = status.get("records_validation", {})
            if isinstance(validation, dict) and validation.get("stdout_path"):
                names.append("records-validation.stdout.log")
            for name in names:
                files.append({"path": f"main-{sha}/{name}", "sha256": self.file_hash(directory / name)})
            entries.append({"main_sha": sha, "status": state, "files": files})
        return entries

    def write_records(self):
        doc = self.root / "docs/porting/multiarch"
        evidence = doc / "evidence"
        old_tree, main_tree = self.tree_sha(self.old_main), self.tree_sha(self.main)
        names = run(self.root, "ls-tree", "-r", "--name-only", self.source).stdout.splitlines()
        source_paths = sorted(name for name in names if name.startswith(verify_records.SOURCE_PREFIXES) and name.endswith(verify_records.SOURCE_SUFFIXES))
        inventory = {
            "schema": "nagi-multiarch-source-inventory/1", "baseline_sha": self.main,
            "source_commit_sha": self.source, "source_tree_sha": self.tree,
            "files": [{"path": name, "sha256": digest(run(self.root, "show", f"{self.source}:{name}").stdout.encode())} for name in source_paths],
        }
        current = evidence / ("main-" + self.main)
        current.mkdir()
        self.write_json(current / "source-inventory.json", inventory)
        authorities = {name: digest(run(self.root, "show", f"{self.source}:{name}").stdout.encode()) for name in verify_records.AUTHORITY_PATHS}
        self.write_json(current / "authority-sources.json", authorities)
        changed = run(self.root, "diff", "--name-status", self.old_main, self.main).stdout.splitlines()
        self.write_json(current / "baseline-drift.json", {
            "schema": "nagi-pr57-main-drift/1", "old_main_sha": self.old_main, "current_main_sha": self.main,
            "old_main_tree_sha": old_tree, "current_main_tree_sha": main_tree,
            "source_commit_sha": self.source, "source_tree_sha": self.tree,
            "pr_head_sha": self.main, "candidate_merge_sha": self.main,
            "historical_common_ancestor_sha": run(self.root, "merge-base", self.old_main, self.main).stdout.strip(),
            "main_changed_paths": changed, "main_changed_path_count": len(changed),
            "main_changes_in_owned_prefixes": [], "owned_prefixes_unchanged_from_old_main": True,
            "pr_to_merge_changes_in_owned_prefixes": [], "pr_contract_and_spike_tree_preserved_by_main_merge": True,
            "remote_publish_or_PR_mutation": "NOT_RUN",
        })
        (doc / "state.json").write_text(json.dumps({"last_verified": {"commit_sha": self.source}}) + "\n")
        self.write_json(doc / "evidence/HISTORICAL.json", {
            "schema": "port-multiarch-historical-evidence/1", "current_validity": "NONE",
            "base_sha": HISTORICAL_BASE,
            "files": [
                {"path": "docs/porting/multiarch/evidence/legacy.txt", "sha256": self.file_hash(evidence / "legacy.txt")},
                {"path": "docs/porting/multiarch/validation-evidence.json", "sha256": self.file_hash(doc / "validation-evidence.json")},
            ],
        })
        status = {
            "schema": "nagi-multiarch-current-status/1", "main_sha": self.main, "main_tree_sha": main_tree,
            "source_commit_sha": self.source, "source_tree_sha": self.tree,
            "phases": {
                "A": {"status": "PENDING", "checks": {"current_static_inventory": "PASS", "current_authority_hashes": "PASS", "registry_unregistered_and_record_validation": "PENDING", "ownership_and_live_ref_snapshot": "NOT_RUN"}, "reason": "Fixture; ownership is not recorded."},
                "B": {"status": "PENDING", "checks": {"model_free_tool_acceptance": "PASS", "host_Cargo_contract_tests": "NOT_RUN"}, "reason": "Fixture; Cargo tests are unrun."},
                "C": {"status": "NOT_RUN", "checks": {"new_ELF": "NOT_RUN", "QEMU_EL1": "NOT_RUN", "QEMU_EL2_negative": "NOT_RUN", "independent_rebuild": "NOT_RUN"}, "reason": "Fixture; no ELF or QEMU run occurred."},
            },
            "tool_acceptance": {
                "model_free_python_tests": {"outcome": "PASS", "tests": 1, "contracts_audit_and_records": 1, "runner_and_receipt": 0},
                "python_syntax": {"outcome": "PASS", "files": 9},
                "shell_syntax": {"outcome": "PASS", "scripts": 2},
                "cargo_fmt": {"outcome": "PASS", "packages": 2, "build_performed": False},
                "static_inventory": {"outcome": "PASS", "source_files": len(inventory["files"]), "matches": 0, "bootinfo_field_references": 0, "authority_files": len(authorities)},
                "static_inventory_exact_check": {"outcome": "PASS", "source_commit_explicit": True},
            },
            "records_validation": {
                "outcome": "PENDING", "record_head_sha": None, "record_tree_sha": None,
                "source_commit_sha": self.source,
                "command": f"python3 docs/porting/multiarch/verify-records.py --main {self.main} --evidence-dir docs/porting/multiarch/evidence/main-{self.main}",
                "exit_code": None, "stdout_path": None, "stdout_sha256": None,
            },
            "execution": {"cargo_check": "NOT_RUN", "cargo_clippy": "NOT_RUN", "cargo_build": "NOT_RUN", "new_ELF_generation": "NOT_RUN", "QEMU": "NOT_RUN", "VM": "NOT_RUN", "Miri": "NOT_RUN", "independent_rebuild": "NOT_RUN"},
            "artifact_hashes": {},
        }
        self.write_json(current / "current-status.json", status)
        old = evidence / ("main-" + self.old_main)
        old.mkdir()
        self.write_json(old / "source-inventory.json", {"baseline_sha": self.old_main})
        self.write_json(old / "authority-sources.json", {})
        self.write_json(old / "baseline-drift.json", {})
        self.write_json(old / "current-status.json", {"main_sha": self.old_main})
        status["artifact_hashes"] = {
            "source_inventory_sha256": self.file_hash(current / "source-inventory.json"),
            "authority_sources_sha256": self.file_hash(current / "authority-sources.json"),
            "baseline_drift_sha256": self.file_hash(current / "baseline-drift.json"),
        }
        self.write_json(current / "current-status.json", status)
        self.write_json(evidence / "main-records-index.json", {
            "schema": "nagi-multiarch-main-records-index/1", "current_main_sha": self.main,
            "entries": self.build_index_entries(evidence),
        })
        run(self.root, "add", "-A")
        run(self.root, "commit", "-qm", "records-only evidence")

    def commit_change(self, message):
        run(self.root, "add", "-A")
        run(self.root, "commit", "-qm", message)

    def refresh_status_index(self):
        evidence = self.root / "docs/porting/multiarch/evidence"
        current = evidence / ("main-" + self.main)
        status_path = current / "current-status.json"
        status = json.loads(status_path.read_text())
        inventory = json.loads((current / "source-inventory.json").read_text())
        status["tool_acceptance"]["static_inventory"]["source_files"] = len(inventory.get("files", []))
        status["artifact_hashes"] = {
            "source_inventory_sha256": self.file_hash(current / "source-inventory.json"),
            "authority_sources_sha256": self.file_hash(current / "authority-sources.json"),
            "baseline_drift_sha256": self.file_hash(current / "baseline-drift.json"),
        }
        self.write_json(status_path, status)
        index_path = evidence / "main-records-index.json"
        index = json.loads(index_path.read_text())
        index["entries"] = self.build_index_entries(evidence)
        self.write_json(index_path, index)

    def verify(self):
        return verify_records.verify(self.root, self.main, self.root / "docs/porting/multiarch/evidence" / ("main-" + self.main))

    def test_verify_accepts_complete_current_and_superseded_records(self):
        result = self.verify()
        self.assertEqual(result["main"], self.main)
        self.assertEqual(result["source"], self.source)
        self.assertEqual(result["historical_files"], 2)

    def test_verify_rejects_added_or_removed_inventory_entries(self):
        path = self.root / "docs/porting/multiarch/evidence" / ("main-" + self.main) / "source-inventory.json"
        for mode in ("add", "remove"):
            with self.subTest(mode=mode):
                record = json.loads(path.read_text())
                if mode == "add":
                    record["files"].append({"path": "crates/missing/src/lib.rs", "sha256": "0" * 64})
                else:
                    record["files"].pop()
                self.write_json(path, record)
                self.refresh_status_index()
                self.commit_change("change inventory entries")
                with self.assertRaisesRegex(RecordsError, "inventory file set"):
                    self.verify()
                self.tearDown()
                self.setUp()
                path = self.root / "docs/porting/multiarch/evidence" / ("main-" + self.main) / "source-inventory.json"

    def test_verify_rejects_source_change_in_records_only_child(self):
        (self.root / "crates/fixture/src/lib.rs").write_text("pub fn changed() {}\n")
        self.commit_change("source changed after records source")
        with self.assertRaisesRegex(RecordsError, "records-only child changed non-record paths"):
            self.verify()

    def test_verify_rejects_authority_mismatch(self):
        path = self.root / "docs/porting/multiarch/evidence" / ("main-" + self.main) / "authority-sources.json"
        values = json.loads(path.read_text())
        values[verify_records.AUTHORITY_PATHS[0]] = "0" * 64
        self.write_json(path, values)
        self.refresh_status_index()
        self.commit_change("change authority digest")
        with self.assertRaisesRegex(RecordsError, "authority hash mismatch"):
            self.verify()

    def test_verify_rejects_historical_file_tampering(self):
        (self.root / "docs/porting/multiarch/evidence/legacy.txt").write_text("tampered\n")
        self.commit_change("tamper historical file")
        with self.assertRaisesRegex(RecordsError, "historical evidence changed"):
            self.verify()

    def test_verify_rejects_invalid_phase_status(self):
        path = self.root / "docs/porting/multiarch/evidence" / ("main-" + self.main) / "current-status.json"
        status = json.loads(path.read_text())
        status["phases"]["C"]["status"] = "PASS"
        self.write_json(path, status)
        self.refresh_status_index()
        self.commit_change("invalid phase")
        with self.assertRaisesRegex(RecordsError, "strict pending-state contract"):
            self.verify()

    def test_verify_rejects_baseline_drift_count_tampering(self):
        path = self.root / "docs/porting/multiarch/evidence" / ("main-" + self.main) / "baseline-drift.json"
        record = json.loads(path.read_text())
        record["main_changed_path_count"] += 1
        self.write_json(path, record)
        self.refresh_status_index()
        self.commit_change("invalid drift count")
        with self.assertRaisesRegex(RecordsError, "path list/count"):
            self.verify()

    def test_verify_rejects_records_validation_commit_tampering(self):
        path = self.root / "docs/porting/multiarch/evidence" / ("main-" + self.main) / "current-status.json"
        status = json.loads(path.read_text())
        status["records_validation"].update({
            "outcome": "PASS", "record_head_sha": "0" * 40, "record_tree_sha": self.tree,
            "stdout_sha256": "a" * 64,
        })
        status["phases"]["A"]["checks"]["registry_unregistered_and_record_validation"] = "PASS"
        self.write_json(path, status)
        self.refresh_status_index()
        self.commit_change("invalid records validation")
        with self.assertRaisesRegex(RecordsError, "does not resolve"):
            self.verify()

    def test_verify_rejects_pending_validation_with_result_hash(self):
        path = self.root / "docs/porting/multiarch/evidence" / ("main-" + self.main) / "current-status.json"
        status = json.loads(path.read_text())
        status["records_validation"]["stdout_sha256"] = "a" * 64
        self.write_json(path, status)
        self.refresh_status_index()
        self.commit_change("pending validation has fabricated result")
        with self.assertRaisesRegex(RecordsError, "must not claim"):
            self.verify()

    def test_verify_rejects_unknown_validation_outcome(self):
        path = self.root / "docs/porting/multiarch/evidence" / ("main-" + self.main) / "current-status.json"
        status = json.loads(path.read_text())
        status["records_validation"]["outcome"] = "UNKNOWN"
        status["phases"]["A"]["checks"]["registry_unregistered_and_record_validation"] = "PASS"
        self.write_json(path, status)
        self.refresh_status_index()
        self.commit_change("unknown validation outcome")
        with self.assertRaisesRegex(RecordsError, "outcome must be PENDING or PASS"):
            self.verify()

    def test_verify_accepts_pending_to_pass_records_only_transition(self):
        initial = self.verify()
        self.assertEqual(initial["source"], self.source)
        path = self.root / "docs/porting/multiarch/evidence" / ("main-" + self.main) / "current-status.json"
        status = json.loads(path.read_text())
        record_tree = self.tree_sha(self.records)
        stdout_path = path.parent / "records-validation.stdout.log"
        stdout = (
            f"PASS records-only child; main={self.main} source={self.source}; "
            f"record_head={self.records} record_tree={record_tree}; fixture output\n"
        ).encode()
        stdout_path.write_bytes(stdout)
        status["records_validation"].update({
            "outcome": "PASS", "record_head_sha": self.records, "record_tree_sha": record_tree,
            "exit_code": 0,
            "stdout_path": f"docs/porting/multiarch/evidence/main-{self.main}/records-validation.stdout.log",
            "stdout_sha256": digest(stdout),
        })
        status["phases"]["A"]["checks"]["registry_unregistered_and_record_validation"] = "PASS"
        self.write_json(path, status)
        self.refresh_status_index()
        self.commit_change("record successful verification")
        result = self.verify()
        self.assertEqual(result["main"], self.main)
        (path.parent / "records-validation.stdout.log").write_text("forged verifier result\\n")
        self.refresh_status_index()
        self.commit_change("tamper saved records-validation output")
        with self.assertRaisesRegex(RecordsError, "stdout hash does not match"):
            self.verify()

    def test_verify_rejects_source_commit_as_records_validation_head(self):
        path = self.root / "docs/porting/multiarch/evidence" / ("main-" + self.main) / "current-status.json"
        status = json.loads(path.read_text())
        stdout_path = path.parent / "records-validation.stdout.log"
        stdout = (
            f"PASS records-only child; main={self.main} source={self.source}; "
            f"record_head={self.source} record_tree={self.tree}; fixture output\\n"
        ).encode()
        stdout_path.write_bytes(stdout)
        status["records_validation"].update({
            "outcome": "PASS", "record_head_sha": self.source, "record_tree_sha": self.tree,
            "exit_code": 0,
            "stdout_path": f"docs/porting/multiarch/evidence/main-{self.main}/records-validation.stdout.log",
            "stdout_sha256": digest(stdout),
        })
        status["phases"]["A"]["checks"]["registry_unregistered_and_record_validation"] = "PASS"
        self.write_json(path, status)
        self.refresh_status_index()
        self.commit_change("invalid source-as-record head")
        with self.assertRaisesRegex(RecordsError, "strict records-only child"):
            self.verify()

    def test_verify_rejects_nonzero_records_validation_exit_code(self):
        self.verify()
        path = self.root / "docs/porting/multiarch/evidence" / ("main-" + self.main) / "current-status.json"
        status = json.loads(path.read_text())
        record_tree = self.tree_sha(self.records)
        stdout = (
            f"PASS records-only child; main={self.main} source={self.source}; "
            f"record_head={self.records} record_tree={record_tree}; fixture output\n"
        ).encode()
        (path.parent / "records-validation.stdout.log").write_bytes(stdout)
        status["records_validation"].update({
            "outcome": "PASS", "record_head_sha": self.records, "record_tree_sha": record_tree,
            "exit_code": 1,
            "stdout_path": f"docs/porting/multiarch/evidence/main-{self.main}/records-validation.stdout.log",
            "stdout_sha256": digest(stdout),
        })
        status["phases"]["A"]["checks"]["registry_unregistered_and_record_validation"] = "PASS"
        self.write_json(path, status)
        self.refresh_status_index()
        self.commit_change("records validation has nonzero exit code")
        with self.assertRaisesRegex(RecordsError, "successful zero exit code"):
            self.verify()

    def test_verify_rejects_unindexed_superseded_main_file(self):
        path = self.root / "docs/porting/multiarch/evidence" / ("main-" + self.old_main) / "extra.json"
        path.write_text("{}\n")
        self.commit_change("add unindexed historical main file")
        with self.assertRaisesRegex(RecordsError, "file set is incomplete or unexpected"):
            self.verify()


if __name__ == "__main__":
    unittest.main()
