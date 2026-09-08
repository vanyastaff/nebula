use std::{fs, path::PathBuf};

use serde_json::{Value, json};

use super::*;

fn input() -> Value {
    json!({"source_revision":"a".repeat(40)})
}

fn ci() -> Value {
    json!({"repository":"fixture/repository","workflow_path":".github/workflows/test-matrix.yml","job_id":"postgres-conformance","run_id":"1234","run_attempt":1})
}

/// The verifying runner, agreeing with `input()` and `ci()`.
fn runner() -> RunnerIdentity {
    RunnerIdentity {
        source_revision: "a".repeat(40),
        repository: "fixture/repository".to_owned(),
        run_id: "1234".to_owned(),
        run_attempt: 1,
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    root: PathBuf,
    expected: Value,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("observations");
        fs::create_dir(&root).unwrap();
        let mut artifacts = Vec::new();
        // This is synthetic verifier input only, never a runtime gate report.
        for (identity, required) in policy().unwrap() {
            let path = artifact_path(&identity, &required).unwrap();
            let backend = identity
                .backend
                .map_or(Value::Null, |backend| json!(<&str>::from(backend)));
            let identity = json!({"gate":identity.gate,"backend":backend});
            let environment = json!({"toolchain":"synthetic-toolchain-version"});
            let observations: Vec<_> = required
                .cases
                .into_iter()
                .map(|case| json!({"case":case,"events":[{"synthetic_fixture":true}]}))
                .collect();
            let artifact = json!({"observation_version":1,"verifier_policy_sha256":loader::digest(POLICY_SOURCE.as_bytes()),"identity":identity,"input":input(),"ci":ci(),"environment":environment,"observations":observations});
            let bytes = serde_json::to_vec(&artifact).unwrap();
            let artifact_path = root.join(&path);
            fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
            fs::write(&artifact_path, &bytes).unwrap();
            artifacts.push(
                json!({"identity":identity,"path":path,"sha256":loader::digest(&bytes),"ci":ci(),"environment":environment}),
            );
        }
        Self {
            directory,
            root,
            expected: json!({"provenance_version":1,"registry_sha256":loader::digest(REGISTRY.as_bytes()),"verifier_policy_sha256":loader::digest(POLICY_SOURCE.as_bytes()),"input":input(),"artifacts":artifacts}),
        }
    }

    fn admit(&self) -> Result<(), VerificationError> {
        self.admit_as(&runner())
    }

    fn with_verified_observations() -> Self {
        let mut fixture = Self::new();
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let artifact_count = fixture.expected["artifacts"].as_array().unwrap().len();
        for index in 0..artifact_count {
            let identity: GateBackend =
                serde_json::from_value(fixture.expected["artifacts"][index]["identity"].clone())
                    .unwrap();
            fixture.mutate_artifact(index, |artifact| {
                for observation in artifact["observations"].as_array_mut().unwrap() {
                    let case = observation["case"].as_str().map(str::to_owned);
                    observation["events"] = json!([semantic::fixtures::for_observation(
                        &workspace,
                        &identity,
                        case.as_deref(),
                    )]);
                }
            });
        }
        fixture
    }

    fn expected_path(&self) -> PathBuf {
        let expected_path = self.directory.path().join("trusted.json");
        fs::write(&expected_path, serde_json::to_vec(&self.expected).unwrap()).unwrap();
        expected_path
    }

    fn admit_as(&self, runner: &RunnerIdentity) -> Result<(), VerificationError> {
        let expected = json::decode(&serde_json::to_vec(&self.expected).unwrap())?;
        admit_artifacts(&self.root, &expected, runner)
    }

    fn mutate_artifact(&mut self, index: usize, mutate: impl FnOnce(&mut Value)) {
        let entry = &mut self.expected["artifacts"][index];
        let path = self.root.join(entry["path"].as_str().unwrap());
        let mut artifact = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        mutate(&mut artifact);
        let bytes = serde_json::to_vec(&artifact).unwrap();
        fs::write(path, &bytes).unwrap();
        entry["sha256"] = loader::digest(&bytes).into();
    }

    fn artifact_index(&self, gate: ExternalGateId, backend: Option<Backend>) -> usize {
        self.expected["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .position(|entry| {
                entry["identity"]["gate"] == json!(gate)
                    && entry["identity"]["backend"]
                        == backend.map_or(Value::Null, |value| json!(<&str>::from(value)))
            })
            .unwrap()
    }
}

