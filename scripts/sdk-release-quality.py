#!/usr/bin/env python3
"""Execute and admit the backend-independent NS20 SDK release matrix.

Candidate records are not trust anchors. The protected verifier authenticates
the source run/archive before calling verify with independent runner identity.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

CASES = ("api-snapshots", "semver-classification", "msrv", "rustdoc", "supported-feature-matrices")
MAX_FILE = 4 * 1024 * 1024
PACKAGES = ("nebula-sdk", "nebula-api-contract")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def decode(path):
    require(path.is_file() and not path.is_symlink(), "non-regular evidence file")
    data = path.read_bytes()
    require(len(data) <= MAX_FILE, "evidence size exceeded")

    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, "duplicate JSON key")
            result[key] = value
        return result

    return json.loads(data, object_pairs_hook=unique)


def profiles(features):
    require(isinstance(features, list) and features == sorted(set(features)), "invalid feature inventory")
    require(all(isinstance(name, str) and re.fullmatch(r"[a-zA-Z0-9_-]+", name) for name in features),
            "invalid feature name")
    # Embedded is a CP3 release promise. Its absence is a prerequisite failure,
    # rather than a reason to silently narrow the matrix to today's SDK.
    require("embedded" in features, "CP3 embedded feature is absent")
    return [[], ["--no-default-features"], ["--all-features"]] + [
        ["--no-default-features", "--features", feature] for feature in features if feature != "default"
    ]


def plan(features, base):
    require(re.fullmatch(r"[a-f0-9]{40}", base), "baseline must be an exact revision")
    matrix = profiles(features)
    commands = {case: [] for case in CASES}
    for flags in matrix:
        commands["api-snapshots"].append(
            ["cargo", "test", "--locked", "-p", "nebula-sdk", "--test", "public_api_snapshot",
             "--test", "public_api_profiles", *flags])
        commands["supported-feature-matrices"].append(
            ["cargo", "check", "--locked", "-p", "nebula-sdk", "--all-targets", *flags])
    commands["supported-feature-matrices"].append([
        "cargo", "test", "--locked", "-p", "nebula-sdk", "--test", "public_perimeter_external_contract"])
    commands["supported-feature-matrices"].append([
        "cargo", "test", "--locked", "-p", "nebula-sdk", "--test", "persona_external_contract"])
    for package in PACKAGES:
        commands["semver-classification"].append([
            "cargo", "semver-checks", "check-release", "--package", package, "--all-features", "--baseline-rev", base])
        commands["msrv"].append([
            "cargo", "+1.97.1", "check", "--locked", "-p", package, "--all-features", "--all-targets"])
        commands["rustdoc"].append([
            "cargo", "doc", "--locked", "-p", package, "--all-features", "--no-deps"])
    return commands


def write(path, value):
    with path.open("x", encoding="utf-8") as output:
        json.dump(value, output, sort_keys=True, indent=2)
        output.write("\n")


def enabled_features(feature_map, command):
    if "--all-features" in command:
        return sorted(feature_map)
    pending = [] if "--no-default-features" in command else ["default"]
    if "--features" in command:
        pending.extend(command[command.index("--features") + 1].split(','))
    enabled = set()
    while pending:
        name = pending.pop()
        if name in feature_map and name not in enabled:
            enabled.add(name)
            pending.extend(feature_map[name])
    return sorted(enabled)


def identity(args):
    require(re.fullmatch(r"[a-f0-9]{40}", args.source_revision), "invalid source revision")
    require(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repository), "invalid repository")
    require(re.fullmatch(r"[1-9][0-9]*", args.run_id), "invalid workflow run")
    require(args.run_attempt > 0, "invalid workflow attempt")
    return {"repository": args.repository, "source_revision": args.source_revision,
            "run_id": args.run_id, "run_attempt": args.run_attempt,
            "workflow_path": ".github/workflows/ci.yml", "job_id": "sdk-release-quality"}


def run(args):
    expected = identity(args)
    require(subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
            == args.source_revision, "checkout differs from source revision")
    require(not subprocess.check_output(["git", "diff", "HEAD", "--"], text=True), "dirty source checkout")
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], text=True))
    sdk = [package for package in metadata["packages"] if package["name"] == "nebula-sdk"]
    require(len(sdk) == 1, "SDK metadata missing or ambiguous")
    features = sorted(sdk[0]["features"])
    commands = plan(features, args.base_revision)
    require(all(any(package["name"] == name for package in metadata["packages"]) for name in PACKAGES),
            "supported transport package is absent")
    toolchain = subprocess.check_output(["rustc", "--version"], text=True).strip()
    require(toolchain.startswith("rustc 1.97.1 "), "release toolchain must be pinned stable 1.97.1")
    args.output.mkdir()
    observations = []
    environment = dict(os.environ, CARGO_BUILD_JOBS="8", RUSTDOCFLAGS="-D warnings", INSTA_UPDATE="no")
    for case in CASES:
        executions = []
        for index, command in enumerate(commands[case]):
            filename = f"{case}-{index}.log"
            with (args.output / filename).open("xb") as log:
                command_environment = dict(environment, NEBULA_SDK_ACTIVE_FEATURES=','.join(
                    enabled_features(sdk[0]["features"], command)))
                status = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT,
                                        env=command_environment).returncode
            executions.append({"argv": command, "exit_code": status, "log": filename,
                               "log_sha256": digest((args.output / filename).read_bytes())})
        observations.append({"case": case, "executions": executions})
    write(args.output / "sdk-release-quality.json", {
        "schema_version": 1, "gate": "NS20", "identity": expected, "base_revision": args.base_revision,
        "sdk_features": features, "toolchain": toolchain, "observations": observations})
    return 0 if all(item["exit_code"] == 0 for case in observations for item in case["executions"]) else 1


def verify(args):
    require(args.candidate.is_dir() and not args.candidate.is_symlink(), "invalid candidate root")
    report = decode(args.candidate / "sdk-release-quality.json")
    require(set(report) == {"schema_version", "gate", "identity", "base_revision", "sdk_features",
                            "toolchain", "observations"}, "unexpected report fields")
    require(report["schema_version"] == 1 and report["gate"] == "NS20", "unsupported report")
    require(report["identity"] == identity(args), "wrong source runner identity")
    require(report["base_revision"] == args.base_revision, "untrusted compatibility baseline")
    manifest = tomllib.loads(args.sdk_manifest.read_text(encoding="utf-8"))
    require(report["sdk_features"] == sorted(manifest["features"]), "source feature inventory was narrowed")
    require(report["toolchain"].startswith("rustc 1.97.1 "), "wrong release toolchain")
    commands = plan(report["sdk_features"], report["base_revision"])
    require(len(report["observations"]) == len(CASES), "incomplete or duplicate NS20 inventory")
    expected_files = {"sdk-release-quality.json"}
    for case, observation in zip(CASES, report["observations"], strict=True):
        require(set(observation) == {"case", "executions"} and observation["case"] == case,
                "wrong NS20 case")
        require(len(observation["executions"]) == len(commands[case]), "missing feature or check")
        for index, (item, command) in enumerate(zip(observation["executions"], commands[case], strict=True)):
            require(set(item) == {"argv", "exit_code", "log", "log_sha256"}, "unexpected execution fields")
            require(item["argv"] == command and type(item["exit_code"]) is int and item["exit_code"] == 0,
                    "check failed, skipped or substituted")
            filename = f"{case}-{index}.log"
            require(item["log"] == filename, "invalid log path")
            path = args.candidate / filename
            require(path.is_file() and not path.is_symlink() and 0 < path.stat().st_size <= MAX_FILE,
                    "missing, empty or oversized execution log")
            require(digest(path.read_bytes()) == item["log_sha256"], "execution log changed")
            if command[:2] == ["cargo", "test"]:
                output = path.read_text(encoding="utf-8", errors="replace")
                require(len(re.findall(r"test result: ok\. [1-9][0-9]* passed; 0 failed;", output))
                        >= command.count("--test"),
                        "snapshot or consumer harness executed no passing tests")
            expected_files.add(filename)
    require({path.name for path in args.candidate.iterdir()} == expected_files, "unexpected candidate files")
    print(json.dumps({"schema_version": 1, "gate": "NS20", "effective_state": "partial",
                      "cases": list(CASES), "source_revision": args.source_revision}, sort_keys=True))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    produce = commands.add_parser("run")
    produce.add_argument("--output", type=Path, required=True)
    produce.add_argument("--base-revision", required=True)
    admit = commands.add_parser("verify")
    admit.add_argument("--candidate", type=Path, required=True)
    admit.add_argument("--base-revision", required=True)
    admit.add_argument("--sdk-manifest", type=Path, required=True)
    for command in (produce, admit):
        command.add_argument("--source-revision", required=True)
        command.add_argument("--repository", required=True)
        command.add_argument("--run-id", required=True)
        command.add_argument("--run-attempt", type=int, required=True)
    args = parser.parse_args()
    try:
        return run(args) if args.command == "run" else verify(args)
    except (ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f"SDK release quality rejected: {error}\n")


if __name__ == "__main__":
    sys.exit(main())
