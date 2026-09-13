use nebula_schema::{
    AddressSpaceCommitment, DeclarationAddress, DeclarationUse, DefinitionKey, DefinitionMemberKey,
    MAX_GRAPH_DEFINITIONS, MAX_GRAPH_DIAGNOSTICS, MAX_GRAPH_DOCUMENT_BYTES,
    MAX_GRAPH_IDENTIFIER_BYTES, MAX_GRAPH_REFERENCES, SchemaAdmissionError, SchemaGraphDocument,
    SemanticCommitment,
};
use serde::Deserialize;
use serde_json::{Value, json};

fn document(definitions: Value, root: &str) -> SchemaGraphDocument {
    serde_json::from_value(json!({
        "version": 3,
        "root": { "target": root },
        "definitions": definitions,
    }))
    .expect("fixture is a bounded graph document")
}

fn scalar(key: &str, kind: &str) -> Value {
    json!({ "key": key, "body": { "kind": kind } })
}

fn property(key: &str, target: &str) -> Value {
    json!({
        "key": key,
        "target": target,
        "presence": "required",
        "null": "reject",
        "empty_string": "allow",
        "empty_collection": "allow",
        "expression": "forbidden",
        "rules": [],
        "transformers": [],
        "aliases": { "read": [], "write": null }
    })
}

fn optional_property(key: &str, target: &str) -> Value {
    let mut value = property(key, target);
    value["presence"] = json!("optional");
    value
}

fn record(key: &str, properties: Vec<Value>) -> Value {
    json!({ "key": key, "body": { "kind": "record", "properties": properties } })
}

fn admitted(document: SchemaGraphDocument) -> (SemanticCommitment, AddressSpaceCommitment) {
    let graph = document.admit().expect("fixture admits");
    (
        *graph.semantic_commitment(),
        *graph.address_space_commitment(),
    )
}

fn codes(error: &SchemaAdmissionError) -> Vec<&str> {
    error
        .report()
        .iter()
        .map(nebula_schema::ValidationError::code)
        .collect()
}

#[test]
fn commitments_are_insertion_order_invariant_and_alpha_rename_separates_identity() {
    let first = document(
        json!([
            record("root", vec![property("name", "text")]),
            scalar("text", "string")
        ]),
        "root",
    );
    let reordered = document(
        json!([
            scalar("text", "string"),
            record("root", vec![property("name", "text")])
        ]),
        "root",
    );
    let renamed = document(
        json!([
            scalar("leaf", "string"),
            record("entry", vec![property("name", "leaf")])
        ]),
        "entry",
    );
    let local_reordered = document(
        json!([
            record(
                "root",
                vec![property("zeta", "text"), property("alpha", "text")]
            ),
            scalar("text", "string")
        ]),
        "root",
    );
    let local_sorted = document(
        json!([
            scalar("text", "string"),
            record(
                "root",
                vec![property("alpha", "text"), property("zeta", "text")]
            )
        ]),
        "root",
    );

    let first = admitted(first);
    let reordered = admitted(reordered);
    let renamed = admitted(renamed);
    assert_eq!(first, reordered);
    assert_eq!(first.0, renamed.0);
    assert_ne!(first.1, renamed.1);
    assert_eq!(admitted(local_reordered), admitted(local_sorted));
}

#[test]
fn edge_roles_and_bfs_cycle_reuse_are_committed_exactly() {
    let alias = document(
        json!([
            json!({
                "key": "root",
                "body": { "kind": "alias", "alias": { "target": "text" } }
            }),
            scalar("text", "string")
        ]),
        "root",
    );
    let body = document(
        json!([
            record("root", vec![property("target", "text")]),
            scalar("text", "string")
        ]),
        "root",
    );
    assert_ne!(
        admitted(alias).0,
        admitted(body).0,
        "Alias and Property roles differ"
    );

    let cycle_a = document(
        json!([
            record(
                "root",
                vec![optional_property("next", "node"), property("value", "text")]
            ),
            record("node", vec![optional_property("back", "root")]),
            scalar("text", "string")
        ]),
        "root",
    );
    let cycle_b = document(
        json!([
            scalar("z", "string"),
            record("a", vec![optional_property("back", "r")]),
            record(
                "r",
                vec![property("value", "z"), optional_property("next", "a")]
            )
        ]),
        "r",
    );
    assert_eq!(admitted(cycle_a).0, admitted(cycle_b).0);
}

#[test]
fn admission_diagnostics_are_deterministic_for_structural_failures() {
    let cases = [
        (
            document(
                json!([scalar("root", "string"), scalar("root", "null")]),
                "root",
            ),
            "schema.graph.duplicate_definition",
        ),
        (
            document(
                json!([record("root", vec![property("p", "missing")])]),
                "root",
            ),
            "schema.graph.dangling_reference",
        ),
        (
            document(
                json!([scalar("root", "string"), scalar("unused", "null")]),
                "root",
            ),
            "schema.graph.unreachable_definition",
        ),
        (
            serde_json::from_value(json!({
                "version": 3,
                "root": { "target": "root", "null": "reject" },
                "definitions": [record("root", vec![property("self", "root")])]
            }))
            .expect("fixture is a bounded graph document"),
            "schema.graph.nonproductive_definition",
        ),
    ];

    for (document, expected) in cases {
        let error = document.admit().expect_err("fixture must be rejected");
        assert_eq!(codes(&error), [expected]);
    }
}

#[test]
fn typed_additional_property_with_dangling_target_reports_dangling_reference() {
    let graph = document(
        json!([{
            "key":"root",
            "body":{
                "kind":"record",
                "properties":[],
                "additional_properties":{"typed":{"target":"missing"}}
            }
        }]),
        "root",
    );

    let error = graph
        .admit()
        .expect_err("typed additional property target must resolve");
    assert_eq!(codes(&error), ["schema.graph.dangling_reference"]);
}

#[test]
fn diagnostic_budget_accepts_exact_limit_and_marks_one_over() {
    let graph = |dangling: usize| {
        let properties = (0..dangling)
            .map(|index| optional_property(&format!("p{index}"), &format!("missing{index}")))
            .collect();
        document(json!([record("root", properties)]), "root")
    };
    let exact = graph(MAX_GRAPH_DIAGNOSTICS)
        .admit()
        .expect_err("dangling references reject");
    assert_eq!(exact.report().len(), MAX_GRAPH_DIAGNOSTICS);
    assert!(
        exact
            .report()
            .iter()
            .all(|issue| issue.code() == "schema.graph.dangling_reference")
    );

    let over = graph(MAX_GRAPH_DIAGNOSTICS + 1)
        .admit()
        .expect_err("dangling references reject");
    assert_eq!(over.report().len(), MAX_GRAPH_DIAGNOSTICS);
    assert_eq!(
        over.report().iter().last().unwrap().code(),
        "schema.graph.diagnostic_limit"
    );
}

#[test]
fn validator_owned_rule_budget_rejection_has_a_stable_graph_diagnostic() {
    let oversized_rule = "x".repeat(16 * 1024 + 1);
    let graph = document(
        json!([{
            "key": "root",
            "body": {
                "kind": "string",
                "intrinsic_rules": [{"custom": oversized_rule}]
            }
        }]),
        "root",
    );
    let error = graph
        .admit()
        .expect_err("validator rule budget must reject");
    assert_eq!(codes(&error), ["schema.graph.invalid_rule"]);
}

#[test]
fn exact_number_bounds_do_not_round_large_integers_through_f64() {
    let graph = document(
        json!([{
            "key": "root",
            "body": {
                "kind": "number",
                "minimum": 9_007_199_254_740_993_u64,
                "maximum": 9_007_199_254_740_992.0
            }
        }]),
        "root",
    );
    assert_eq!(
        codes(&graph.admit().expect_err("exactly descending bounds reject")),
        ["schema.graph.invalid_bounds"]
    );
}

