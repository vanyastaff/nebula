//! Compile contract for the supported SDK perimeter.
//!
//! The fixture has exactly one Nebula dependency. Its positive binary exercises
//! typed action authoring (`ActionMetadataDraft`, `simple_action!`, and associated
//! `Input`/`Output`), `WorkflowBuilder`, credential `TestResult`, and manual `Provider` authoring
//! with a consuming terminal hook over a non-Clone instance, using the
//! general-purpose `async-trait` dependency. This is a compile check, not runtime
//! teardown coverage. A second positive binary checks custom resource topology
//! authoring. Each negative binary targets one distinct authority or persistence
//! escape hatch that must stay unavailable, including paths below `__private`:
//! Rust documentation hiding is not access control. Procedural derives have a
//! separate SDK-only compile-pass
//! contract in `derive_external_contract.rs`.

use std::{
    ffi::OsString,
    fs,
    path::Path,
    process::{Command, Output},
};

const FIXTURE_FILES: &[&str] = &[
    "Cargo.toml",
    "src/bin/positive.rs",
    "src/bin/resource_topology.rs",
    "src/bin/resource_manager.rs",
    "src/bin/resource_registry.rs",
    "src/bin/resource_release_queue.rs",
    "src/bin/authority_constructor.rs",
    "src/bin/owner_selector.rs",
    "src/bin/raw_writer.rs",
    "src/bin/admin_repository.rs",
    "src/bin/runtime_constructor.rs",
    "src/bin/unscoped_resolver.rs",
    "src/bin/operation_protocol.rs",
    "src/bin/turn_handoff.rs",
    "src/bin/removed_field_values.rs",
    "src/bin/removed_field_value.rs",
    "src/bin/compiled_program.rs",
    "src/bin/compiled_value.rs",
    "src/bin/engine_expression_context.rs",
    "src/bin/eval_future.rs",
    "src/bin/expression_context.rs",
    "src/bin/action_input.rs",
    "src/bin/prepared_action_input.rs",
    "src/bin/base_metadata.rs",
    "src/bin/action_metadata.rs",
    "src/bin/credential_metadata.rs",
    "src/bin/resource_metadata.rs",
    "src/bin/metadata_trait.rs",
    "src/bin/metadata_build_error.rs",
    "src/bin/value_tree.rs",
    "src/bin/valid_values.rs",
    "src/bin/resolved_value.rs",
    "src/bin/resolved_values.rs",
    "src/bin/resolved_lookup.rs",
    "src/bin/credential_record.rs",
    "src/bin/stateless_action_adapter.rs",
    "src/bin/resource_registration_spec.rs",
    "src/bin/resource_acquire_options.rs",
    "src/bin/resource_slot_identity.rs",
    "src/bin/remote_effect_factory.rs",
    "src/bin/action_builder.rs",
    "src/bin/hidden_resource_manager.rs",
    "src/bin/hidden_resource_factory.rs",
    "src/bin/hidden_resource_kind_activator.rs",
    "src/bin/hidden_resource_register_request.rs",
    "src/bin/hidden_resource_box_fut.rs",
    "src/bin/hidden_resource_metadata.rs",
    "src/bin/hidden_resource_metadata_build_error.rs",
    "src/bin/hidden_resource_slot_identity.rs",
    "src/bin/hidden_resource_bridge_field.rs",
];

