"""Synthetic fixtures test validator orchestration only, never guest acceptance."""
import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import release
import session_acceptance as sa


REVISION = "a" * 40
OBJECT = "object:kept"
HISTORY = "history:page"
URL = "https://example.org/page"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def operation_data():
    data = {
        "files.created": {"object_id": OBJECT, "content_sha256": "b" * 64},
        "files.renamed": {"object_id": OBJECT, "content_sha256": "b" * 64,
                          "old_name": "before.txt", "new_name": "after.txt"},
        "files.searched": {"object_id": OBJECT, "content_sha256": "b" * 64,
                           "query": "after.txt", "found": True},
        "files.deleted": {"object_id": "object:deleted", "absent": True},
        "ai.user_request": {"request_id": "request:1", "user_initiated": True, "local": True},
        "servo.user_navigation": {"engine": "Servo", "user_initiated": True,
                                  "rendered": True, "url": URL},
        "servo.history_recorded": {"history_id": HISTORY, "url": URL, "persisted": True},
    }
    for name in ("ai.summary", "ai.plan"):
        data[name] = {"request_id": "request:1", "provider": "llama.cpp",
                      "model": "IBM Granite 4.2 3B", "model_sha256": release.GRANITE_SHA256,
                      "guest": True, "local": True, "generated_tokens": 32,
                      "inference_ms": 100, "output_sha256": "c" * 64}
    data["ai.plan"].update(plan_id="plan:1", structured=True, object_id=OBJECT)
    for name in ("plan.validated", "plan.policy_allowed", "plan.user_confirmed",
                 "plan.executed", "plan.undone"):
        data[name] = {"request_id": "request:1", "plan_id": "plan:1", "object_id": OBJECT}
    data["plan.validated"]["valid"] = True
    data["plan.policy_allowed"]["allowed"] = True
    data["plan.user_confirmed"].update(user_initiated=True, confirmed=True)
    data["plan.executed"].update(transaction_id="transaction:1", changed=True,
                                  before_sha256="d" * 64, after_sha256="e" * 64)
    data["plan.undone"].update(transaction_id="transaction:1", restored=True, state_sha256="d" * 64)
    return data


class SessionAcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.image = self.root / "ordinary.qcow2"
        # Explicitly synthetic bytes: only orchestration/integrity under test.
        self.image.write_bytes(b"orchestration image")
        self.image_hash = digest(self.image.read_bytes())
        self.info = self.image.with_name(self.image.name + ".build-info")
        self.info.write_text(
            f"format_version=2\nsource_revision={REVISION}\nimage_sha256={self.image_hash}\n"
            "profile=production-session-v1\ninit_features=production-session\n"
            "components=login,files,search,servo,local-ai,history,voice\n"
            f"model_store_sha256={release.GRANITE_SHA256}\n")
        self.manifest = {"schema_version": 1, "source_revision": REVISION,
                         "image_sha256": self.image_hash, "profile": sa.PROFILE,
                         "session_id": "session:1", "phases": {}}
        self.events = {}
        previous = self.image_hash
        for index, (phase, sequence) in enumerate(sa.EVENT_SEQUENCES.items()):
            data = operation_data()
            data.update({"session.profile": {"profile": sa.PROFILE},
                         "session.start": {"ordinary": True, "network": phase != "offline"},
                         "login.unlocked": {"user": "owner", "unlocked": True},
                         "login.ready": {"files": True, "search": True, "local_ai": True, "servo": True},
                         "files.restored": {"object_id": OBJECT, "content_sha256": "b" * 64, "found": True},
                         "servo.history_restored": {"history_id": HISTORY, "url": URL, "found": True},
                         "offline.completed": {"network": False, "files": True, "search": True,
                                               "local_ai": True, "servo_history": True,
                                               "voice": True, "wayback": True},
                         "stress.measured": {"duration_ms": 60000, "desktop": True, "files": True,
                                             "notes": True, "granite_loaded": True, "audio_playing": True,
                                             "semantic_search": True, "servo_tabs": 4, "desktop_samples": 100,
                                             "audio_frames": 48000, "limits": dict(sa.STRESS_LIMITS),
                                             "memory_bytes_start": 1024**3,
                                             "memory_bytes_end": 1024**3 + sa.STRESS_LIMITS["memory_growth_bytes"],
                                             "handles_start": 100, "handles_end": 132,
                                             **sa.STRESS_LIMITS}})
            self.events[phase] = [{"event": event, "session_id": "session:1", "phase": phase,
                                   "disk_id": "disk:1", "image_sha256": self.image_hash,
                                   "data": data[event]} for event in sequence]
            end = digest(f"disk after {index}".encode())
            self.manifest["phases"][phase] = {
                "path": phase + ".log", "sha256": "0" * 64, "disk_id": "disk:1",
                "initial_image_sha256": self.image_hash, "disk_start_sha256": previous,
                "disk_end_sha256": end, "machine": dict(sa.MACHINE), "network": phase != "offline"}
            previous = end
        self.evidence = self.root / "evidence.json"
        self.write()

    def write(self):
        for phase, events in self.events.items():
            content = "serial startup\n" + "".join(sa.EVENT_PREFIX + json.dumps(e) + "\n" for e in events)
            (self.root / (phase + ".log")).write_text(content)
            if phase in self.manifest["phases"]:
                self.manifest["phases"][phase]["sha256"] = digest(content.encode())
        self.evidence.write_text(json.dumps(self.manifest))

    def validate(self):
        sa.validate_session(self.root, self.image, self.evidence, REVISION)

    def reject(self):
        with self.assertRaises(release.ReleaseError):
            self.validate()

    def event(self, name, phase="boot"):
        return next(e for e in self.events[phase] if e["event"] == name)

    def test_complete_synthetic_orchestration(self):
        self.validate()

    def test_bootstrap_diagnostics_are_allowed_only_before_session(self):
        path = self.root / "boot.log"
        content = "Nagi M3 acceptance PASS\nNagi M4 acceptance PASS\nNagi M7 acceptance PASS\n" + path.read_text()
        path.write_text(content)
        self.manifest["phases"]["boot"]["sha256"] = digest(content.encode())
        self.evidence.write_text(json.dumps(self.manifest))
        self.validate()
        content += "Nagi M7 acceptance PASS\n"
        path.write_text(content)
        self.manifest["phases"]["boot"]["sha256"] = digest(content.encode())
        self.evidence.write_text(json.dumps(self.manifest))
        self.reject()

    def test_utf8_names_and_user_filename_words_are_not_fixture_markers(self):
        renamed = self.event("files.renamed")["data"]
        renamed["old_name"] = "日本語.txt"
        renamed["new_name"] = "fixture-report.txt"
        self.event("files.searched")["data"]["query"] = renamed["new_name"]
        self.write()
        self.validate()
        renamed["new_name"] = "../escape"
        self.write()
        self.reject()

    def test_mixed_image_bindings(self):
        for target in ("manifest", "phase", "event", "image"):
            with self.subTest(target=target):
                self.setUp()
                if target == "manifest":
                    self.manifest["image_sha256"] = "f" * 64
                elif target == "phase":
                    self.manifest["phases"]["restart"]["initial_image_sha256"] = "f" * 64
                elif target == "event":
                    self.event("login.ready", "offline")["image_sha256"] = "f" * 64
                else:
                    self.image.write_bytes(b"different image")
                self.write()
                self.reject()

    def test_log_tamper(self):
        with (self.root / "boot.log").open("a") as stream:
            stream.write("extra bytes\n")
        self.reject()

    def test_absent_phases(self):
        for phase in sa.EVENT_SEQUENCES:
            with self.subTest(phase=phase):
                original = self.manifest["phases"].pop(phase)
                self.write()
                self.reject()
                self.manifest["phases"][phase] = original

    def test_sign_in_order_and_missing(self):
        for phase in sa.EVENT_SEQUENCES:
            for mutation in ("reorder", "missing"):
                with self.subTest(phase=phase, mutation=mutation):
                    original = copy.deepcopy(self.events[phase])
                    if mutation == "reorder":
                        self.events[phase][2:4] = reversed(self.events[phase][2:4])
                    else:
                        self.events[phase].pop(2)
                    self.write()
                    self.reject()
                    self.events[phase] = original

    def test_prohibited_markers_even_with_valid_digest(self):
        for marker in ("fixture active", "m20-acceptance", "guest FAIL", "kernel panic",
                       "kernel OOM", "out of memory", "M20 PASS", "M30 PASS"):
            with self.subTest(marker=marker):
                self.write()
                path = self.root / "boot.log"
                path.write_text(path.read_text() + marker + "\n")
                self.manifest["phases"]["boot"]["sha256"] = digest(path.read_bytes())
                self.evidence.write_text(json.dumps(self.manifest))
                self.reject()

    def test_fake_model_success(self):
        for name, key, value in (("ai.summary", "generated_tokens", 0),
                                 ("ai.summary", "guest", False),
                                 ("ai.plan", "model_sha256", "0" * 64),
                                 ("ai.plan", "structured", False),
                                 ("plan.user_confirmed", "confirmed", False),
                                 ("plan.undone", "transaction_id", "transaction:wrong")):
            with self.subTest(name=name, key=key):
                data = self.event(name)["data"]
                old = data[key]
                data[key] = value
                self.write()
                self.reject()
                data[key] = old

    def test_stress_invalid_and_missing(self):
        for phase in ("boot", "offline"):
            data = self.event("stress.measured", phase)["data"]
            original = copy.deepcopy(data)
            for key in sa.STRESS_LIMITS:
                for value in (None, -1, True, sa.STRESS_LIMITS[key] + 1):
                    with self.subTest(phase=phase, key=key, value=value):
                        data.clear()
                        data.update(original)
                        if value is None:
                            del data[key]
                        else:
                            data[key] = value
                        self.write()
                        self.reject()
            for key in ("limits", "duration_ms", "desktop_samples", "audio_frames", "granite_loaded",
                        "memory_bytes_start", "memory_bytes_end", "handles_start", "handles_end"):
                data.clear()
                data.update(original)
                del data[key]
                self.write()
                self.reject()
            data.clear()
            data.update(original)

    def test_every_required_event_is_necessary(self):
        for phase in sa.EVENT_SEQUENCES:
            original = copy.deepcopy(self.events[phase])
            for index, event in enumerate(original):
                with self.subTest(phase=phase, event=event["event"]):
                    self.events[phase] = original[:index] + original[index + 1:]
                    self.write()
                    self.reject()
            self.events[phase] = original

    def test_resource_growth_and_limit_types(self):
        data = self.event("stress.measured")["data"]
        data["handles_end"] += 1
        self.write()
        self.reject()
        data["handles_end"] -= 1
        data["limits"]["oom_count"] = False
        self.write()
        self.reject()

    def test_disk_chain_identity_and_offline_network(self):
        for key, value in (("disk_start_sha256", "0" * 64), ("disk_id", "disk:other"),
                           ("disk_id", ""), ("network", True),
                           ("machine", {**sa.MACHINE, "vcpus": 8})):
            with self.subTest(key=key):
                record = self.manifest["phases"]["offline"]
                old = record[key]
                record[key] = value
                self.write()
                self.reject()
                record[key] = old

    def test_restart_stable_identity_and_history(self):
        for name, key in (("files.restored", "object_id"), ("servo.history_restored", "history_id")):
            data = self.event(name, "restart")["data"]
            old = data[key]
            data[key] = "wrong:identity"
            self.write()
            self.reject()
            data[key] = old

    def test_unsafe_paths(self):
        record = self.manifest["phases"]["boot"]
        for value in ("../boot.log", "/boot.log", "a/../boot.log", "a\\boot.log", "./boot.log"):
            record["path"] = value
            self.write()
            self.reject()
        (self.root / "linked.log").symlink_to(self.root / "boot.log")
        record["path"] = "linked.log"
        self.write()
        self.reject()
        (self.root / "linked-dir").symlink_to(self.root, target_is_directory=True)
        record["path"] = "linked-dir/boot.log"
        self.write()
        self.reject()

    def test_oversized_logs(self):
        with mock.patch.object(sa, "MAX_LOG_BYTES", 32):
            self.reject()

    def test_duplicate_keys_and_invalid_json(self):
        for text in ('{"schema_version":1,"schema_version":1}', '{"x":NaN}', '{}', '[]'):
            self.evidence.write_text(text)
            self.reject()

    def test_production_gate(self):
        self.info.write_text(self.info.read_text().replace("init_features=production-session",
                                                         "init_features=production-session,m20-acceptance"))
        self.reject()

    def test_cli_clean_tree_gate_and_output_scope(self):
        with mock.patch.object(sa, "git_source_revision", return_value=REVISION) as revision:
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                result = sa.main(["--root", str(self.root), "--image", str(self.image),
                                  "--evidence", str(self.evidence)])
            self.assertEqual(result, 0)
            revision.assert_called_once_with(self.root)
            self.assertIn("integrated session evidence validated", out.getvalue())
            self.assertIn("M30 distribution/legal acceptance NOT_EVALUATED", out.getvalue())
            self.assertNotIn("M30 PASS", out.getvalue())
        with mock.patch.object(sa, "git_source_revision", side_effect=release.ReleaseError("dirty tree")):
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(sa.main(["--root", str(self.root), "--image", str(self.image),
                                          "--evidence", str(self.evidence)]), 1)


if __name__ == "__main__":
    unittest.main()
