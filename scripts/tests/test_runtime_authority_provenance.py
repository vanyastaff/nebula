"""Distinguishing checks against the production protected-run resolver."""

import copy
import base64
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile


SCRIPT = Path(__file__).resolve().parents[1] / "runtime-authority-provenance.py"
SPEC = importlib.util.spec_from_file_location("provenance", SCRIPT)
provenance = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(provenance)
WORKFLOW_SOURCE = SCRIPT.parent.parent / ".github/workflows/runtime-authority-provenance.yml"
# Exercise the actual trusted bootstrap before checkout, not a copied oracle.
bootstrap_source = WORKFLOW_SOURCE.read_text(encoding="utf-8").split("python3 - <<'PY'\n", 1)[1].split("          PY\n", 1)[0]
bootstrap_source = "\n".join(line[10:] for line in bootstrap_source.splitlines())
BOOTSTRAP = {"__name__": "protected_bootstrap_test"}
exec(compile(bootstrap_source, str(WORKFLOW_SOURCE), "exec"), BOOTSTRAP)

HEAD, BASE, MERGE, POLICY = "a" * 40, "b" * 40, "c" * 40, "d" * 40
REPOSITORY = "vanyastaff/nebula"


def fixture():
    return {
        "run": {"id": 100, "run_attempt": 2, "repository": {"full_name": REPOSITORY},
                "status": "completed", "conclusion": "success", "workflow_id": 10,
                "path": ".github/workflows/test-matrix.yml", "check_suite_id": 20, "head_sha": HEAD,
                "event": "pull_request", "pull_requests": [
                    {"number": 30, "head": {"sha": HEAD}, "base": {"sha": BASE}}]},
        "workflow": {"id": 10, "path": ".github/workflows/test-matrix.yml"},
        "branch": {"name": "main", "protected": True},
        "suite": {"id": 20, "head_sha": HEAD},
        "source_context": {"repository": REPOSITORY, "run_id": "100", "run_attempt": "2",
                           "source_revision": MERGE, "head_revision": HEAD,
                           "source_ref": "refs/pull/30/merge", "policy_revision": POLICY,
                           "source_workflow": ".github/workflows/test-matrix.yml",
                           "source_workflow_ref": REPOSITORY + "/.github/workflows/test-matrix.yml@refs/pull/30/merge"},
        "pull_request": {"number": 30, "state": "open", "merge_commit_sha": MERGE,
                         "head": {"sha": HEAD}, "base": {"sha": BASE, "ref": "main",
                         "repo": {"full_name": REPOSITORY}}},
        "merge_commit": {"sha": MERGE, "parents": [{"sha": BASE}, {"sha": HEAD}]},
        "jobs": {"total_count": 1, "jobs": [
            {"name": "PostgreSQL storage conformance", "conclusion": "success", "run_id": 100,
             "run_attempt": 2, "started_at": "2026-10-05T10:00:00Z",
             "completed_at": "2026-10-05T10:20:00Z"}]},
        "artifacts": {"total_count": 1, "artifacts": [
            {"name": "runtime-authority-candidate", "id": 40, "expired": False,
             "created_at": "2026-10-05T10:19:00Z", "size_in_bytes": 1024,
             "digest": "sha256:" + "e" * 64, "workflow_run": {"id": 100, "head_sha": HEAD}}]},
    }