const FORBIDDEN: &[(&str, &str)] = &[
    ("resource_manager", "Manager"),
    ("resource_registry", "Registry"),
    ("resource_release_queue", "ReleaseQueue"),
    ("authority_constructor", "Principal"),
    ("owner_selector", "CredentialOwner"),
    ("raw_writer", "CredentialPersistence"),
    ("admin_repository", "OwnerScopedCredentialRepository"),
    ("runtime_constructor", "CredentialService"),
    ("unscoped_resolver", "CredentialResolver"),
    ("operation_protocol", "PreparedEffectContract"),
    ("turn_handoff", "ExecutionTurnHandoff"),
    ("removed_field_values", "FieldValues"),
    ("removed_field_value", "FieldValue"),
    ("compiled_program", "CompiledProgram"),
    ("compiled_value", "CompiledValue"),
    ("engine_expression_context", "EngineExpressionContext"),
    ("eval_future", "EvalFuture"),
    ("expression_context", "ExpressionContext"),
    ("action_input", "ActionInput"),
    ("prepared_action_input", "PreparedActionInput"),
    ("base_metadata", "BaseMetadata"),
    ("action_metadata", "ActionMetadata"),
    ("credential_metadata", "CredentialMetadata"),
    ("resource_metadata", "ResourceMetadata"),
    ("metadata_trait", "Metadata"),
    ("metadata_build_error", "MetadataBuildError"),
    ("value_tree", "ValueTree"),
    ("valid_values", "ValidValues"),
    ("resolved_value", "ResolvedValue"),
    ("resolved_values", "ResolvedValues"),
    ("resolved_lookup", "ResolvedLookup"),
    ("credential_record", "CredentialRecord"),
    ("stateless_action_adapter", "StatelessActionAdapter"),
    ("resource_registration_spec", "RegistrationSpec"),
    ("resource_acquire_options", "AcquireOptions"),
    ("resource_slot_identity", "SlotIdentity"),
    ("remote_effect_factory", "RemoteEffectFactory"),
    ("action_builder", "ActionBuilder"),
    ("hidden_resource_manager", "Manager"),
    ("hidden_resource_factory", "ResourceFactory"),
    ("hidden_resource_kind_activator", "factory"),
    ("hidden_resource_register_request", "factory"),
    ("hidden_resource_box_fut", "factory"),
    ("hidden_resource_metadata", "ResourceMetadata"),
    ("hidden_resource_metadata_build_error", "MetadataBuildError"),
    ("hidden_resource_slot_identity", "SlotIdentity"),
];

const OPAQUE: &[(&str, &str)] = &[("hidden_resource_bridge_field", "factory")];

#[test]
fn sdk_only_consumer_cannot_name_authority_or_raw_persistence() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture_dir = manifest_dir.join("tests/fixtures/public_perimeter_consumer");
    let fixture_manifest = fs::read_to_string(fixture_dir.join("Cargo.toml"))
        .expect("read committed public-perimeter manifest");

    let nebula_dependencies = fixture_manifest
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("nebula-") && line.contains('='))
        .collect::<Vec<_>>();
    assert_eq!(
        nebula_dependencies,
        [format!(
            "nebula-sdk = {{ version = \"={}\", default-features = false }}",
            env!("CARGO_PKG_VERSION")
        )
        .as_str()],
        "fixture must have exactly one Nebula dependency: nebula-sdk"
    );

    let temp = tempfile::tempdir().expect("create isolated public-perimeter directory");
    copy_fixture(&fixture_dir, temp.path());

    let sdk_path = manifest_dir
        .canonicalize()
        .expect("canonicalize local nebula-sdk path");
    let mut patched_manifest = fixture_manifest;
    patched_manifest.push_str(&format!(
        "\n[patch.crates-io]\nnebula-sdk = {{ path = \"{}\" }}\n",
        toml_basic_string(&sdk_path)
    ));
    fs::write(temp.path().join("Cargo.toml"), patched_manifest)
        .expect("write patched public-perimeter manifest");

    let workspace_root = manifest_dir.join("../..");
    fs::copy(
        workspace_root.join("Cargo.lock"),
        temp.path().join("Cargo.lock"),
    )
    .expect("copy workspace lockfile into public-perimeter fixture");

    for &(binary, forbidden_segment) in FORBIDDEN {
        let output = cargo_check(temp.path(), binary);
        assert!(
            !output.status.success(),
            "forbidden perimeter probe `{binary}` unexpectedly compiled"
        );
        let diagnostics = compiler_errors(&output);
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.message.contains(forbidden_segment)
                    && diagnostic.highlighted == forbidden_segment
                    && (diagnostic.message.contains("could not find")
                        || diagnostic.message.contains("cannot find")
                        || diagnostic.message.contains("unresolved import"))
            }),
            "probe `{binary}` did not produce an exact missing-leaf diagnostic for \
             `{forbidden_segment}`; diagnostics: {diagnostics:#?}\n{}",
            render_output(&output)
        );

        let rendered = render_output(&output);
        let lower = rendered.to_ascii_lowercase();
        for unrelated in [
            "no matching package named",
            "failed to get `nebula-sdk`",
            "failed to load source for dependency",
            "could not find `integration` in `nebula_sdk`",
            "could not find `credential` in `integration`",
        ] {
            assert!(
                !lower.contains(unrelated),
                "probe `{binary}` failed for unrelated reason `{unrelated}`:\n{}",
                render_output(&output)
            );
        }
    }

    for &(binary, private_segment) in OPAQUE {
        let output = cargo_check(temp.path(), binary);
        assert!(
            !output.status.success(),
            "opaque bridge probe `{binary}` unexpectedly compiled"
        );
        let diagnostics = compiler_errors(&output);
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.message.contains(private_segment)
                    && diagnostic.highlighted == private_segment
                    && diagnostic.message.contains("private")
            }),
            "probe `{binary}` did not produce an exact private-member diagnostic for \
             `{private_segment}`; diagnostics: {diagnostics:#?}\n{}",
            render_output(&output)
        );
    }

    let positive = cargo_check(temp.path(), "positive");
    assert!(
        positive.status.success(),
        "supported SDK authoring path must compile:\n{}",
        render_output(&positive)
    );
    let topology = cargo_check(temp.path(), "resource_topology");
    assert!(
        topology.status.success(),
        "custom topology authoring must compile:\n{}",
        render_output(&topology)
    );
}