#[test]
fn facet_applicability_and_intrinsic_context_are_rejected_after_shape_checks() {
    let incompatible_transformer = document(
        json!([
            record(
                "root",
                vec![{
                    let mut property = property("enabled", "flag");
                    property["transformers"] = json!([{"kind":"trim"}]);
                    property
                }]
            ),
            scalar("flag", "boolean")
        ]),
        "root",
    );
    assert_eq!(
        codes(&incompatible_transformer.admit().unwrap_err()),
        ["schema.graph.inapplicable_facet"]
    );

    let contextual_intrinsic = document(
        json!([{
            "key": "root",
            "body": {
                "kind": "string",
                "intrinsic_rules": [{"custom":"requires_runtime_context"}]
            }
        }]),
        "root",
    );
    assert_eq!(
        codes(&contextual_intrinsic.admit().unwrap_err()),
        ["schema.graph.inapplicable_facet"]
    );

    let mut aliased_property = property("name", "text_alias");
    aliased_property["transformers"] = json!([{"kind":"trim"}]);
    let alias_to_string = document(
        json!([
            record("root", vec![aliased_property]),
            json!({
                "key":"text_alias",
                "body":{"kind":"alias","alias":{"target":"text"}}
            }),
            scalar("text", "string")
        ]),
        "root",
    );
    assert!(alias_to_string.admit().is_ok());
}

#[test]
fn admitted_declaration_addresses_are_bound_to_one_admission() {
    let fixture = || {
        document(
            json!([
                record("root", vec![property("name", "text")]),
                scalar("text", "string")
            ]),
            "root",
        )
        .admit()
        .unwrap()
    };
    let first = fixture();
    let second = fixture();
    let address = DeclarationAddress::new(
        DefinitionKey::new("root").unwrap(),
        DeclarationUse::Property(DefinitionMemberKey::new("name").unwrap()),
    );
    let first_address = first.resolve_address(&address).unwrap();
    let same_address = first.resolve_address(&address).unwrap();
    let second_address = second.resolve_address(&address).unwrap();
    assert!(first_address.belongs_to(&first));
    assert!(!first_address.belongs_to(&second));
    assert!(first_address.same_declaration(&same_address));
    assert!(!first_address.same_declaration(&second_address));
}