#[test]
fn compiled_policy_preserves_issue_scope_and_exact_backend_case_denominator() {
    let required = policy().unwrap();
    assert_eq!(required.len(), 25);
    assert_eq!(
        required.keys().map(|key| key.gate).collect::<BTreeSet<_>>(),
        RuntimeAuthorityGate::ALL
            .iter()
            .copied()
            .map(ExternalGateId::from)
            .collect()
    );
    for backend in [Backend::InMemory, Backend::Sqlite, Backend::Postgresql] {
        assert_eq!(
            required[&GateBackend {
                gate: ExternalGateId::PersistenceConformance,
                backend: Some(backend)
            }]
                .cases,
            [
                "checkpoint-reconnect",
                "owner-fencing",
                "atomic-transition",
                "tenant-isolation",
                "keyed-acceptance",
                "publication-atomicity",
                "lease-recovery",
                "claim-generation-fencing",
                "exact-revision-routing",
                "backend-reinitialization",
                "remote-effects",
            ]
            .into_iter()
            .map(|name| Some(name.to_owned()))
            .collect()
        );
    }
    for backend in [Backend::Sqlite, Backend::Postgresql] {
        assert_eq!(
            required[&GateBackend {
                gate: ExternalGateId::OrderedMigrations,
                backend: Some(backend)
            }]
                .cases,
            BTreeSet::from([
                Some("clean".into()),
                Some("previous-supported-version".into())
            ])
        );
    }
}

#[test]
fn checked_in_runtime_states_remain_a_conservative_baseline() {
    let registry: GateRegistry = toml::from_str(REGISTRY).unwrap();
    let expected = [
        (ExternalGateId::ExecutionIdentity, "red"),
        (ExternalGateId::ExactRevisionRouting, "red"),
        (ExternalGateId::ClaimGenerationFencing, "partial"),
        (ExternalGateId::KeyedAcceptance, "partial"),
        (ExternalGateId::ClaimHandoff, "red"),
        (ExternalGateId::PersistenceConformance, "partial"),
        (ExternalGateId::RequiredPostgresql, "partial"),
        (ExternalGateId::OrderedMigrations, "partial"),
        (ExternalGateId::ActivationDiagnostics, "missing"),
        (ExternalGateId::RemoteEffects, "missing"),
    ];

    for (gate_id, expected_state) in expected {
        let gate = registry
            .gates
            .iter()
            .find(|gate| gate.id == gate_id)
            .unwrap();
        assert_eq!(<&'static str>::from(gate.state), expected_state);
    }
}

#[test]
fn structurally_complete_synthetic_evidence_fails_semantic_policy() {
    let fixture = Fixture::new();
    assert_eq!(fixture.admit(), Ok(()));
    let expected_path = fixture.expected_path();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(
        verify(&workspace, &fixture.root, &expected_path, &runner()),
        Err(VerificationError::SemanticArtifact {
            gate: <&'static str>::from(ExternalGateId::ExecutionIdentity).to_owned(),
            backend: "in-memory".to_owned(),
            case: "default".to_owned(),
        })
    );
}

#[test]
fn failed_provenance_and_semantics_produce_no_derived_state_output() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

    let semantic_failure = Fixture::new();
    let semantic_expected = semantic_failure.expected_path();
    assert_eq!(
        verify(
            &workspace,
            &semantic_failure.root,
            &semantic_expected,
            &runner(),
        ),
        Err(VerificationError::SemanticArtifact {
            gate: <&'static str>::from(ExternalGateId::ExecutionIdentity).to_owned(),
            backend: "in-memory".to_owned(),
            case: "default".to_owned(),
        })
    );

    let provenance_failure = Fixture::with_verified_observations();
    let provenance_expected = provenance_failure.expected_path();
    assert_eq!(
        verify(
            &workspace,
            &provenance_failure.root,
            &provenance_expected,
            &RunnerIdentity {
                repository: "foreign/repository".to_owned(),
                ..runner()
            },
        ),
        Err(VerificationError::ProvenanceMismatch)
    );
}

#[test]
fn semantic_failure_identifies_a_named_sqlite_case() {
    let mut fixture = Fixture::with_verified_observations();
    let index = fixture.artifact_index(
        ExternalGateId::PersistenceConformance,
        Some(Backend::Sqlite),
    );
    fixture.mutate_artifact(index, |artifact| {
        let observation = artifact["observations"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|observation| observation["case"] == "backend-reinitialization")
            .unwrap();
        observation["events"] = json!([{}]);
    });
    let expected_path = fixture.expected_path();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

    assert_eq!(
        verify(&workspace, &fixture.root, &expected_path, &runner()),
        Err(VerificationError::SemanticArtifact {
            gate: <&'static str>::from(ExternalGateId::PersistenceConformance).to_owned(),
            backend: <&'static str>::from(Backend::Sqlite).to_owned(),
            case: "backend-reinitialization".to_owned(),
        })
    );
}