#[test]
fn macro_private_surface_matches_the_explicit_allowlist() {
    const EXPECTED: &str = r#"
        pub mod __private {
            pub mod action {
                pub use nebula_action::{
                    Action, ActionContext, ActionContextExt, ActionError, ActionMetadataDraft,
                    ActionResult, FromWorkflowNode, MetadataVersion, StatelessAction, metadata_name,
                };
            }
            pub mod core {
                pub use nebula_core::{
                    ActionKey, CredentialKey, DeclaresDependencies, Dependencies, ResourceKey,
                    SlotField, SlotKind, action_key,
                };
                pub mod auth {
                    pub use nebula_core::auth::{
                        AuthPattern, AuthScheme, ExternalScheme, PublicScheme, SensitiveScheme,
                    };
                }
                pub mod sync {
                    pub use nebula_core::sync::Lazy;
                }
            }
            pub mod credential {
                pub use nebula_credential::{
                    AuthScheme, Credential, CredentialGuard, CredentialLifecycle,
                    CredentialMetadataDraft, CredentialPolicy, CredentialState, Dynamic, Interactive,
                    RefreshStrategy, Refreshable, Revocable, RevokeStrategy, Testable,
                    credential_key, metadata_name, schema_of,
                };
                pub mod contract {
                    pub mod plugin_capability_report {
                        pub use nebula_credential::contract::plugin_capability_report::{
                            IsDynamic, IsInteractive, IsRefreshable, IsRevocable, IsTestable,
                        };
                    }
                }
            }
            pub mod plugin {
                pub use nebula_plugin::{Plugin, PluginManifest};
            }
            pub mod resource {
                pub use nebula_resource::{Error, HasCredentialSlots, ResourceConfig};
                pub mod contribution {
                    pub use crate::resource_contribution::{
                        ResourceContribution, ResourceContributionBridge,
                    };
                }
                #[expect(
                    clippy::module_inception,
                    reason = "the hidden macro ABI mirrors nebula_resource::resource paths emitted by existing derives"
                )]
                pub mod resource {
                    pub use nebula_resource::resource::Provider;
                }
                pub mod topology {
                    pub use nebula_resource::topology::{Pooled, Resident, Topology};
                    pub mod pooled {
                        pub mod config {
                            pub use nebula_resource::topology::pooled::config::Config;
                        }
                    }
                    pub mod resident {
                        pub mod config {
                            pub use nebula_resource::topology::resident::config::Config;
                        }
                    }
                }
            }
            pub mod schema {
                pub use nebula_schema::{
                    AuthoredValue, ExpressionMode, Field, FieldKey, HasSchema, HasSelectOptions, InputHint,
                    RootShape, Rule, ScalarSchema, Schema, SelectOption, SerdeTagging, StringWidget,
                    ValidSchema, ValidationError, ValidationReport,
                };
                pub mod error {
                    pub use nebula_schema::error::ValidationReport;
                }
                pub mod __private {
                    pub use nebula_schema::__private::{serde_json, tracing, union_newtype_payload};
                }
            }
            pub mod validator {
                pub use nebula_validator::validators;
                pub mod combinators {
                    pub use nebula_validator::combinators::{SelfValidating, and, nested_validator, or};
                }
                pub mod foundation {
                    pub use nebula_validator::foundation::{Validate, ValidationError, ValidationErrors};
                }
                pub mod __private {
                    pub mod regex {
                        pub use nebula_validator::__private::regex::Regex;
                    }
                }
            }
            pub mod workflow {
                pub use nebula_workflow::NodeDefinition;
            }
            pub use semver;
        }
    "#;

    let source = include_str!("../src/lib.rs");
    let start = source
        .find("pub mod __private {")
        .expect("SDK macro-private module exists");
    let tail = &source[start..];
    let end = tail
        .find("\npub mod action;")
        .expect("macro-private module precedes public persona modules");
    assert_eq!(
        normalize_private_surface(&tail[..end]),
        normalize_private_surface(EXPECTED),
        "any macro-private export requires an explicit perimeter review"
    );
}