#[test]
fn definition_budget_accepts_exact_limit_and_rejects_one_over() {
    let chain = |count: usize| {
        (0..count)
            .map(|index| {
                if index + 1 == count {
                    scalar(&format!("d{index}"), "string")
                } else {
                    json!({
                        "key": format!("d{index}"),
                        "body": {
                            "kind": "alias",
                            "alias": { "target": format!("d{}", index + 1) }
                        }
                    })
                }
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        document(Value::Array(chain(MAX_GRAPH_DEFINITIONS)), "d0")
            .admit()
            .unwrap()
            .definition_count(),
        MAX_GRAPH_DEFINITIONS
    );
    let error = document(Value::Array(chain(MAX_GRAPH_DEFINITIONS + 1)), "d0")
        .admit()
        .expect_err("definition limit must reject");
    assert_eq!(codes(&error), ["schema.graph.definition_limit"]);
}

#[test]
fn reference_budget_accepts_exact_limit_and_rejects_one_over() {
    let graph = |references: usize| {
        let properties = (0..references - 1)
            .map(|index| optional_property(&format!("p{index}"), "leaf"))
            .collect();
        document(
            json!([record("root", properties), scalar("leaf", "string")]),
            "root",
        )
    };
    assert_eq!(
        graph(MAX_GRAPH_REFERENCES)
            .admit()
            .unwrap()
            .reference_count(),
        MAX_GRAPH_REFERENCES
    );
    let error = graph(MAX_GRAPH_REFERENCES + 1)
        .admit()
        .expect_err("reference limit must reject");
    assert_eq!(codes(&error), ["schema.graph.reference_limit"]);
}

#[test]
fn identifier_budget_counts_actual_definition_property_and_reference_identifiers() {
    let graph = |one_over: bool| {
        let mut properties = (0..2016)
            .map(|index| optional_property(&format!("p{index:063}"), "l"))
            .collect::<Vec<_>>();
        properties[0]["key"] = if one_over {
            json!(format!("q{:063}", 0))
        } else {
            json!(format!("q{:062}", 0))
        };
        let root = "r".repeat(16);
        document(
            json!([record(&root, properties), scalar("l", "string")]),
            &root,
        )
    };
    let admitted = graph(false)
        .admit()
        .expect("exact identifier byte limit admits");
    assert_eq!(admitted.reference_count(), 2017);
    let error = graph(true)
        .admit()
        .expect_err("one identifier byte over must reject");
    assert_eq!(codes(&error), ["schema.graph.identifier_bytes_limit"]);
    assert_eq!(MAX_GRAPH_IDENTIFIER_BYTES, 128 * 1024);
}

#[test]
fn structured_document_byte_budget_accepts_exact_limit_and_rejects_one_over() {
    let mut raw = json!({
        "version": 3,
        "root": { "target": "root" },
        "definitions": [{ "key": "root", "body": { "kind": "string" } }],
        "x-large": "",
    });
    let overhead = serde_json::to_vec(&raw).unwrap().len();
    raw["x-large"] = json!("a".repeat(MAX_GRAPH_DOCUMENT_BYTES - overhead));
    assert_eq!(
        serde_json::to_vec(&raw).unwrap().len(),
        MAX_GRAPH_DOCUMENT_BYTES
    );
    assert!(serde_json::from_value::<SchemaGraphDocument>(raw.clone()).is_ok());

    raw["x-large"] = json!("a".repeat(MAX_GRAPH_DOCUMENT_BYTES - overhead + 1));
    assert!(serde_json::from_value::<SchemaGraphDocument>(raw).is_err());
}

#[test]
fn every_semantic_facet_changes_the_semantic_commitment() {
    let base = json!({
        "version": 3,
        "root": { "target": "root" },
        "definitions": [
            record("root", vec![property("name", "text")]),
            scalar("text", "string")
        ]
    });
    let base_commitment = admitted(serde_json::from_value(base.clone()).unwrap()).0;
    let mutations = [
        (
            "/definitions/0/body/properties/0/presence",
            json!("optional"),
        ),
        (
            "/definitions/0/body/properties/0/presence",
            json!({"required_when":{"eq":["/enabled",true]}}),
        ),
        ("/definitions/0/body/properties/0/null", json!("allow")),
        (
            "/definitions/0/body/properties/0/null",
            json!({"reject_when":{"eq":["/enabled",true]}}),
        ),
        (
            "/definitions/0/body/properties/0/empty_string",
            json!("reject"),
        ),
        (
            "/definitions/0/body/properties/0/empty_string",
            json!({"reject_when":{"eq":["/enabled",true]}}),
        ),
        (
            "/definitions/0/body/properties/0/expression",
            json!("allowed"),
        ),
        (
            "/definitions/0/body/properties/0/aliases/read",
            json!(["old_name"]),
        ),
        (
            "/definitions/0/body/properties/0/aliases/write",
            json!("output_name"),
        ),
        (
            "/definitions/0/body/properties/0/rules",
            json!([{"custom": "check_name"}]),
        ),
        (
            "/definitions/0/body/properties/0/transformers",
            json!([{"kind": "trim"}]),
        ),
    ];
    for (pointer, replacement) in mutations {
        let mut changed = base.clone();
        *changed
            .pointer_mut(pointer)
            .expect("fixture pointer exists") = replacement;
        let changed = admitted(serde_json::from_value(changed).unwrap()).0;
        assert_ne!(
            base_commitment, changed,
            "mutation at {pointer} was omitted"
        );
    }
}

#[test]
fn directional_read_alias_priority_is_committed_in_authored_order() {
    let graph = |aliases: Value| {
        let mut name = property("name", "text");
        name["aliases"]["read"] = aliases;
        document(
            json!([record("root", vec![name]), scalar("text", "string")]),
            "root",
        )
    };
    assert_ne!(
        admitted(graph(json!(["first", "second"]))).0,
        admitted(graph(json!(["second", "first"]))).0
    );
}

#[test]
fn root_empty_collection_and_body_specific_semantics_are_all_committed() {
    let commitment = |raw: Value| admitted(serde_json::from_value(raw).unwrap()).0;
    let assert_mutates = |base: Value, pointer: &str, replacement: Value| {
        let original = commitment(base.clone());
        let mut changed = base;
        *changed
            .pointer_mut(pointer)
            .expect("fixture pointer exists") = replacement;
        assert_ne!(
            original,
            commitment(changed),
            "semantic field {pointer} was omitted"
        );
    };

    let root_record = json!({
        "version": 3,
        "root": { "target": "root", "null": "allow", "empty_collection": "allow" },
        "definitions": [record("root", vec![])]
    });
    assert_mutates(root_record.clone(), "/root/null", json!("reject"));
    assert_mutates(root_record, "/root/empty_collection", json!("reject"));

    let property_record = json!({
        "version": 3,
        "root": { "target": "root" },
        "definitions": [record("root", vec![property("child", "child")]), record("child", vec![])]
    });
    assert_mutates(
        property_record,
        "/definitions/0/body/properties/0/empty_collection",
        json!("reject"),
    );

    let integer = json!({
        "version": 3, "root": { "target": "root" },
        "definitions": [{"key":"root","body":{"kind":"integer","minimum":0,"maximum":10,"intrinsic_rules":[]}}]
    });
    assert_mutates(integer.clone(), "/definitions/0/body/minimum", json!(-1));
    assert_mutates(integer.clone(), "/definitions/0/body/maximum", json!(11));
    assert_mutates(
        integer,
        "/definitions/0/body/intrinsic_rules",
        json!([{"min":-10}]),
    );

    let number = json!({
        "version": 3, "root": { "target": "root" },
        "definitions": [{"key":"root","body":{"kind":"number","minimum":0.25,"maximum":10.5}}]
    });
    assert_mutates(number.clone(), "/definitions/0/body/minimum", json!(0.5));
    assert_mutates(number, "/definitions/0/body/maximum", json!(11.5));

    let array = json!({
        "version": 3, "root": { "target": "root" },
        "definitions": [
            {"key":"root","body":{"kind":"array","element":{"target":"leaf","expression":"forbidden"},"min_items":0,"max_items":5,"unique":false,"intrinsic_rules":[]}},
            scalar("leaf", "string")
        ]
    });
    assert_mutates(array.clone(), "/definitions/0/body/min_items", json!(1));
    assert_mutates(array.clone(), "/definitions/0/body/max_items", json!(6));
    assert_mutates(array.clone(), "/definitions/0/body/unique", json!(true));
    assert_mutates(
        array.clone(),
        "/definitions/0/body/intrinsic_rules",
        json!([{"min_items":0}]),
    );
    assert_mutates(
        array,
        "/definitions/0/body/element/expression",
        json!("allowed"),
    );

    let union = json!({
        "version": 3, "root": { "target": "root" },
        "definitions": [
            {"key":"root","body":{"kind":"union","tagging":"external","variants":[
                {"key":"none","payload":null},{"key":"some","payload":{"target":"leaf","expression":"forbidden"}}
            ],"selector_normalization":{"default_variant":"none","aliases":{"legacy":"some"}}}},
            scalar("leaf", "string")
        ]
    });
    assert_mutates(
        union.clone(),
        "/definitions/0/body/tagging",
        json!({"adjacent":{"tag":"kind","content":"value"}}),
    );
    assert_mutates(
        union.clone(),
        "/definitions/0/body/selector_normalization/default_variant",
        json!("some"),
    );
    assert_mutates(
        union.clone(),
        "/definitions/0/body/selector_normalization/aliases/legacy",
        json!("none"),
    );
    assert_mutates(
        union.clone(),
        "/definitions/0/body/variants/0/payload",
        json!({"target":"leaf"}),
    );
    assert_mutates(
        union,
        "/definitions/0/body/variants/1/payload/expression",
        json!("allowed"),
    );

    let scalar_rules = |kind: &str| {
        json!({
            "version": 3, "root": { "target": "root" },
            "definitions": [{"key":"root","body":{"kind":kind,"intrinsic_rules":[]}}]
        })
    };
    assert_mutates(
        scalar_rules("boolean"),
        "/definitions/0/body/intrinsic_rules",
        json!([{"one_of":[true]}]),
    );
    assert_mutates(
        scalar_rules("string"),
        "/definitions/0/body/intrinsic_rules",
        json!([{"min_length":1}]),
    );

    let kinds = ["any", "null", "boolean", "integer", "number", "string"];
    let commitments = kinds.map(|kind| {
        commitment(json!({
            "version": 3, "root": { "target": "root" },
            "definitions": [{"key":"root","body":{"kind":kind}}]
        }))
    });
    for left in 0..commitments.len() {
        for right in left + 1..commitments.len() {
            assert_ne!(commitments[left], commitments[right]);
        }
    }
}

#[test]
fn root_string_occurrence_policies_are_committed() {
    let base = json!({
        "version": 3,
        "root": {
            "target": "root", "empty_string": "allow", "expression": "forbidden",
            "rules": [], "transformers": []
        },
        "definitions": [scalar("root", "string")]
    });
    let original = admitted(serde_json::from_value(base.clone()).unwrap()).0;
    let mutations = [
        ("/root/empty_string", json!("reject")),
        ("/root/expression", json!("allowed")),
        ("/root/rules", json!([{"custom":"root_rule"}])),
        ("/root/transformers", json!([{"kind":"trim"}])),
    ];
    for (pointer, replacement) in mutations {
        let mut changed = base.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        assert_ne!(
            original,
            admitted(serde_json::from_value(changed).unwrap()).0
        );
    }
}

#[test]
fn every_explicit_edge_role_and_unit_variant_has_distinct_semantic_encoding() {
    let fixtures = [
        document(json!([scalar("root", "string")]), "root"),
        document(
            json!([
                json!({"key":"root","body":{"kind":"alias","alias":{"target":"leaf"}}}),
                scalar("leaf", "string")
            ]),
            "root",
        ),
        document(
            json!([
                record("root", vec![property("value", "leaf")]),
                scalar("leaf", "string")
            ]),
            "root",
        ),
        document(
            json!([
                json!({"key":"root","body":{"kind":"array","element":{"target":"leaf"}}}),
                scalar("leaf", "string")
            ]),
            "root",
        ),
        document(
            json!([
                json!({"key":"root","body":{"kind":"union","variants":[{"key":"v","payload":{"target":"leaf"}}],"tagging":"external"}}),
                scalar("leaf", "string")
            ]),
            "root",
        ),
        document(
            json!([
                json!({"key":"root","body":{"kind":"union","variants":[{"key":"v","payload":null}],"tagging":"external"}})
            ]),
            "root",
        ),
    ];
    let commitments = fixtures
        .into_iter()
        .map(|fixture| admitted(fixture).0)
        .collect::<Vec<_>>();
    for left in 0..commitments.len() {
        for right in left + 1..commitments.len() {
            assert_ne!(
                commitments[left], commitments[right],
                "roles {left} and {right} collided"
            );
        }
    }
}

#[test]
fn unknown_semantics_round_trip_but_only_optional_extensions_are_ignored() {
    let unknown = json!({
        "version": 3,
        "root": { "target": "root" },
        "definitions": [{
            "key": "root",
            "body": { "kind": "future_scalar", "future": { "private": "PAYLOAD" } }
        }]
    });
    let decoded: SchemaGraphDocument = serde_json::from_value(unknown.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), unknown);
    let error = decoded.admit().expect_err("unknown body must fail closed");
    assert_eq!(codes(&error), ["schema.graph.unknown_body"]);
    assert!(!format!("{error:?}{error}").contains("PAYLOAD"));

    let known_body_unknown_facet = json!({
        "version": 3,
        "root": { "target": "root" },
        "definitions": [record("root", vec![{
            let mut property = property("name", "text");
            property["future_occurrence_policy"] = json!({"private":"OCCURRENCE_PAYLOAD"});
            property
        }]), scalar("text", "string")]
    });
    let preserved: SchemaGraphDocument =
        serde_json::from_value(known_body_unknown_facet.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&preserved).unwrap(),
        known_body_unknown_facet
    );
    let error = preserved
        .admit()
        .expect_err("unknown occurrence facet must fail closed");
    assert_eq!(codes(&error), ["schema.graph.unknown_facet"]);
    assert!(!format!("{error:?}{error}").contains("OCCURRENCE_PAYLOAD"));

    let required_extension: SchemaGraphDocument = serde_json::from_value(json!({
        "version": 3,
        "root": { "target": "root" },
        "definitions": [scalar("root", "string")],
        "required_extensions": ["future.required"]
    }))
    .unwrap();
    assert_eq!(
        codes(&required_extension.admit().unwrap_err()),
        ["schema.graph.unknown_required_extension"]
    );

    let mut optional = json!({
        "version": 3,
        "root": { "target": "root", "x-root": { "opaque": true } },
        "definitions": [{
            "key": "root",
            "body": { "kind": "string", "x-widget": "ignored" },
            "x-owner": 7
        }],
        "x-document": [1, 2, 3]
    });
    let with_extensions: SchemaGraphDocument = serde_json::from_value(optional.clone()).unwrap();
    assert_eq!(serde_json::to_value(&with_extensions).unwrap(), optional);
    let with_commitments = admitted(with_extensions);
    optional.as_object_mut().unwrap().remove("x-document");
    optional["root"].as_object_mut().unwrap().remove("x-root");
    optional["definitions"][0]
        .as_object_mut()
        .unwrap()
        .remove("x-owner");
    optional["definitions"][0]["body"]
        .as_object_mut()
        .unwrap()
        .remove("x-widget");
    assert_eq!(
        with_commitments,
        admitted(serde_json::from_value(optional).unwrap())
    );
}

