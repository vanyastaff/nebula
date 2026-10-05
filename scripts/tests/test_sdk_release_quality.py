"""Synthetic NS20 trust-boundary probes; these never certify SDK builds."""

import argparse
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("quality", Path(__file__).parents[1] / "sdk-release-quality.py")
quality = importlib.util.module_from_spec(spec)
spec.loader.exec_module(quality)


class Admission(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.candidate = self.root / "candidate"
        self.candidate.mkdir()
        manifest = self.root / "Cargo.toml"
        manifest.write_text('[features]\ndefault=["derive"]\nderive=[]\nembedded=[]\nhttp=[]\n', encoding="utf-8")
        self.args = argparse.Namespace(candidate=self.candidate, sdk_manifest=manifest,
                                       source_revision="a" * 40, base_revision="b" * 40,
                                       repository="fixture/nebula", run_id="123", run_attempt=1)
        observations = []
        for case, commands in quality.plan(["default", "derive", "embedded", "http"], "b" * 40).items():
            executions = []
            for index, command in enumerate(commands):
                filename = f"{case}-{index}.log"
                data = (b"synthetic verifier fixture, not execution evidence\n"
                        + b"test result: ok. 2 passed; 0 failed;\n" * max(1, command.count("--test")))
                (self.candidate / filename).write_bytes(data)
                executions.append({"argv": command, "exit_code": 0, "log": filename,
                                   "log_sha256": quality.digest(data)})
            observations.append({"case": case, "executions": executions})
        self.report = {"schema_version": 1, "gate": "NS20", "identity": quality.identity(self.args),
                       "base_revision": "b" * 40, "sdk_features": ["default", "derive", "embedded", "http"],
                       "toolchain": "rustc 1.97.1 (synthetic fixture)", "observations": observations}

    def verify(self):
        (self.candidate / "sdk-release-quality.json").write_text(json.dumps(self.report), encoding="utf-8")
        with contextlib.redirect_stdout(io.StringIO()):
            return quality.verify(self.args)

    def test_complete_inventory_is_admitted(self):
        self.assertEqual(self.verify(), 0)

    def test_failed_cancelled_and_skipped_commands_cannot_be_passes(self):
        for status in (1, 101, -15, "skipped", False):
            with self.subTest(status=status):
                self.report["observations"][0]["executions"][0]["exit_code"] = status
                with self.assertRaises(ValueError):
                    self.verify()

    def test_changed_log_is_rejected_even_when_report_claims_success(self):
        (self.candidate / "api-snapshots-0.log").write_text("tampered", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "log changed"):
            self.verify()

    def test_zero_test_harness_is_rejected_with_valid_log_hash(self):
        data = b"test result: ok. 0 passed; 0 failed;\n"
        (self.candidate / "api-snapshots-0.log").write_bytes(data)
        self.report["observations"][0]["executions"][0]["log_sha256"] = quality.digest(data)
        with self.assertRaisesRegex(ValueError, "no passing tests"):
            self.verify()

    def test_semver_cannot_select_current_source_as_baseline(self):
        self.report["base_revision"] = self.args.source_revision
        with self.assertRaisesRegex(ValueError, "baseline"):
            self.verify()

    def test_feature_inventory_cannot_omit_real_source_feature(self):
        self.report["sdk_features"].remove("http")
        with self.assertRaisesRegex(ValueError, "feature inventory"):
            self.verify()

    def test_feature_check_cannot_be_replaced_by_noop(self):
        self.report["observations"][-1]["executions"][0]["argv"] = ["true"]
        with self.assertRaisesRegex(ValueError, "substituted"):
            self.verify()

    def test_missing_and_duplicate_cases_are_rejected(self):
        self.report["observations"][1] = self.report["observations"][0]
        with self.assertRaises(ValueError):
            self.verify()

    def test_stale_attempt_and_foreign_source_are_rejected(self):
        for key, value in (("run_attempt", 2), ("source_revision", "c" * 40)):
            with self.subTest(key=key):
                original = self.report["identity"][key]
                self.report["identity"][key] = value
                with self.assertRaisesRegex(ValueError, "runner identity"):
                    self.verify()
                self.report["identity"][key] = original

    def test_duplicate_json_keys_are_rejected(self):
        path = self.root / "duplicate.json"
        path.write_text('{"gate":"NS20","gate":"NS20"}', encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
            quality.decode(path)

    def test_missing_embedded_is_not_a_narrowed_success(self):
        with self.assertRaisesRegex(ValueError, "embedded"):
            quality.profiles(["default", "derive"])

    def test_profile_features_follow_local_cargo_closure(self):
        features = {"default": ["derive"], "derive": ["dep:macros"],
                    "embedded": ["testing", "dep:engine"], "testing": ["dep:uuid"]}
        self.assertEqual(quality.enabled_features(features, ["cargo", "test"]), ["default", "derive"])
        self.assertEqual(quality.enabled_features(features, ["cargo", "test", "--no-default-features"]), [])
        self.assertEqual(quality.enabled_features(features, ["cargo", "test", "--no-default-features",
                                                           "--features", "embedded"]), ["embedded", "testing"])


if __name__ == "__main__":
    unittest.main()