#[derive(Debug)]
struct CompilerError {
    message: String,
    highlighted: String,
}

fn compiler_errors(output: &Output) -> Vec<CompilerError> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| {
            value.get("reason").and_then(serde_json::Value::as_str) == Some("compiler-message")
        })
        .filter_map(|value| {
            let diagnostic = value.get("message")?;
            if diagnostic.get("level")?.as_str()? != "error" {
                return None;
            }
            let message = diagnostic.get("message")?.as_str()?.to_owned();
            let span = diagnostic.get("spans")?.as_array()?.iter().find(|span| {
                span.get("is_primary").and_then(serde_json::Value::as_bool) == Some(true)
            })?;
            let text = span.get("text")?.as_array()?.first()?;
            let source = text.get("text")?.as_str()?;
            let start = text.get("highlight_start")?.as_u64()?.checked_sub(1)? as usize;
            let end = text.get("highlight_end")?.as_u64()?.checked_sub(1)? as usize;
            let highlighted = source
                .chars()
                .skip(start)
                .take(end.saturating_sub(start))
                .collect();
            Some(CompilerError {
                message,
                highlighted,
            })
        })
        .collect()
}

fn normalize_private_surface(source: &str) -> String {
    source
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty() && !line.starts_with("///") && !line.starts_with("#[doc(hidden)]")
        })
        .flat_map(str::split_whitespace)
        .collect::<String>()
}

fn copy_fixture(source_root: &Path, destination_root: &Path) {
    for relative in FIXTURE_FILES {
        let source = source_root.join(relative);
        let destination = destination_root.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).expect("create temporary fixture directory");
        }
        fs::copy(source, destination).expect("copy committed fixture into temporary directory");
    }
}

fn cargo_check(fixture_root: &Path, binary: &str) -> Output {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    Command::new(cargo)
        .current_dir(fixture_root)
        .args([
            "check",
            "--offline",
            "--quiet",
            "--message-format=json",
            "--bin",
            binary,
        ])
        .env("CARGO_TERM_COLOR", "never")
        .env("CARGO_TARGET_DIR", fixture_root.join("target"))
        .output()
        .expect("run cargo check for external SDK perimeter consumer")
}

fn toml_basic_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn render_output(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