#[test]
fn admission_failure_returns_the_lossless_document_and_redacts_debug() {
    let raw = json!({
        "version": 3,
        "root": { "target": "missing", "x-private": "PRIVATE_ROOT" },
        "definitions": [{ "key": "root", "body": { "kind": "string" } }]
    });
    let document: SchemaGraphDocument = serde_json::from_value(raw.clone()).unwrap();
    let error = document.admit().expect_err("dangling root must reject");
    assert_eq!(serde_json::to_value(error.document()).unwrap(), raw);
    assert!(!format!("{error:?}{error}").contains("PRIVATE_ROOT"));
    let recovered = error.into_document();
    assert_eq!(serde_json::to_value(recovered).unwrap(), raw);
}

#[test]
fn semantically_equal_numbers_share_one_exact_commitment_encoding() {
    let bounded = |minimum: Value| {
        document(
            json!([{
                "key": "root",
                "body": { "kind": "number", "minimum": minimum }
            }]),
            "root",
        )
    };
    assert_eq!(
        admitted(bounded(json!(1))).0,
        admitted(bounded(json!(1.0))).0
    );
    assert_eq!(
        admitted(bounded(serde_json::from_str("-0.0").unwrap())).0,
        admitted(bounded(json!(0))).0
    );
    assert_ne!(
        admitted(bounded(json!(1))).0,
        admitted(bounded(json!(10))).0
    );
    assert_ne!(
        admitted(bounded(json!(-1))).0,
        admitted(bounded(json!(1))).0
    );

    let ruled = |member: &str| {
        let rule: Value =
            serde_json::from_str(&format!("{{\"all\":[{{\"greater_than\":{member}}}]}}")).unwrap();
        document(
            json!([{
                "key": "root",
                "body": { "kind": "number", "intrinsic_rules": [rule] }
            }]),
            "root",
        )
    };
    assert_eq!(admitted(ruled("1")).0, admitted(ruled("1.0")).0);
    assert_eq!(admitted(ruled("-0.0")).0, admitted(ruled("0")).0);
    assert_ne!(admitted(ruled("1")).0, admitted(ruled("10")).0);
}

#[test]
fn productivity_uses_terminal_nullability_at_every_occurrence_role() {
    let admitted_code = |definitions: Value| {
        serde_json::from_value::<SchemaGraphDocument>(json!({
            "version": 3,
            "root": { "target": "root", "null": "reject" },
            "definitions": definitions,
        }))
        .unwrap()
        .admit()
        .map(|_| ())
    };

    let root_nullable = document(
        json!([{"key":"root","body":{"kind":"alias","alias":{"target":"root","null":"reject"}}}]),
        "root",
    );
    let mut root_nullable_raw = serde_json::to_value(root_nullable).unwrap();
    root_nullable_raw["root"]["null"] = json!("allow");
    assert!(
        serde_json::from_value::<SchemaGraphDocument>(root_nullable_raw)
            .unwrap()
            .admit()
            .is_ok()
    );

    let recursive_record = |null: &str| {
        json!([record(
            "root",
            vec![{
                let mut recursive = property("next", "root");
                recursive["null"] = Value::String(null.to_owned());
                recursive
            }]
        )])
    };
    assert!(admitted_code(recursive_record("allow")).is_ok());
    assert_eq!(
        codes(&admitted_code(recursive_record("reject")).unwrap_err()),
        ["schema.graph.nonproductive_definition"]
    );

    let recursive_array = |null: &str| {
        json!([{"key":"root","body":{"kind":"array","min_items":1,
            "element":{"target":"root","null":(null)}}}])
    };
    assert!(admitted_code(recursive_array("allow")).is_ok());
    assert_eq!(
        codes(&admitted_code(recursive_array("reject")).unwrap_err()),
        ["schema.graph.nonproductive_definition"]
    );

    let recursive_alias = |null: &str| {
        json!([{"key":"root","body":{"kind":"alias",
            "alias":{"target":"root","null":(null)}}}])
    };
    assert_eq!(
        codes(&admitted_code(recursive_alias("allow")).unwrap_err()),
        ["schema.graph.nonproductive_definition"],
        "an outer null rejection filters an alias's terminal null"
    );
    assert!(
        document(recursive_alias("allow"), "root").admit().is_ok(),
        "a nullable root may accept the alias's terminal null"
    );
    assert_eq!(
        codes(&admitted_code(recursive_alias("reject")).unwrap_err()),
        ["schema.graph.nonproductive_definition"]
    );

    let recursive_payload = |null: &str| {
        json!([{"key":"root","body":{"kind":"union","variants":[
            {"key":"again","payload":{"target":"root","null":(null)}}
        ]}}])
    };
    assert!(admitted_code(recursive_payload("allow")).is_ok());
    assert_eq!(
        codes(&admitted_code(recursive_payload("reject")).unwrap_err()),
        ["schema.graph.nonproductive_definition"]
    );

    assert!(
        document(json!([scalar("root", "null")]), "root")
            .admit()
            .is_ok()
    );
    assert_eq!(
        codes(
            &serde_json::from_value::<SchemaGraphDocument>(json!({
                "version": 3,
                "root": { "target": "root", "null": "reject" },
                "definitions": [scalar("root", "null")],
            }))
            .unwrap()
            .admit()
            .unwrap_err()
        ),
        ["schema.graph.nonproductive_definition"]
    );
}

#[test]
fn optional_occurrences_may_reach_a_nonproductive_non_null_body() {
    let graph = serde_json::from_value::<SchemaGraphDocument>(json!({
        "version": 3,
        "root": { "target": "root", "null": "reject" },
        "definitions": [record("root", vec![optional_property("next", "cycle")]),
            {"key":"cycle","body":{"kind":"alias","alias":{"target":"cycle","null":"reject"}}}],
    }))
    .unwrap();
    assert!(
        graph.admit().is_ok(),
        "the optional property can be omitted"
    );
}

#[test]
fn open_record_without_declared_properties_can_produce_a_nonempty_object() {
    let graph = serde_json::from_value::<SchemaGraphDocument>(json!({
        "version": 3,
        "root": {
            "target": "root",
            "null": "reject",
            "empty_collection": "reject"
        },
        "definitions": [record("root", vec![])],
    }))
    .unwrap();

    assert!(
        graph.admit().is_ok(),
        "records are open, so an undeclared property can make the object nonempty"
    );
}

#[test]
fn selector_alias_namespace_and_adjacent_tagging_are_locally_validated() {
    let union = |aliases: Value, tagging: Value| {
        document(
            json!([{"key":"root","body":{"kind":"union","tagging":tagging,
                "variants":[{"key":"one","payload":null},{"key":"two","payload":null}],
                "selector_normalization":{"aliases":aliases}}}]),
            "root",
        )
    };
    for aliases in [json!({"one":"two"}), json!({"one":"one"})] {
        assert_eq!(
            codes(&union(aliases, json!("external")).admit().unwrap_err()),
            ["schema.graph.duplicate_local_key"]
        );
    }
    assert!(
        union(
            json!({"legacy_one":"one","old_one":"one"}),
            json!("external")
        )
        .admit()
        .is_ok()
    );
    assert_eq!(
        codes(
            &union(
                json!({}),
                json!({"adjacent":{"tag":"kind","content":"kind"}})
            )
            .admit()
            .unwrap_err()
        ),
        ["schema.graph.invalid_document"]
    );
}