#[test]
fn semantic_failure_identifies_a_backend_independent_artifact() {
    let mut fixture = Fixture::with_verified_observations();
    let index = fixture.artifact_index(ExternalGateId::ActivationDiagnostics, None);
    fixture.mutate_artifact(index, |artifact| {
        artifact["observations"][0]["events"] = json!([{}]);
    });
    let expected_path = fixture.expected_path();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

    assert_eq!(
        verify(&workspace, &fixture.root, &expected_path, &runner()),
        Err(VerificationError::SemanticArtifact {
            gate: <&'static str>::from(ExternalGateId::ActivationDiagnostics).to_owned(),
            backend: "independent".to_owned(),
            case: "default".to_owned(),
        })
    );
}

#[test]
fn verified_artifacts_emit_the_exact_ordered_derived_state_set() {
    let fixture = Fixture::with_verified_observations();
    let expected_path = fixture.expected_path();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = verify(&workspace, &fixture.root, &expected_path, &runner()).unwrap();
    let summary: Value = serde_json::from_slice(&output).unwrap();
    let expected = RuntimeAuthorityGate::ALL
        .into_iter()
        .map(|gate| json!({"gate": ExternalGateId::from(gate), "state": "partial"}))
        .collect::<Vec<_>>();

    assert_eq!(
        summary,
        json!({
            "status": "verified",
            "contract": "runtime-authority",
            "derived_states": Value::Array(expected),
        })
    );
    assert_eq!(output.last(), Some(&b'\n'));
}

#[test]
fn every_required_artifact_rejects_omission_duplicate_or_extra() {
    let mut fixture = Fixture::new();
    let original = fixture.expected["artifacts"].as_array().unwrap().clone();
    for index in 0..original.len() {
        let mut missing = original.clone();
        missing.remove(index);
        fixture.expected["artifacts"] = missing.into();
        assert_eq!(fixture.admit(), Err(VerificationError::InventoryMismatch));
        let mut duplicate = original.clone();
        duplicate[index] = original[(index + 1) % original.len()].clone();
        fixture.expected["artifacts"] = duplicate.into();
        assert_eq!(fixture.admit(), Err(VerificationError::InventoryMismatch));
    }
    let mut extra = original;
    extra.push(extra[0].clone());
    fixture.expected["artifacts"] = extra.into();
    assert_eq!(fixture.admit(), Err(VerificationError::InventoryMismatch));
}

#[test]
fn observations_reject_omissions_duplicates_unknown_cases_and_empty_events() {
    for change in ["missing", "duplicate", "unknown", "empty"] {
        let mut fixture = Fixture::new();
        fixture.mutate_artifact(0, |artifact| {
            let observations = artifact["observations"].as_array_mut().unwrap();
            match change {
                "missing" => observations.clear(),
                "duplicate" => observations.push(observations[0].clone()),
                "unknown" => observations[0]["case"] = "invented".into(),
                "empty" => observations[0]["events"] = json!([]),
                _ => unreachable!(),
            }
        });
        let expected = if change == "empty" {
            VerificationError::InvalidObservation
        } else {
            VerificationError::InventoryMismatch
        };
        assert_eq!(fixture.admit(), Err(expected), "{change}");
    }
}

#[test]
fn artifact_provenance_is_compared_to_independent_expected_values() {
    for path in [
        "/input/source_revision",
        "/ci/run_id",
        "/ci/workflow_path",
        "/ci/job_id",
        "/ci/repository",
        "/environment/toolchain",
        "/verifier_policy_sha256",
    ] {
        let mut fixture = Fixture::new();
        fixture.mutate_artifact(0, |artifact| {
            *artifact.pointer_mut(path).unwrap() = "different".into();
        });
        assert_eq!(
            fixture.admit(),
            Err(VerificationError::ProvenanceMismatch),
            "{path}"
        );
    }
    let mut fixture = Fixture::new();
    fixture.mutate_artifact(0, |artifact| artifact["ci"]["run_attempt"] = 2.into());
    assert_eq!(fixture.admit(), Err(VerificationError::ProvenanceMismatch));
}