class IdentityTests(unittest.TestCase):
    def resolve(self, metadata):
        return provenance.resolve(metadata, REPOSITORY, 100, 2, POLICY)

    def test_pr_uses_original_protected_merge_context_instead_of_head_or_candidate_claim(self):
        data = fixture()
        data["candidate_claim"] = {"source_revision": "f" * 40, "run_id": 999}
        result = self.resolve(data)
        self.assertEqual(result["source_revision"], MERGE)
        self.assertNotEqual(result["source_revision"], HEAD)
        self.assertEqual(result["run_id"], 100)

    def test_push_and_merge_group_use_source_run_revision(self):
        for event in ("push", "merge_group", "workflow_dispatch"):
            with self.subTest(event=event):
                data = fixture()
                data["run"].update(event=event, head_branch="main")
                data["source_context"].update(source_revision=HEAD,
                    source_ref="refs/heads/gh-readonly-queue/main/pr-30-test" if event == "merge_group" else "refs/heads/main")
                self.assertEqual(self.resolve(data)["source_revision"], HEAD)

    def test_untrusted_source_and_replay_variants_are_rejected(self):
        changes = [
            ("run", "id", 101), ("run", "run_attempt", 1),
            ("run", "conclusion", "failure"), ("run", "status", "queued"),
            ("run", "workflow_id", 11), ("run", "path", ".github/workflows/other.yml"),
            ("run", "check_suite_id", 21), ("suite", "head_sha", BASE),
            ("branch", "protected", False), ("branch", "name", "topic"),
            ("source_context", "run_id", "101"), ("source_context", "run_attempt", "1"),
            ("source_context", "head_revision", "f" * 40),
            ("source_context", "source_ref", "refs/heads/attacker"),
            ("source_context", "policy_revision", "f" * 40),
            ("run", "event", "pull_request_target"),
            ("jobs", "total_count", 2), ("artifacts", "total_count", 2),
        ]
        for section, key, value in changes:
            with self.subTest(section=section, key=key):
                data = fixture()
                data[section][key] = value
                with self.assertRaises((ValueError, KeyError)):
                    self.resolve(data)

    def test_mutable_current_pr_refs_cannot_override_original_merge_context(self):
        for ref in ("head", "base"):
            with self.subTest(ref=ref):
                data = fixture()
                data["pull_request"][ref]["sha"] = "f" * 40
                self.assertEqual(self.resolve(data)["source_revision"], MERGE)

    def test_mutable_associated_pr_cannot_replace_original_run_head(self):
        data = fixture()
        advanced = "f" * 40
        data["run"]["pull_requests"][0]["head"]["sha"] = advanced
        data["pull_request"]["head"]["sha"] = advanced
        self.assertEqual(self.resolve(data)["source_revision"], MERGE)

    def test_later_api_merge_identity_cannot_override_original_merge_context(self):
        for owner in ("run", "suite"):
            with self.subTest(owner=owner):
                data = fixture()
                data[owner]["merge_commit_sha"] = "f" * 40
                self.assertEqual(self.resolve(data)["source_revision"], MERGE)

    def test_api_merge_parents_are_not_a_trust_anchor(self):
        data = fixture()
        data["merge_commit"]["parents"][0]["sha"] = "f" * 40
        self.assertEqual(self.resolve(data)["source_revision"], MERGE)

    def test_base_and_merge_refresh_attack_cannot_change_expected_source(self):
        data = fixture()
        advanced = "f" * 40
        data["run"]["pull_requests"][0]["base"]["sha"] = advanced
        data["pull_request"]["base"]["sha"] = advanced
        data["pull_request"]["merge_commit_sha"] = "e" * 40
        data["merge_commit"] = {"sha": "e" * 40, "parents": [{"sha": advanced}, {"sha": HEAD}]}
        self.assertEqual(self.resolve(data)["source_revision"], MERGE)

    def test_active_source_run_is_admitted_only_with_completed_successful_producer(self):
        data = fixture()
        data["run"].update(status="in_progress", conclusion=None)
        self.assertEqual(self.resolve(data)["source_revision"], MERGE)
        data["jobs"]["jobs"][0]["conclusion"] = None
        with self.assertRaisesRegex(ValueError, "producer job"):
            self.resolve(data)

    def test_other_repository_is_rejected(self):
        data = fixture()
        data["run"]["repository"]["full_name"] = "attacker/nebula"
        with self.assertRaisesRegex(ValueError, "repository"):
            self.resolve(data)

    def test_failed_or_other_attempt_producer_is_rejected(self):
        for key, value in (("conclusion", "failure"), ("run_attempt", 1), ("run_id", 101)):
            with self.subTest(key=key):
                data = fixture()
                data["jobs"]["jobs"][0][key] = value
                with self.assertRaises(ValueError):
                    self.resolve(data)

    def test_old_attempt_artifact_is_rejected(self):
        data = fixture()
        data["artifacts"]["artifacts"][0]["created_at"] = "2026-10-05T09:19:00Z"
        with self.assertRaisesRegex(ValueError, "missing or ambiguous"):
            self.resolve(data)

    def test_candidate_identity_digest_and_inventory_fail_closed(self):
        for key, value in (("expired", True), ("digest", None), ("size_in_bytes", 0),
                           ("size_in_bytes", provenance.MAX_RUNTIME_TRANSPORT + 1)):
            with self.subTest(key=key):
                data = fixture()
                data["artifacts"]["artifacts"][0][key] = value
                with self.assertRaises(ValueError):
                    self.resolve(data)
        data = fixture()
        data["artifacts"]["artifacts"].append(copy.deepcopy(data["artifacts"]["artifacts"][0]))
        data["artifacts"]["total_count"] = 2
        with self.assertRaisesRegex(ValueError, "ambiguous"):
            self.resolve(data)