#[test]
fn array_unique_defaults_only_when_absent_and_requires_a_boolean_when_present() {
    let array = |unique: Option<Value>| {
        let mut body = json!({"kind":"array","element":{"target":"leaf"}});
        if let Some(unique) = unique {
            body["unique"] = unique;
        }
        document(
            json!([{"key":"root","body":body}, scalar("leaf", "string")]),
            "root",
        )
    };
    assert!(array(None).admit().is_ok());
    assert!(array(Some(json!(false))).admit().is_ok());
    assert!(array(Some(json!(true))).admit().is_ok());
    for malformed in [json!("false"), Value::Null, json!({}), json!(0)] {
        assert_eq!(
            codes(&array(Some(malformed)).admit().unwrap_err()),
            ["schema.graph.invalid_document"]
        );
    }
}

#[test]
fn duplicate_members_are_rejected_during_lossless_document_decoding_at_any_depth() {
    let duplicates = [
        r#"{"version":3,"version":3,"root":{"target":"root"},"definitions":[{"key":"root","body":{"kind":"string"}}]}"#,
        r#"{"version":3,"root":{"target":"root"},"definitions":[{"key":"root","body":{"kind":"string","kind":"string"}}]}"#,
        r#"{"version":3,"root":{"target":"root"},"definitions":[{"key":"root","body":{"kind":"string","x-meta":{"nested":1,"nested":2}}}]}"#,
    ];
    for raw in duplicates {
        let error = serde_json::from_str::<SchemaGraphDocument>(raw)
            .expect_err("duplicate object members must fail before admission");
        assert!(error.to_string().contains("duplicate object member"));
    }
}

#[test]
fn address_commitment_has_an_independent_framing_oracle_and_tracks_address_changes() {
    fn append_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&u32::try_from(value.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }

    let fixture = document(
        json!([
            record("root", vec![property("child", "leaf")]),
            scalar("leaf", "string")
        ]),
        "root",
    );
    let actual = admitted(fixture).1;
    let mut expected = b"nebula-schema-graph-address-space".to_vec();
    expected.extend_from_slice(&1_u16.to_be_bytes());
    expected.push(0);
    append_string(&mut expected, "root");
    expected.extend_from_slice(&2_u32.to_be_bytes());
    append_string(&mut expected, "leaf");
    expected.extend_from_slice(&0_u32.to_be_bytes());
    append_string(&mut expected, "root");
    expected.extend_from_slice(&1_u32.to_be_bytes());
    expected.push(2);
    expected.push(1);
    append_string(&mut expected, "child");
    expected.extend_from_slice(&0_u32.to_be_bytes());
    append_string(&mut expected, "leaf");
    assert_eq!(actual.as_bytes(), blake3::hash(&expected).as_bytes());

    let alias = document(
        json!([{"key":"root","body":{"kind":"alias","alias":{"target":"leaf"}}}, scalar("leaf", "string")]),
        "root",
    );
    let element = document(
        json!([{"key":"root","body":{"kind":"array","element":{"target":"leaf"}}}, scalar("leaf", "string")]),
        "root",
    );
    assert_ne!(admitted(alias).1, admitted(element).1, "role must commit");

    let local = |key: &str| {
        document(
            json!([
                record("root", vec![property(key, "leaf")]),
                scalar("leaf", "string")
            ]),
            "root",
        )
    };
    assert_ne!(admitted(local("left")).1, admitted(local("right")).1);

    let mapping = |swap: bool| {
        let (left, right) = if swap { ("b", "a") } else { ("a", "b") };
        document(
            json!([
                record(
                    "root",
                    vec![property("left", left), property("right", right)]
                ),
                scalar("a", "string"),
                scalar("b", "string")
            ]),
            "root",
        )
    };
    assert_ne!(admitted(mapping(false)).1, admitted(mapping(true)).1);
}

