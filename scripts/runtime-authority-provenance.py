#!/usr/bin/env python3
"""Admit authenticated GitHub run metadata and bounded candidate bytes.

The protected workflow fetches API responses. This module never reads a
candidate manifest to select its expected run, source revision, or digest.
"""

import argparse
import datetime
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import stat
import zipfile


WORKFLOW = ".github/workflows/test-matrix.yml"
PRODUCER_JOB = "PostgreSQL storage conformance"
ARTIFACT_NAME = "runtime-authority-candidate"
PROFILE = "runtime-authority"
MAX_ARCHIVE = 256 * 1024 * 1024
MAX_FILE = 4 * 1024 * 1024
MAX_FILES = 256
MAX_NS19_FILE = 64 * 1024 * 1024
MAX_NS19_SET = 512 * 1024 * 1024
# ZIP framing is bounded separately; the base and typed binary payload budgets
# remain independent during extraction.
MAX_RUNTIME_TRANSPORT = MAX_ARCHIVE + MAX_NS19_SET + 1024 * 1024
NS19_ARCHIVE_PARTS = ("NS19", "published-manifest-precision", "archives")
NS19_REPORT_PARTS = ("NS19", "published-manifest-precision", "published-manifest-precision.json")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(value):
    require(isinstance(value, str) and re.fullmatch(r"[a-f0-9]{40}", value), "invalid revision")
    return value


def positive(value):
    require(type(value) is int and value > 0, "invalid GitHub identity")
    return value


def timestamp(value):
    return datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))


def select_profile(source_workflow):
    """Select protected policy only from the OIDC-bound caller workflow."""
    global WORKFLOW, PRODUCER_JOB, ARTIFACT_NAME, PROFILE
    if source_workflow == ".github/workflows/test-matrix.yml":
        WORKFLOW, PRODUCER_JOB, ARTIFACT_NAME, PROFILE = source_workflow, "PostgreSQL storage conformance", "runtime-authority-candidate", "runtime-authority"
    elif source_workflow == ".github/workflows/ci.yml":
        WORKFLOW, PRODUCER_JOB, ARTIFACT_NAME, PROFILE = source_workflow, "SDK release quality", "sdk-release-quality-candidate", "sdk-release-quality"
    else:
        raise ValueError("untrusted source workflow profile")
    return PROFILE


def compatibility_baseline(context, commit):
    """Use original event baseline, otherwise the authenticated source first parent."""
    require(commit["sha"] == sha(context["source_revision"]), "source commit mismatch")
    baseline = context.get("base_revision", "")
    if baseline and baseline != "0" * 40:
        return sha(baseline)
    require(len(commit["parents"]) > 0, "source has no compatibility baseline")
    return sha(commit["parents"][0]["sha"])


def resolve(metadata, repository, run_id, attempt, policy_revision):
    """Return independent expectations; all metadata inputs are GitHub API data."""
    select_profile(metadata["source_context"]["source_workflow"])
    run = metadata["run"]
    workflow = metadata["workflow"]
    branch = metadata["branch"]
    suite = metadata["suite"]
    require(branch["name"] == "main" and branch["protected"] is True, "main is not protected")
    require(run["repository"]["full_name"] == repository, "wrong source repository")
    require(run["id"] == positive(run_id) and run["run_attempt"] == positive(attempt), "stale run attempt")
    # The reusable verifier runs inside the source workflow. Its producer
    # must have succeeded; the enclosing run is necessarily still active.
    require((run["status"] == "in_progress" and run["conclusion"] is None)
            or (run["status"] == "completed" and run["conclusion"] == "success"),
            "source run is not active or successful")
    require(workflow["path"] == WORKFLOW and run["path"] == WORKFLOW, "wrong source workflow")
    require(workflow["id"] == run["workflow_id"], "wrong source workflow identity")
    require(suite["id"] == run["check_suite_id"], "wrong source check suite")
    require(suite["head_sha"] == run["head_sha"], "source check suite revision mismatch")
    context = metadata["source_context"]
    require(context["repository"] == repository and context["run_id"] == str(run_id)
            and context["run_attempt"] == str(attempt), "protected runner context mismatch")
    source = sha(context["source_revision"])
    require(context["head_revision"] == run["head_sha"], "source run head mismatch")
    require(context["policy_revision"] == policy_revision, "protected policy context mismatch")
    event = run["event"]
    if event == "pull_request":
        require(re.fullmatch(r"refs/pull/[1-9][0-9]*/merge", context["source_ref"]),
                "untrusted pull request source ref")
    else:
        require(event in ("push", "merge_group", "workflow_dispatch"), "unsupported source event")
        require(source == run["head_sha"], "source revision mismatch")
        if event in ("push", "workflow_dispatch"):
            require(run["head_branch"] == "main" and context["source_ref"] == "refs/heads/main",
                    "untrusted source ref")
        else:
            require(context["source_ref"].startswith("refs/heads/gh-readonly-queue/main/"),
                    "untrusted merge-group source ref")

    require(metadata["jobs"]["total_count"] == len(metadata["jobs"]["jobs"]), "incomplete job inventory")
    require(metadata["artifacts"]["total_count"] == len(metadata["artifacts"]["artifacts"]),
            "incomplete artifact inventory")
    jobs = [job for job in metadata["jobs"]["jobs"] if job["name"] == PRODUCER_JOB]
    require(len(jobs) == 1 and jobs[0]["conclusion"] == "success", "producer job did not succeed")
    job = jobs[0]
    require(job["run_id"] == run_id and job["run_attempt"] == attempt, "producer job attempt mismatch")
    start, finish = timestamp(job["started_at"]), timestamp(job["completed_at"])
    candidates = [artifact for artifact in metadata["artifacts"]["artifacts"]
                  if artifact["name"] == ARTIFACT_NAME and
                  start <= timestamp(artifact["created_at"]) <= finish]
    require(len(candidates) == 1, "missing or ambiguous source-attempt candidate")
    artifact = candidates[0]
    require(artifact["expired"] is False, "candidate expired")
    require(0 < artifact["size_in_bytes"] <= transport_limit(), "candidate archive size exceeded")
    origin = artifact["workflow_run"]
    require(origin["id"] == run_id and origin["head_sha"] == run["head_sha"], "candidate belongs to another run")
    require(isinstance(artifact["digest"], str) and
            re.fullmatch(r"sha256:[a-f0-9]{64}", artifact["digest"]), "missing authenticated archive digest")
    return {
        "repository": repository,
        "profile": PROFILE,
        "source_revision": source,
        "source_event": event,
        "run_id": run_id,
        "run_attempt": attempt,
        "artifact_id": positive(artifact["id"]),
        "artifact_sha256": artifact["digest"][7:],
        "policy_revision": sha(policy_revision),
    }