#[test]
fn versions_digest_and_trusted_policy_cannot_be_overridden() {
    let mut fixture = Fixture::new();
    fixture.expected["provenance_version"] = 2.into();
    assert_eq!(fixture.admit(), Err(VerificationError::UnsupportedVersion));
    fixture.expected["provenance_version"] = 1.into();
    fixture.mutate_artifact(0, |artifact| artifact["observation_version"] = 2.into());
    assert_eq!(fixture.admit(), Err(VerificationError::UnsupportedVersion));
    fixture.expected["registry_sha256"] = "b".repeat(64).into();
    assert_eq!(fixture.admit(), Err(VerificationError::PolicyMismatch));
    let mut fixture = Fixture::new();
    fixture.expected["artifacts"][0]["sha256"] = "b".repeat(64).into();
    assert_eq!(fixture.admit(), Err(VerificationError::ArtifactDigest));
}

#[test]
fn duplicate_json_keys_at_any_depth_and_trailing_values_are_rejected() {
    for bytes in [
        br#"{"a":1,"a":2}"#.as_slice(),
        br#"{"outer":[{"a":1,"a":2}]}"#,
        br#"{"a":1,"\u0061":2}"#,
        b"{} {}",
    ] {
        assert_eq!(
            json::decode::<Value>(bytes),
            Err(VerificationError::InvalidJson)
        );
    }
}

#[test]
fn json_depth_collection_and_string_limits_are_enforced() {
    for document in [
        format!("{}0{}", "[".repeat(18), "]".repeat(18)),
        serde_json::to_string(&vec![0; 257]).unwrap(),
        serde_json::to_string(&"x".repeat(4097)).unwrap(),
    ] {
        assert_eq!(
            json::decode::<Value>(document.as_bytes()),
            Err(VerificationError::InvalidJson)
        );
    }
}

#[test]
fn artifact_paths_are_confined_and_files_are_bounded_before_parsing() {
    let fixture = Fixture::new();
    for path in [
        "../outside.json",
        "/etc/passwd",
        "./0.json",
        "a//b.json",
        "a\\b.json",
        "",
    ] {
        assert_eq!(
            loader::artifact(&fixture.root, path, &"0".repeat(64)),
            Err(VerificationError::ArtifactPath)
        );
    }
    assert_eq!(
        loader::bounded_file(&fixture.root),
        Err(VerificationError::ArtifactPath)
    );
    let oversized = fixture.root.join("large.json");
    fs::File::create(&oversized)
        .unwrap()
        .set_len((loader::MAX_FILE_BYTES + 1) as u64)
        .unwrap();
    assert_eq!(
        loader::bounded_file(&oversized),
        Err(VerificationError::ArtifactSize)
    );
}

#[cfg(unix)]
#[test]
fn symlink_artifacts_and_linked_directories_are_rejected() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let outside = fixture.directory.path().join("outside.json");
    fs::write(&outside, b"{}").unwrap();
    symlink(&outside, fixture.root.join("link.json")).unwrap();
    symlink(fixture.directory.path(), fixture.root.join("linked")).unwrap();
    for path in ["link.json", "linked/outside.json"] {
        assert_eq!(
            loader::artifact(&fixture.root, path, &loader::digest(b"{}")),
            Err(VerificationError::ArtifactPath)
        );
    }
}

#[test]
fn provenance_manifest_must_be_separate_from_the_artifact_root() {
    let fixture = Fixture::new();
    let path = fixture.root.join("self-asserted.json");
    fs::write(&path, serde_json::to_vec(&fixture.expected).unwrap()).unwrap();
    assert_eq!(
        verify(Path::new("."), &fixture.root, &path, &runner()),
        Err(VerificationError::ProvenancePath)
    );
}