class ArchiveTests(unittest.TestCase):
    def extract(self, entries, digest=None):
        provenance.select_profile(".github/workflows/test-matrix.yml")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "candidate.zip"
            with zipfile.ZipFile(archive, "w") as output:
                for name, value in entries:
                    if isinstance(name, str):
                        # Preserve malicious wire paths instead of letting the
                        # Windows ZipInfo constructor normalize them first.
                        entry = zipfile.ZipInfo("placeholder")
                        entry.filename = name
                        name = entry
                    output.writestr(name, value)
            expected = digest or hashlib.sha256(archive.read_bytes()).hexdigest()
            destination = root / "candidate"
            provenance.extract(archive, destination, expected)
            return sorted(str(path.relative_to(destination)) for path in destination.rglob("*") if path.is_file())

    def test_valid_archive_is_extracted_as_data(self):
        self.assertEqual(len(self.extract([("runtime-authority-expected.json", "{}"),
                                         ("runtime-authority-bundle/ns01/in-memory.json", "{}")])), 2)

    def test_replaced_candidate_with_original_trusted_digest_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            self.extract([("runtime-authority-expected.json", "changed")], "f" * 64)

    def test_bundle_and_manifest_replaced_together_fail_before_extraction(self):
        provenance.select_profile(".github/workflows/test-matrix.yml")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "candidate.zip"
            with zipfile.ZipFile(archive, "w") as output:
                output.writestr("runtime-authority-expected.json", '{"inventory":"original"}')
                output.writestr("runtime-authority-bundle/ns01/in-memory.json", '{"observation":"original"}')
            trusted = hashlib.sha256(archive.read_bytes()).hexdigest()
            with zipfile.ZipFile(archive, "w") as output:
                output.writestr("runtime-authority-expected.json", '{"inventory":"replaced"}')
                output.writestr("runtime-authority-bundle/ns01/in-memory.json", '{"observation":"replaced"}')
            destination = root / "candidate"
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                provenance.extract(archive, destination, trusted)
            self.assertFalse(destination.exists())

    def test_archive_path_escape_or_extra_code_is_rejected(self):
        for name in ("../outside", "/outside", "C:/outside", "runtime-authority/../../outside",
                     "runtime-authority\\outside", ".cargo/config.toml", "scripts/evil.py",
                     "./runtime-authority/outside", "runtime-authority//outside",
                     "./NS19/published-manifest-precision/archives/nebula-sdk-0.32.0.crate",
                     "NS19//published-manifest-precision/archives/nebula-sdk-0.32.0.crate"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.extract([(name, "untrusted")])

    def test_symlink_is_rejected(self):
        link = zipfile.ZipInfo("runtime-authority/link")
        link.create_system = 3
        link.external_attr = (stat.S_IFLNK | 0o777) << 16
        with self.assertRaisesRegex(ValueError, "non-regular"):
            self.extract([(link, "../outside")])

    def test_oversized_observation_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "file size"):
            self.extract([("runtime-authority/large.json", "x" * (provenance.MAX_FILE + 1))])

    def test_only_typed_ns19_crate_subtree_receives_larger_file_budget(self):
        payload = "x" * (provenance.MAX_FILE + 1)
        exact = "NS19/published-manifest-precision/archives/nebula-sdk-0.32.0.crate"
        self.assertEqual(len(self.extract([(exact, payload)])), 1)
        for name in ("runtime-authority/nebula-sdk-0.32.0.crate",
                     "NS19/published-manifest-precision/archives/log.txt",
                     "NS19/published-manifest-precision/archives/nested/nebula-sdk-0.32.0.crate",
                     "NS20/published-manifest-precision/archives/nebula-sdk-0.32.0.crate"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.extract([(name, payload)])

    def test_ns19_report_receives_only_the_ordinary_json_budget(self):
        report = "NS19/published-manifest-precision/published-manifest-precision.json"
        self.assertEqual(len(self.extract([(report, "{}") ])), 1)
        with self.assertRaisesRegex(ValueError, "file size"):
            self.extract([(report, "x" * (provenance.MAX_FILE + 1))])
        with self.assertRaises(ValueError):
            self.extract([("NS19/published-manifest-precision/extra.json", "{}")])

    def test_protected_ns19_archive_verification_precedes_effective_states_and_attestation(self):
        source = WORKFLOW_SOURCE.read_text()
        start = source.index('[[ "$profile" == "runtime-authority" ]]')
        archive = source.index("cargo xtask packaging verify-archives", start)
        semantic = source.index("cargo xtask north-star-gates verify-runtime-authority", start)
        attest = source.index("- name: Attest independently verified candidate digest", start)
        self.assertLess(archive, semantic)
        self.assertLess(semantic, attest)
        self.assertIn('--report "$candidate/NS19/published-manifest-precision/published-manifest-precision.json"', source)
        self.assertIn('--archive-root "$candidate/NS19/published-manifest-precision/archives"', source)

    def test_ns19_binary_and_base_payload_budgets_are_separate(self):
        exact = "NS19/published-manifest-precision/archives/nebula-sdk-0.32.0.crate"
        with patch.object(provenance, "MAX_NS19_FILE", 8), patch.object(provenance, "MAX_NS19_SET", 10):
            with self.assertRaisesRegex(ValueError, "NS19 archive file size"):
                self.extract([(exact, "x" * 9)])
            with self.assertRaisesRegex(ValueError, "NS19 archive set size"):
                self.extract([(exact, "x" * 6),
                    ("NS19/published-manifest-precision/archives/nebula-core-0.32.0.crate", "x" * 6)])
        with patch.object(provenance, "MAX_ARCHIVE", 5):
            with self.assertRaisesRegex(ValueError, "expanded size"):
                self.extract([(exact, "x" * 6), ("runtime-authority/a.json", "x" * 6)])

    def test_cli_failure_emits_no_trusted_identity_stdout(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "invalid.json"
            path.write_text("{}", encoding="utf-8")
            result = subprocess.run([sys.executable, str(SCRIPT), "resolve", "--metadata", str(path),
                                     "--repository", REPOSITORY, "--run-id", "100", "--run-attempt", "2",
                                     "--policy-revision", POLICY], capture_output=True, check=False)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, b"")


class BootstrapTests(unittest.TestCase):
    def claims(self):
        return {"iss": "https://token.actions.githubusercontent.com", "repository": REPOSITORY,
                "aud": "nebula-runtime-authority-policy",
                "sha": MERGE, "ref": "refs/pull/30/merge", "run_id": "100", "run_attempt": "2",
                "workflow_ref": REPOSITORY + "/.github/workflows/test-matrix.yml@refs/pull/30/merge",
                "job_workflow_ref": REPOSITORY + "/.github/workflows/runtime-authority-provenance.yml@refs/heads/main",
                "job_workflow_sha": POLICY}

    def test_protected_reusable_identity_and_original_merge_context_are_admitted(self):
        self.assertEqual(BOOTSTRAP["admit_policy_claims"](self.claims(), fixture()["source_context"]), POLICY)

    def test_replaced_signer_or_source_identity_is_rejected_before_checkout(self):
        for key, value in (("iss", "https://attacker.invalid"), ("aud", "attacker-audience"),
                           ("repository", "attacker/nebula"),
                           ("sha", "f" * 40), ("ref", "refs/heads/main"), ("run_id", "101"),
                           ("run_attempt", "1"), ("workflow_ref", REPOSITORY + "/.github/workflows/ci.yml@refs/pull/30/merge"), ("job_workflow_ref", REPOSITORY + "/.github/workflows/runtime-authority-provenance.yml@refs/pull/30/merge"),
                           ("job_workflow_sha", "not-a-commit")):
            with self.subTest(key=key):
                claims = self.claims()
                claims[key] = value
                with self.assertRaises(ValueError):
                    BOOTSTRAP["admit_policy_claims"](claims, fixture()["source_context"])

    def test_sdk_caller_oidc_profile_is_bound_without_candidate_input(self):
        context = fixture()["source_context"]
        context["source_workflow_ref"] = REPOSITORY + "/.github/workflows/ci.yml@refs/pull/30/merge"
        claims = self.claims()
        claims["workflow_ref"] = context["source_workflow_ref"]
        self.assertEqual(BOOTSTRAP["admit_policy_claims"](claims, context), POLICY)
        claims["workflow_ref"] = REPOSITORY + "/.github/workflows/test-matrix.yml@refs/pull/30/merge"
        with self.assertRaisesRegex(ValueError, "caller workflow"):
            BOOTSTRAP["admit_policy_claims"](claims, context)

    def test_authenticated_endpoint_bootstrap_retains_only_selected_context(self):
        claims = base64.urlsafe_b64encode(json.dumps(self.claims()).encode()).decode().rstrip("=")
        token = "header." + claims + ".signature"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            environment = {"GITHUB_REPOSITORY": REPOSITORY, "SOURCE_RUN_ID": "100", "SOURCE_RUN_ATTEMPT": "2",
                           "SOURCE_REVISION": MERGE, "SOURCE_HEAD_REVISION": HEAD, "SOURCE_REF": "refs/pull/30/merge",
                           "SOURCE_WORKFLOW_REF": REPOSITORY + "/.github/workflows/test-matrix.yml@refs/pull/30/merge",
                           "ACTIONS_ID_TOKEN_REQUEST_URL": "https://pipelines.actions.githubusercontent.com/token?api-version=2",
                           "ACTIONS_ID_TOKEN_REQUEST_TOKEN": "fixture-request-token", "RUNNER_TEMP": str(root),
                           "GITHUB_OUTPUT": str(root / "outputs.txt")}
            requests = []
            def endpoint(request, timeout):
                requests.append(request)
                self.assertEqual(timeout, 30)
                return io.BytesIO(json.dumps({"value": token}).encode())
            with patch.dict(BOOTSTRAP["os"].environ, environment, clear=True), patch.dict(BOOTSTRAP, urlopen=endpoint):
                BOOTSTRAP["bootstrap"]()
            self.assertIn("audience=nebula-runtime-authority-policy", requests[0].full_url)
            context = json.loads((root / "source-context.json").read_text())
            self.assertEqual(context["source_revision"], MERGE)
            self.assertEqual(context["policy_revision"], POLICY)
            self.assertEqual((root / "outputs.txt").read_text(), "policy-revision=" + POLICY + "\n")
            for path in root.iterdir():
                self.assertNotIn(token, path.read_text())
                self.assertNotIn("fixture-request-token", path.read_text())


class SdkProfileTests(unittest.TestCase):
    def tearDown(self):
        provenance.select_profile(".github/workflows/test-matrix.yml")

    def sdk_fixture(self):
        data = fixture()
        path = ".github/workflows/ci.yml"
        data["source_context"]["source_workflow"] = path
        data["run"]["path"] = path
        data["workflow"]["path"] = path
        data["jobs"]["jobs"][0]["name"] = "SDK release quality"
        data["artifacts"]["artifacts"][0]["name"] = "sdk-release-quality-candidate"
        return data

    def test_authenticated_source_workflow_selects_sdk_profile(self):
        data = self.sdk_fixture()
        data["candidate_claim"] = {"profile": "runtime-authority"}
        result = provenance.resolve(data, REPOSITORY, 100, 2, POLICY)
        self.assertEqual(result["profile"], "sdk-release-quality")
        self.assertEqual(result["source_revision"], MERGE)

    def test_other_workflow_or_runtime_artifact_cannot_select_sdk_policy(self):
        for section, key, value in (("source_context", "source_workflow", ".github/workflows/attacker.yml"),
                                    ("run", "path", ".github/workflows/test-matrix.yml")):
            data = self.sdk_fixture()
            data[section][key] = value
            with self.assertRaises(ValueError):
                provenance.resolve(data, REPOSITORY, 100, 2, POLICY)
        data = self.sdk_fixture()
        data["artifacts"]["artifacts"][0]["name"] = "runtime-authority-candidate"
        with self.assertRaises(ValueError):
            provenance.resolve(data, REPOSITORY, 100, 2, POLICY)

    def test_compatibility_baseline_uses_original_event_or_exact_source_parent(self):
        context = {"source_revision": MERGE, "base_revision": BASE}
        commit = {"sha": MERGE, "parents": [{"sha": HEAD}]}
        self.assertEqual(provenance.compatibility_baseline(context, commit), BASE)
        for empty in ("", "0" * 40):
            context["base_revision"] = empty
            self.assertEqual(provenance.compatibility_baseline(context, commit), HEAD)
        commit["sha"] = "f" * 40
        with self.assertRaises(ValueError):
            provenance.compatibility_baseline(context, commit)

    def test_sdk_archive_admits_only_flat_case_logs_and_report(self):
        provenance.select_profile(".github/workflows/ci.yml")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for index, name in enumerate(("sdk-release-quality.json", "api-snapshots-0.log",
                                          "scripts/evil.py", "runtime-authority-expected.json", "msrv-0.log/evil",
                                          "NS19/published-manifest-precision/archives/nebula-sdk-0.32.0.crate")):
                archive = root / (str(index) + ".zip")
                with zipfile.ZipFile(archive, "w") as output:
                    output.writestr(name, "{}")
                expected = hashlib.sha256(archive.read_bytes()).hexdigest()
                if index < 2:
                    provenance.extract(archive, root / ("out" + str(index)), expected)
                else:
                    with self.assertRaises(ValueError):
                        provenance.extract(archive, root / ("out" + str(index)), expected)


if __name__ == "__main__":
    unittest.main()