#[test]
fn rule_commitments_follow_validator_numeric_and_value_equality_semantics() {
    let value_rule = |rule: Value| {
        document(
            json!([{"key":"root","body":{"kind":"number","intrinsic_rules":[rule]}}]),
            "root",
        )
    };
    for name in ["min", "max", "greater_than", "less_than"] {
        assert_eq!(
            admitted(value_rule(json!({(name): 1}))).0,
            admitted(value_rule(json!({(name): 1.0}))).0,
            "numeric value rule {name} must use mathematical equality"
        );
    }
    assert_ne!(
        admitted(value_rule(json!({"one_of":[1]}))).0,
        admitted(value_rule(json!({"one_of":[1.0]}))).0,
        "OneOf uses serde_json::Value equality"
    );
    assert_ne!(
        admitted(value_rule(json!({"one_of":[0]}))).0,
        admitted(value_rule(
            serde_json::from_str(r#"{"one_of":[-0.0]}"#).unwrap()
        ))
        .0
    );
    assert_eq!(
        admitted(value_rule(
            serde_json::from_str(r#"{"one_of":[0.0]}"#).unwrap()
        ))
        .0,
        admitted(value_rule(
            serde_json::from_str(r#"{"one_of":[-0.0]}"#).unwrap()
        ))
        .0
    );

    let predicate_rule = |rule: Value| {
        let mut root = json!({"target":"root","rules":[rule]});
        root["null"] = json!("reject");
        serde_json::from_value::<SchemaGraphDocument>(json!({
            "version": 3,
            "root": root,
            "definitions": [scalar("root", "string")],
        }))
        .unwrap()
    };
    for name in ["gt", "gte", "lt", "lte"] {
        assert_eq!(
            admitted(predicate_rule(json!({(name):["/n",1]}))).0,
            admitted(predicate_rule(json!({(name):["/n",1.0]}))).0,
            "numeric predicate {name} must use mathematical equality"
        );
    }
    for name in ["eq", "ne", "contains"] {
        assert_ne!(
            admitted(predicate_rule(json!({(name):["/n",1]}))).0,
            admitted(predicate_rule(json!({(name):["/n",1.0]}))).0,
            "equality predicate {name} must preserve JSON number representation"
        );
    }
    assert_ne!(
        admitted(predicate_rule(json!({"in":["/n",[1]]}))).0,
        admitted(predicate_rule(json!({"in":["/n",[1.0]]}))).0
    );
    assert_ne!(
        admitted(predicate_rule(json!({"eq":["/n",{"nested":1}]}))).0,
        admitted(predicate_rule(json!({"eq":["/n",{"nested":1.0}]}))).0
    );
    assert_eq!(
        admitted(predicate_rule(
            serde_json::from_str(r#"{"eq":["/n",0.0]}"#).unwrap()
        ))
        .0,
        admitted(predicate_rule(
            serde_json::from_str(r#"{"eq":["/n",-0.0]}"#).unwrap()
        ))
        .0
    );
}

#[test]
fn every_current_rule_variant_has_a_distinct_committed_encoding() {
    let ruled = |kind: &str, rule: Value| {
        serde_json::from_value::<SchemaGraphDocument>(json!({
            "version": 3,
            "root": { "target": "root", "rules": [rule] },
            "definitions": [{ "key": "root", "body": { "kind": kind } }],
        }))
        .unwrap()
    };
    let cases = [
        ("string", json!({"min_length":1})),
        ("string", json!({"max_length":4})),
        ("string", json!({"pattern":"^x"})),
        ("number", json!({"min":1})),
        ("number", json!({"max":4})),
        ("number", json!({"greater_than":0})),
        ("number", json!({"less_than":5})),
        ("number", json!({"one_of":[1,2]})),
        ("string", json!("email")),
        ("string", json!("url")),
        ("string", json!({"eq":["/x",1]})),
        ("string", json!({"ne":["/x",1]})),
        ("string", json!({"gt":["/x",1]})),
        ("string", json!({"gte":["/x",1]})),
        ("string", json!({"lt":["/x",1]})),
        ("string", json!({"lte":["/x",1]})),
        ("string", json!({"is_true":"/x"})),
        ("string", json!({"is_false":"/x"})),
        ("string", json!({"set":"/x"})),
        ("string", json!({"empty":"/x"})),
        ("string", json!({"contains":["/x",1]})),
        ("string", json!({"matches":["/x","^x"]})),
        ("string", json!({"in":["/x",[1,2]]})),
        ("string", json!({"custom":"runtime"})),
        ("string", json!({"unique_by":"/id"})),
        ("string", json!({"all":[]})),
        ("string", json!({"any":[]})),
        ("string", json!({"not":{"eq":["/x",1]}})),
        ("string", json!({"described":[{"eq":["/x",1]},"message"]})),
    ];
    let commitments = cases.map(|(kind, rule)| admitted(ruled(kind, rule)).0);
    for left in 0..commitments.len() {
        for right in left + 1..commitments.len() {
            assert_ne!(commitments[left], commitments[right]);
        }
    }

    let array_rules = [json!({"min_items":0}), json!({"max_items":2})];
    for rule in array_rules {
        let graph = document(
            json!([{"key":"root","body":{"kind":"array","element":{"target":"leaf"},
                "intrinsic_rules":[rule]}}, scalar("leaf", "string")]),
            "root",
        );
        assert_ne!(
            admitted(graph).0,
            admitted(document(
                json!([{"key":"root","body":{"kind":"array","element":{"target":"leaf"}}},
                    scalar("leaf", "string")]),
                "root"
            ))
            .0
        );
    }
}

#[test]
fn structural_productivity_honors_constant_conditions_and_empty_collections() {
    let graph = |root: Value, definitions: Value| {
        serde_json::from_value::<SchemaGraphDocument>(json!({
            "version": 3, "root": root, "definitions": definitions,
        }))
        .unwrap()
    };
    let conditional_record = |condition: Value| {
        let mut recursive = property("next", "root");
        recursive["presence"] = json!({"required_when":condition});
        graph(
            json!({"target":"root","null":"reject"}),
            json!([record("root", vec![recursive])]),
        )
    };
    for always_true in [
        json!({"all":[]}),
        json!({"not":{"any":[]}}),
        json!({"any":[{"all":[]}]}),
    ] {
        assert_eq!(
            codes(&conditional_record(always_true).admit().unwrap_err()),
            ["schema.graph.nonproductive_definition"]
        );
    }
    for not_always_true in [
        json!({"any":[]}),
        json!({"not":{"all":[]}}),
        json!({"eq":["/unknown",true]}),
    ] {
        assert!(conditional_record(not_always_true).admit().is_ok());
    }

    let rejection_record = |condition: Value| {
        let mut recursive = property("next", "root");
        recursive["null"] = json!({"reject_when":condition});
        graph(
            json!({"target":"root","null":"reject"}),
            json!([record("root", vec![recursive])]),
        )
    };
    assert_eq!(
        codes(&rejection_record(json!({"all":[]})).admit().unwrap_err()),
        ["schema.graph.nonproductive_definition"]
    );
    assert!(rejection_record(json!({"any":[]})).admit().is_ok());
    assert!(
        rejection_record(json!({"eq":["/unknown",true]}))
            .admit()
            .is_ok()
    );

    let empty_array = graph(
        json!({"target":"root","null":"reject","empty_collection":"reject"}),
        json!([{"key":"root","body":{"kind":"array","min_items":0,"max_items":0,
            "element":{"target":"leaf"}}}, scalar("leaf", "string")]),
    );
    assert_eq!(
        codes(&empty_array.admit().unwrap_err()),
        ["schema.graph.nonproductive_definition"]
    );
    let nonempty_array = graph(
        json!({"target":"root","null":"reject","empty_collection":"reject"}),
        json!([{"key":"root","body":{"kind":"array","min_items":0,"max_items":1,
            "element":{"target":"leaf"}}}, scalar("leaf", "string")]),
    );
    assert!(nonempty_array.admit().is_ok());

    let optional_nonempty_record = graph(
        json!({"target":"root","null":"reject","empty_collection":"reject"}),
        json!([
            record("root", vec![optional_property("value", "leaf")]),
            scalar("leaf", "string")
        ]),
    );
    assert!(optional_nonempty_record.admit().is_ok());
    let required_nonempty_record = graph(
        json!({"target":"root","null":"reject","empty_collection":"reject"}),
        json!([
            record("root", vec![property("value", "leaf")]),
            scalar("leaf", "string")
        ]),
    );
    assert!(required_nonempty_record.admit().is_ok());

    let conditional_empty = graph(
        json!({"target":"root","null":"reject",
            "empty_collection":{"reject_when":{"all":[]}}}),
        json!([
            {"key":"root","body":{"kind":"array","min_items":0,"max_items":0,
                "element":{"target":"leaf"}}},
            scalar("leaf", "string")
        ]),
    );
    assert_eq!(
        codes(&conditional_empty.admit().unwrap_err()),
        ["schema.graph.nonproductive_definition"]
    );
    let conditionally_permitted_empty = graph(
        json!({"target":"root","null":"reject",
            "empty_collection":{"reject_when":{"eq":["/unknown",true]}}}),
        json!([
            {"key":"root","body":{"kind":"array","min_items":0,"max_items":0,
                "element":{"target":"leaf"}}},
            scalar("leaf", "string")
        ]),
    );
    assert!(conditionally_permitted_empty.admit().is_ok());

    let alias_to_empty = |alias_empty_policy: &str| {
        graph(
            json!({"target":"root","null":"reject","empty_collection":"allow"}),
            json!([
                {"key":"root","body":{"kind":"alias","alias":{"target":"bag",
                    "null":"reject","empty_collection":alias_empty_policy}}},
                {"key":"bag","body":{"kind":"array","min_items":0,"max_items":0,
                    "element":{"target":"leaf"}}},
                scalar("leaf", "string")
            ]),
        )
    };
    assert!(alias_to_empty("allow").admit().is_ok());
    assert_eq!(
        codes(&alias_to_empty("reject").admit().unwrap_err()),
        ["schema.graph.nonproductive_definition"]
    );

    let array_of_empty = |element_empty_policy: &str| {
        graph(
            json!({"target":"root","null":"reject"}),
            json!([
                {"key":"root","body":{"kind":"array","min_items":1,
                    "element":{"target":"bag","null":"reject",
                        "empty_collection":element_empty_policy}}},
                {"key":"bag","body":{"kind":"array","min_items":0,"max_items":0,
                    "element":{"target":"leaf"}}},
                scalar("leaf", "string")
            ]),
        )
    };
    assert!(array_of_empty("allow").admit().is_ok());
    assert_eq!(
        codes(&array_of_empty("reject").admit().unwrap_err()),
        ["schema.graph.nonproductive_definition"]
    );

    let payload_of_empty = |payload_empty_policy: &str| {
        graph(
            json!({"target":"root","null":"reject"}),
            json!([
                {"key":"root","body":{"kind":"union","variants":[
                    {"key":"value","payload":{"target":"bag","null":"reject",
                        "empty_collection":payload_empty_policy}}
                ]}},
                {"key":"bag","body":{"kind":"array","min_items":0,"max_items":0,
                    "element":{"target":"leaf"}}},
                scalar("leaf", "string")
            ]),
        )
    };
    assert!(payload_of_empty("allow").admit().is_ok());
    assert_eq!(
        codes(&payload_of_empty("reject").admit().unwrap_err()),
        ["schema.graph.nonproductive_definition"]
    );
}

#[test]
fn json_data_model_visitor_checks_i128_and_u128_number_ranges() {
    use serde::de::value::{Error, I128Deserializer, U128Deserializer};

    assert!(
        SchemaGraphDocument::deserialize(I128Deserializer::<Error>::new(i128::from(i64::MIN)))
            .is_ok()
    );
    assert!(
        SchemaGraphDocument::deserialize(U128Deserializer::<Error>::new(u128::from(u64::MAX)))
            .is_ok()
    );
    assert!(SchemaGraphDocument::deserialize(I128Deserializer::<Error>::new(i128::MAX)).is_err());
    assert!(SchemaGraphDocument::deserialize(U128Deserializer::<Error>::new(u128::MAX)).is_err());
}

#[test]
fn semantic_commitment_has_an_independent_numeric_and_equality_framing_oracle() {
    fn append_u32(bytes: &mut Vec<u8>, value: usize) {
        bytes.extend_from_slice(&u32::try_from(value).unwrap().to_be_bytes());
    }
    fn append_bytes(bytes: &mut Vec<u8>, value: &[u8]) {
        append_u32(bytes, value.len());
        bytes.extend_from_slice(value);
    }

    let fixture = document(
        json!([{"key":"root","body":{"kind":"number","minimum":1.0,
            "intrinsic_rules":[{"one_of":[1.0]}]}}]),
        "root",
    );
    let actual = admitted(fixture).0;
    let mut expected = b"nebula-schema-graph-semantic".to_vec();
    expected.extend_from_slice(&1_u16.to_be_bytes());
    expected.push(0);
    expected.extend_from_slice(&0_u32.to_be_bytes());
    expected.extend_from_slice(&[0, 0, 0]);
    append_bytes(&mut expected, br#""forbidden""#);
    expected.extend_from_slice(&[0, 0]);
    expected.extend_from_slice(&0_u32.to_be_bytes());
    append_bytes(&mut expected, b"[]");
    expected.extend_from_slice(&1_u32.to_be_bytes());
    expected.extend_from_slice(&0_u32.to_be_bytes());
    expected.push(0x14);
    expected.push(1);
    append_bytes(&mut expected, b"1e0");
    expected.push(0);
    expected.extend_from_slice(&1_u32.to_be_bytes());
    expected.extend_from_slice(&[0x30, 0x47]);
    expected.extend_from_slice(&1_u32.to_be_bytes());
    append_bytes(&mut expected, b"1.0");
    assert_eq!(actual.as_bytes(), blake3::hash(&expected).as_bytes());

    let normalized_comparison = document(
        json!([{"key":"root","body":{"kind":"number","minimum":1,
            "intrinsic_rules":[{"one_of":[1.0]}]}}]),
        "root",
    );
    assert_eq!(actual, admitted(normalized_comparison).0);
    let distinct_equality = document(
        json!([{"key":"root","body":{"kind":"number","minimum":1,
            "intrinsic_rules":[{"one_of":[1]}]}}]),
        "root",
    );
    assert_ne!(actual, admitted(distinct_equality).0);
}

#[test]
fn protection_is_exact_use_site_semantics_and_allows_public_secret_reuse() {
    let reused = |protection: &str| {
        let mut secret = property("secret", "text");
        secret["protection"] = json!(protection);
        document(
            json!([
                record("root", vec![property("public", "text"), secret]),
                scalar("text", "string")
            ]),
            "root",
        )
    };
    let public = admitted(reused("public"));
    let secret = admitted(reused("secret_utf8"));
    assert_ne!(public.0, secret.0);
    assert_eq!(public.1, secret.1);

    for (protection, target) in [
        ("secret_utf8", "number"),
        ("secret_bytes", "string"),
        ("secret_utf8", "bytes"),
    ] {
        let mut protected = property("value", "leaf");
        protected["protection"] = json!(protection);
        let leaf = if target == "bytes" {
            json!({"key":"leaf","body":{"kind":"bytes","encoding":"base64"}})
        } else {
            scalar("leaf", target)
        };
        assert_eq!(
            codes(
                &document(json!([record("root", vec![protected]), leaf]), "root")
                    .admit()
                    .unwrap_err()
            ),
            ["schema.graph.inapplicable_facet"]
        );
    }

    let body_secret = document(json!([{"key":"root","body":{"kind":"secret"}}]), "root");
    assert_eq!(
        codes(&body_secret.admit().unwrap_err()),
        ["schema.graph.unknown_body"]
    );

    for (protection, terminal, admits) in [
        ("secret_utf8", "string", true),
        ("secret_utf8", "bytes", false),
        ("secret_bytes", "bytes", true),
        ("secret_bytes", "string", false),
    ] {
        let mut protected = property("value", "alias");
        protected["protection"] = json!(protection);
        let terminal = if terminal == "bytes" {
            json!({"key":"terminal","body":{"kind":"bytes","encoding":"base64"}})
        } else {
            scalar("terminal", terminal)
        };
        let graph = document(
            json!([
                record("root", vec![protected]),
                {"key":"alias","body":{"kind":"alias","alias":{"target":"terminal"}}},
                terminal
            ]),
            "root",
        );
        assert_eq!(graph.admit().is_ok(), admits, "{protection}");
    }
}

#[test]
fn bytes_use_canonical_base64_and_file_reference_is_deferred() {
    let bytes = document(
        json!([{"key":"root","body":{"kind":"bytes","encoding":"base64"}}]),
        "root",
    );
    assert!(bytes.admit().is_ok());
    for malformed in [
        json!({"kind":"bytes"}),
        json!({"kind":"bytes","encoding":"hex"}),
        json!({"kind":"bytes","encoding":"base64","rules":[]}),
        json!({"kind":"file_reference"}),
        json!({"kind":"file_reference","media_type":"text/plain"}),
    ] {
        assert!(
            document(json!([{"key":"root","body":malformed}]), "root")
                .admit()
                .is_err()
        );
    }

    let with_default = |value: &str| {
        let mut property = property("value", "bytes");
        property["input_default"] = json!(value);
        document(
            json!([
                record("root", vec![property]),
                {"key":"bytes","body":{"kind":"bytes","encoding":"base64"}}
            ]),
            "root",
        )
    };
    for canonical in ["", "TQ==", "TWE=", "TWFu", "+/8="] {
        assert!(with_default(canonical).admit().is_ok(), "{canonical}");
    }
    for rejected in ["TQ", "TQ=", "TQ===", "TR==", "TWF=", "TQ==\n", "-_8="] {
        assert!(with_default(rejected).admit().is_err(), "{rejected}");
    }
    let mut nonempty_bytes = property("value", "bytes");
    nonempty_bytes["empty_string"] = json!("reject");
    assert!(
        document(
            json!([
                record("root", vec![nonempty_bytes]),
                {"key":"bytes","body":{"kind":"bytes","encoding":"base64"}}
            ]),
            "root"
        )
        .admit()
        .is_ok(),
        "empty-string policy governs the zero-byte encoding"
    );
    for facet in [
        json!({"rules":[{"min_length":1}]}),
        json!({"transformers":[{"kind":"trim"}]}),
    ] {
        let mut bytes_use = property("value", "bytes");
        for (key, value) in facet.as_object().unwrap() {
            bytes_use[key] = value.clone();
        }
        assert!(
            document(
                json!([
                    record("root", vec![bytes_use]),
                    {"key":"bytes","body":{"kind":"bytes","encoding":"base64"}}
                ]),
                "root"
            )
            .admit()
            .is_err()
        );
    }
}

#[test]
fn accepted_domain_is_exact_nonempty_duplicate_free_and_occurrence_compatible() {
    let with_domain = |target: &str, domain: Value| {
        let mut value = property("value", "leaf");
        value["accepted_domain"] = domain;
        document(
            json!([record("root", vec![value]), scalar("leaf", target)]),
            "root",
        )
    };
    assert!(
        with_domain("number", json!({"closed":[1, 1.0]}))
            .admit()
            .is_ok()
    );
    assert_eq!(
        codes(
            &with_domain("number", json!({"closed":[1, 1]}))
                .admit()
                .unwrap_err()
        ),
        ["schema.graph.invalid_bounds"]
    );
    assert_eq!(
        codes(
            &with_domain("string", json!({"closed":[]}))
                .admit()
                .unwrap_err()
        ),
        ["schema.graph.invalid_bounds"]
    );
    assert_eq!(
        codes(
            &with_domain("string", json!({"closed":[1]}))
                .admit()
                .unwrap_err()
        ),
        ["schema.graph.inapplicable_facet"]
    );

    let mut nullable = property("value", "leaf");
    nullable["accepted_domain"] = json!({"closed":[null]});
    nullable["null"] = json!("reject");
    assert!(
        document(
            json!([record("root", vec![nullable]), scalar("leaf", "string")]),
            "root"
        )
        .admit()
        .is_err()
    );

    let domain_graph = |values: Value| {
        let mut value = property("value", "leaf");
        value["accepted_domain"] = json!({"closed":values});
        document(
            json!([record("root", vec![value]), scalar("leaf", "any")]),
            "root",
        )
    };
    assert_ne!(
        admitted(domain_graph(json!([1]))).0,
        admitted(domain_graph(json!([1.0]))).0
    );
    assert_eq!(
        admitted(domain_graph(json!([{"a":1,"b":2}]))).0,
        admitted(domain_graph(json!([{"b":2,"a":1}]))).0
    );
    assert_eq!(
        admitted(domain_graph(json!([1]))).1,
        admitted(domain_graph(json!([2]))).1
    );
    assert_eq!(
        admitted(domain_graph(json!([1, 2.0, {"key":"value"}]))).0,
        admitted(domain_graph(json!([{"key":"value"}, 1, 2.0]))).0,
        "closed domain order is not semantic"
    );
    assert_eq!(
        codes(
            &domain_graph(json!([{"a":1,"b":2}, {"b":2,"a":1}]))
                .admit()
                .unwrap_err()
        ),
        ["schema.graph.invalid_bounds"]
    );

    let mut protected = property("value", "leaf");
    protected["protection"] = json!("secret_utf8");
    protected["accepted_domain"] = json!({"closed":["x"]});
    assert!(
        document(
            json!([record("root", vec![protected]), scalar("leaf", "string")]),
            "root"
        )
        .admit()
        .is_err()
    );

    let array = |values: Value| {
        document(
            json!([
                {"key":"root","body":{"kind":"array","element":{
                    "target":"leaf","accepted_domain":{"closed":values}}}},
                scalar("leaf", "string")
            ]),
            "root",
        )
    };
    assert!(array(json!(["a", "b"])).admit().is_ok());
    assert!(array(json!([["a", "b"]])).admit().is_err());

    let mut container = property("value", "nested");
    container["accepted_domain"] = json!({"closed":[{"password":"x"}]});
    let mut password = property("password", "text");
    password["protection"] = json!("secret_utf8");
    assert!(
        document(
            json!([
                record("root", vec![container]),
                record("nested", vec![password]),
                scalar("text", "string")
            ]),
            "root"
        )
        .admit()
        .is_err()
    );
}

#[test]
fn additional_properties_policy_changes_the_correct_commitments_and_graph_edges() {
    let graph = |policy: Value, include_leaf: bool| {
        let mut definitions = vec![json!({"key":"root","body":{"kind":"record","properties":[],
            "additional_properties":policy}})];
        if include_leaf {
            definitions.push(scalar("leaf", "string"));
        }
        document(Value::Array(definitions), "root")
    };
    let open = admitted(graph(json!("open"), false));
    let closed = admitted(document(
        json!([
            {"key":"root","body":{"kind":"record","properties":[],
                "additional_properties":"closed"}}
        ]),
        "root",
    ));
    let typed = admitted(graph(json!({"typed":{"target":"leaf"}}), true));
    assert_ne!(open.0, closed.0);
    assert_ne!(open.0, typed.0);
    assert_ne!(open.1, typed.1);
    assert_eq!(open.1, admitted(graph(json!("open"), false)).1);

    let recursive = document(
        json!([
            {"key":"root","body":{"kind":"record","properties":[],
                "additional_properties":{"typed":{"target":"root","null":"reject"}}}}
        ]),
        "root",
    );
    assert!(
        recursive.admit().is_ok(),
        "an empty open-key object is productive"
    );

    let closed_nonempty = document(
        json!([{"key":"root","body":{"kind":"record","properties":[],
            "additional_properties":"closed"}}]),
        "root",
    );
    let mut raw = serde_json::to_value(closed_nonempty).unwrap();
    raw["root"]["null"] = json!("reject");
    raw["root"]["empty_collection"] = json!("reject");
    assert_eq!(
        codes(
            &serde_json::from_value::<SchemaGraphDocument>(raw)
                .unwrap()
                .admit()
                .unwrap_err()
        ),
        ["schema.graph.nonproductive_definition"]
    );
}

#[test]
fn input_default_is_property_only_context_free_semantics() {
    let graph = |default: Value, rules: Value| {
        let mut value = property("value", "leaf");
        value["input_default"] = default;
        value["rules"] = rules;
        document(
            json!([record("root", vec![value]), scalar("leaf", "string")]),
            "root",
        )
    };
    let plain = admitted(graph(json!("ok"), json!([])));
    let changed = admitted(graph(json!("other"), json!([])));
    assert_ne!(plain.0, changed.0);
    assert_eq!(plain.1, changed.1);
    assert!(graph(json!(1), json!([])).admit().is_err());
    let mut domain_default = property("value", "leaf");
    domain_default["accepted_domain"] = json!({"closed":["allowed"]});
    domain_default["input_default"] = json!("denied");
    assert!(
        document(
            json!([
                record("root", vec![domain_default]),
                scalar("leaf", "string")
            ]),
            "root"
        )
        .admit()
        .is_err()
    );
    assert!(
        graph(json!("x"), json!([{"min_length":2}]))
            .admit()
            .is_err()
    );
    assert!(
        graph(json!("xx"), json!([{"min_length":2}]))
            .admit()
            .is_ok()
    );
    assert!(
        graph(json!("x"), json!([{"eq":["/other",1]}]))
            .admit()
            .is_err()
    );
    assert!(
        graph(json!("x"), json!([{"custom":"runtime"}]))
            .admit()
            .is_err()
    );

    let mut protected = property("value", "leaf");
    protected["protection"] = json!("secret_utf8");
    protected["input_default"] = json!("secret");
    assert!(
        document(
            json!([record("root", vec![protected]), scalar("leaf", "string")]),
            "root"
        )
        .admit()
        .is_err()
    );

    let mut required_expression = property("value", "leaf");
    required_expression["expression"] = json!("required");
    required_expression["input_default"] = json!("x");
    assert!(
        document(
            json!([
                record("root", vec![required_expression]),
                scalar("leaf", "string")
            ]),
            "root"
        )
        .admit()
        .is_err()
    );

    let mut container = property("value", "nested");
    container["input_default"] = json!({"password":"x"});
    let mut password = property("password", "text");
    password["protection"] = json!("secret_utf8");
    assert!(
        document(
            json!([
                record("root", vec![container]),
                record("nested", vec![password]),
                scalar("text", "string")
            ]),
            "root"
        )
        .admit()
        .is_err()
    );

    let object_default = |value: Value| {
        let mut property = property("value", "leaf");
        property["input_default"] = value;
        document(
            json!([
                record("root", vec![property]),
                {"key":"leaf","body":{"kind":"any"}}
            ]),
            "root",
        )
    };
    assert_eq!(
        admitted(object_default(json!({"a":1,"b":2}))).0,
        admitted(object_default(json!({"b":2,"a":1}))).0
    );
    assert_ne!(
        admitted(object_default(json!(1))).0,
        admitted(object_default(json!(1.0))).0
    );
}

#[test]
fn input_default_rejects_required_expression_on_traversed_alias() {
    let mut value = property("value", "alias");
    value["input_default"] = json!("literal");
    let graph = document(
        json!([
            record("root", vec![value]),
            {
                "key":"alias",
                "body":{
                    "kind":"alias",
                    "alias":{"target":"text","expression":"required"}
                }
            },
            scalar("text", "string")
        ]),
        "root",
    );

    let error = graph
        .admit()
        .expect_err("defaults cannot cross an expression-required alias use");
    assert_eq!(codes(&error), ["schema.graph.invalid_default"]);
}

#[test]
fn typed_additional_property_is_addressable_with_one_frozen_role_edge() {
    let graph = document(
        json!([
            {"key":"root","body":{"kind":"record","properties":[],
                "additional_properties":{"typed":{"target":"leaf"}}}},
            scalar("leaf", "string")
        ]),
        "root",
    )
    .admit()
    .unwrap();
    assert_eq!(graph.reference_count(), 2);
    let address = DeclarationAddress::new(
        DefinitionKey::new("root").unwrap(),
        DeclarationUse::AdditionalProperty,
    );
    assert!(graph.resolve_address(&address).is_some());
}

#[test]
fn new_semantic_fields_fail_closed_outside_their_exact_locations() {
    for key in ["protection", "accepted_domain", "input_default"] {
        let mut body = json!({"kind":"string"});
        body[key] = json!("open");
        assert!(
            document(json!([{"key":"root","body":body}]), "root")
                .admit()
                .is_err()
        );
    }
    for forbidden in [
        "loader",
        "presentation",
        "media_type",
        "max_size",
        "visibility",
    ] {
        let mut root = json!({"target":"root"});
        root[forbidden] = json!(true);
        let document: SchemaGraphDocument = serde_json::from_value(json!({
            "version":3,"root":root,"definitions":[scalar("root", "string")]
        }))
        .unwrap();
        assert!(document.admit().is_err());
    }
}