/// Every runtime gate lists the `#tests` aggregation job in `required_ci`, and
/// that job runs no tests — so set membership alone let an artifact claim it
/// came from there. Only the job that actually produces the bundle qualifies.
#[test]
fn only_the_producing_job_may_be_named_as_the_source_of_an_artifact() {
    let policy = RequiredArtifact {
        cases: BTreeSet::from([None]),
        ci_jobs: [
            ".github/workflows/test-matrix.yml#tests",
            ".github/workflows/test-matrix.yml#postgres-conformance",
            ".github/workflows/test-matrix.yml#runtime-authority",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        artifact_stem: "keyed-acceptance".to_owned(),
    };
    let job = |id: &str| CiProvenance {
        repository: "fixture/repository".to_owned(),
        workflow_path: ".github/workflows/test-matrix.yml".to_owned(),
        job_id: id.to_owned(),
        run_id: "1234".to_owned(),
        run_attempt: 1,
    };

    assert_eq!(validate_ci(&job("postgres-conformance"), &policy), Ok(()));
    for impostor in ["tests", "runtime-authority", "select-matrix"] {
        assert_eq!(
            validate_ci(&job(impostor), &policy),
            Err(VerificationError::ProvenanceMismatch),
            "job `{impostor}` must not qualify as the producer"
        );
    }
}

/// Provenance is only a trust anchor if the verifying job compares it to its
/// own identity. Before this check every field was merely shape-validated, so
/// a bundle naming any well-formed revision, repository, or run verified.
#[test]
fn provenance_naming_another_revision_repository_or_run_is_rejected() {
    let fixture = Fixture::new();
    assert_eq!(fixture.admit(), Ok(()));
    for foreign in [
        RunnerIdentity {
            source_revision: "b".repeat(40),
            ..runner()
        },
        RunnerIdentity {
            repository: "attacker/repository".to_owned(),
            ..runner()
        },
        RunnerIdentity {
            run_id: "9999".to_owned(),
            ..runner()
        },
        RunnerIdentity {
            run_attempt: 2,
            ..runner()
        },
    ] {
        assert_eq!(
            fixture.admit_as(&foreign),
            Err(VerificationError::ProvenanceMismatch)
        );
    }
}

#[test]
fn every_declared_conformance_case_is_mandatory_even_when_other_cases_exist() {
    for case in [
        "checkpoint-reconnect",
        "owner-fencing",
        "atomic-transition",
        "tenant-isolation",
        "keyed-acceptance",
        "publication-atomicity",
        "lease-recovery",
        "claim-generation-fencing",
        "exact-revision-routing",
        "backend-reinitialization",
        "remote-effects",
    ] {
        let mut fixture = Fixture::new();
        let index = fixture.expected["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .position(|entry| {
                entry["identity"]["gate"]
                    == serde_json::to_value(ExternalGateId::PersistenceConformance).unwrap()
            })
            .unwrap();
        fixture.mutate_artifact(index, |artifact| {
            artifact["observations"]
                .as_array_mut()
                .unwrap()
                .retain(|observation| observation["case"] != case);
        });
        assert_eq!(fixture.admit(), Err(VerificationError::InventoryMismatch));
    }
}

#[test]
fn submitted_pass_summaries_are_not_observation_artifacts() {
    let mut fixture = Fixture::new();
    fixture.mutate_artifact(0, |artifact| artifact["passed"] = true.into());
    assert_eq!(fixture.admit(), Err(VerificationError::InvalidJson));
    fixture.mutate_artifact(0, |artifact| {
        artifact.as_object_mut().unwrap().remove("passed");
    });
}

#[test]
fn sha256_encoding_matches_a_known_vector() {
    assert_eq!(
        loader::digest(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn explicit_null_backend_and_case_cannot_be_omitted() {
    assert!(matches!(
        json::decode::<GateBackend>(
            format!(
                r#"{{"gate":"{}"}}"#,
                <&'static str>::from(ExternalGateId::ActivationDiagnostics)
            )
            .as_bytes()
        ),
        Err(VerificationError::InvalidJson)
    ));
    let mut fixture = Fixture::new();
    fixture.mutate_artifact(0, |artifact| {
        artifact["observations"][0]
            .as_object_mut()
            .unwrap()
            .remove("case");
    });
    assert_eq!(fixture.admit(), Err(VerificationError::InvalidJson));
}

#[test]
fn required_execution_environment_and_policy_provenance_cannot_be_omitted() {
    for (object, field) in [("ci", "repository"), ("environment", "toolchain")] {
        let mut fixture = Fixture::new();
        fixture.mutate_artifact(0, |artifact| {
            artifact[object].as_object_mut().unwrap().remove(field);
        });
        assert_eq!(fixture.admit(), Err(VerificationError::InvalidJson));
    }
    let mut fixture = Fixture::new();
    fixture.mutate_artifact(0, |artifact| {
        artifact
            .as_object_mut()
            .unwrap()
            .remove("verifier_policy_sha256");
    });
    assert_eq!(fixture.admit(), Err(VerificationError::InvalidJson));
    fixture
        .expected
        .as_object_mut()
        .unwrap()
        .remove("verifier_policy_sha256");
    assert_eq!(fixture.admit(), Err(VerificationError::InvalidJson));
}