def ns19_archive(path):
    return (PROFILE == "runtime-authority" and len(path.parts) == 4
            and path.parts[:3] == NS19_ARCHIVE_PARTS
            and re.fullmatch(r"[a-zA-Z0-9_-]+-[0-9][a-zA-Z0-9.+-]*\.crate", path.parts[3]))


def transport_limit():
    return MAX_RUNTIME_TRANSPORT if PROFILE == "runtime-authority" else MAX_ARCHIVE


def extract(archive, destination, expected_digest):
    """Check GitHub's independent digest before bounded non-executable extraction."""
    require(re.fullmatch(r"[a-f0-9]{64}", expected_digest), "invalid archive expectation")
    require(0 < archive.stat().st_size <= transport_limit(), "candidate archive size exceeded")
    with archive.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    require(digest == expected_digest, "archive digest mismatch")
    require(not destination.exists(), "candidate destination must be new")
    with zipfile.ZipFile(archive) as source:
        entries = source.infolist()
        require(0 < len(entries) <= MAX_FILES, "candidate inventory size exceeded")
        base_bytes = 0
        ns19_bytes = 0
        names = set()
        for entry in entries:
            # ZipInfo normalizes platform separators and truncates NUL names.
            # Admit the original wire name, never the normalized surrogate.
            name = entry.orig_filename
            path = PurePosixPath(name)
            # Preserve the wire namespace rather than admitting aliases after
            # PurePosixPath has discarded dot or empty components.
            components = name[:-1].split("/") if name.endswith("/") else name.split("/")
            require(all(component not in ("", ".") for component in components),
                    "non-canonical archive path")
            require(name not in names, "duplicate archive entry")
            names.add(name)
            require(name == entry.filename and not path.is_absolute() and ".." not in path.parts
                    and "\\" not in name and ":" not in name and "\0" not in name and path.parts,
                    "unsafe archive path")
            if PROFILE == "sdk-release-quality":
                require(len(path.parts) == 1 and (name == "sdk-release-quality.json" or
                        re.fullmatch(r"(?:api-snapshots|semver-classification|msrv|rustdoc|supported-feature-matrices)-[0-9]+\.log", name)),
                        "unexpected SDK candidate root")
            else:
                require(path.parts[0] in ("runtime-authority", "runtime-authority-bundle", "runtime-authority-expected.json")
                        or ns19_archive(path)
                        or path.parts == NS19_REPORT_PARTS
                        or (entry.is_dir() and path.parts == NS19_ARCHIVE_PARTS[:len(path.parts)]),
                        "unexpected candidate root")
            mode = entry.external_attr >> 16
            require(not stat.S_ISLNK(mode) and (not stat.S_IFMT(mode) or stat.S_ISREG(mode) or stat.S_ISDIR(mode)),
                    "non-regular archive entry")
            if ns19_archive(path):
                require(not entry.is_dir() and entry.file_size <= MAX_NS19_FILE,
                        "NS19 archive file size exceeded")
                ns19_bytes += entry.file_size
                require(ns19_bytes <= MAX_NS19_SET, "NS19 archive set size exceeded")
            else:
                require(entry.file_size <= MAX_FILE, "candidate file size exceeded")
                base_bytes += entry.file_size
                require(base_bytes <= MAX_ARCHIVE, "candidate expanded size exceeded")
        destination.mkdir()
        for entry in entries:
            target = destination.joinpath(*PurePosixPath(entry.filename).parts)
            if entry.is_dir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with target.open("xb") as output:
                    output.write(source.read(entry))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    identity = commands.add_parser("resolve")
    identity.add_argument("--metadata", type=Path, required=True)
    identity.add_argument("--repository", required=True)
    identity.add_argument("--run-id", type=int, required=True)
    identity.add_argument("--run-attempt", type=int, required=True)
    identity.add_argument("--policy-revision", required=True)
    archive = commands.add_parser("extract")
    archive.add_argument("--source-workflow", required=True)
    archive.add_argument("--archive", type=Path, required=True)
    archive.add_argument("--destination", type=Path, required=True)
    archive.add_argument("--expected-sha256", required=True)
    args = parser.parse_args()
    try:
        if args.command == "resolve":
            metadata = json.loads(args.metadata.read_text(encoding="utf-8"))
            result = resolve(metadata, args.repository, args.run_id, args.run_attempt, args.policy_revision)
            print(json.dumps(result, sort_keys=True))
        else:
            select_profile(args.source_workflow)
            extract(args.archive, args.destination, args.expected_sha256)
    except (ValueError, KeyError, TypeError, OSError, zipfile.BadZipFile) as error:
        parser.exit(1, f"runtime authority provenance rejected: {error}\n")


if __name__ == "__main__":
    main()
