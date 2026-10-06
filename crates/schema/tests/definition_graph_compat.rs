//! Exact graph compatibility must preserve graph topology and occurrence policy.

use nebula_schema::{
    AdmittedSchemaGraph, Assignability, GraphRootKind, SchemaGraphDocument, ValidSchema, ValuePath,
    explain_graph_assignable, explain_graph_successor,
};
use serde_json::{Value, json};

fn graph(root: Value, definitions: Value) -> AdmittedSchemaGraph {
    serde_json::from_value::<SchemaGraphDocument>(json!({
        "version":3, "root":root, "definitions":definitions
    }))
    .unwrap()
    .admit()
    .unwrap_or_else(|error| panic!("test graph must admit: {:?}", error.report()))
}

fn root(target: &str) -> Value {
    json!({"target":target,"null":"reject"})
}

fn scalar(key: &str, kind: &str) -> Value {
    json!({"key":key,"body":{"kind":kind}})
}

fn property(key: &str, target: &str, required: bool) -> Value {
    json!({"key":key,"target":target,"null":"reject",
        "presence":if required {"required"} else {"optional"}})
}

fn record(key: &str, properties: Vec<Value>) -> Value {
    json!({"key":key,"body":{"kind":"record","properties":properties,
        "additional_properties":"closed"}})
}

fn compare(output: &AdmittedSchemaGraph, input: &AdmittedSchemaGraph) -> Assignability {
    explain_graph_assignable(output, input)
}

#[test]
fn recursive_records_compare_coinductively_without_lowering() {
    let recursive = |name: &str, leaf: &str, kind: &str| {
        graph(
            root(name),
            json!([
                record(
                    name,
                    vec![property("name", leaf, true), property("next", name, false)]
                ),
                scalar(leaf, kind)
            ]),
        )
    };
    let output = recursive("node", "text", "string");
    let input = recursive("renamed", "value", "string");
    assert!(ValidSchema::from_graph(&output).is_err());
    assert_eq!(compare(&output, &input), Assignability::Yes);
    assert!(matches!(
        compare(&output, &recursive("other", "value", "integer")),
        Assignability::No(_)
    ));
}

#[test]
fn arrays_compare_elements_and_exact_cardinality_domains() {
    let array = |kind: &str, min: u32, max: u32, unique: bool| {
        graph(
            root("items"),
            json!([
                {"key":"items","body":{"kind":"array","element":root("element"),
                    "min_items":min,"max_items":max,"unique":unique}},scalar("element",kind)
            ]),
        )
    };
    let output = array("integer", 2, 4, true);
    assert_eq!(
        compare(&output, &array("number", 1, 5, false)),
        Assignability::Yes
    );
    assert!(matches!(
        compare(&output, &array("string", 1, 5, false)),
        Assignability::No(_)
    ));
    let verdict = compare(&array("integer", 0, 10, false), &output);
    // The empty producer array is a concrete counterexample to min_items=2.
    assert!(matches!(verdict, Assignability::No(_)), "{verdict:?}");
}

#[test]
fn exact_numeric_bounds_do_not_round_large_integers() {
    let bounded = |minimum: u64, maximum: u64| {
        graph(
            root("integer"),
            json!([
                {"key":"integer","body":{"kind":"integer","minimum":minimum,"maximum":maximum}}
            ]),
        )
    };
    let base = 9_007_199_254_740_992;
    assert_eq!(
        compare(&bounded(base + 1, base + 2), &bounded(base, base + 3)),
        Assignability::Yes
    );
    assert!(matches!(
        compare(&bounded(base, base + 3), &bounded(base + 1, base + 2)),
        Assignability::Unknown(_)
    ));
}

#[test]
fn union_covariance_and_tagging_are_checked_on_graphs() {
    let union = |variants: &[&str], tagging: Value| {
        graph(
            root("sum"),
            json!([
                {"key":"sum","body":{"kind":"union","tagging":tagging,
                    "variants":variants.iter().map(|key|json!({"key":key,"payload":root("text")})).collect::<Vec<_>>()}},
                scalar("text","string")
            ]),
        )
    };
    let output = union(&["a"], json!("external"));
    assert_eq!(
        compare(&output, &union(&["a", "b"], json!("external"))),
        Assignability::Yes
    );
    assert!(matches!(
        compare(&union(&["a", "b"], json!("external")), &output),
        Assignability::No(_)
    ));
    assert!(matches!(
        compare(
            &output,
            &union(&["a"], json!({"adjacent":{"tag":"kind","content":"value"}}))
        ),
        Assignability::No(_)
    ));
}

#[test]
fn aliases_are_references_not_any_or_scalar_fallbacks() {
    let alias = |name: &str, target: &str| {
        graph(
            root(name),
            json!([
                {"key":name,"body":{"kind":"alias","alias":root(target)}},scalar(target,"string")
            ]),
        )
    };
    assert_eq!(
        compare(&alias("a", "x"), &alias("b", "y")),
        Assignability::Yes
    );
    assert_eq!(
        compare(
            &alias("a", "x"),
            &graph(root("text"), json!([scalar("text", "string")]))
        ),
        Assignability::Yes
    );
    assert!(matches!(
        compare(
            &alias("a", "x"),
            &graph(root("number"), json!([scalar("number", "number")]))
        ),
        Assignability::No(_)
    ));
}

#[test]
fn occurrence_presence_null_and_closed_domains_are_preserved() {
    let with_property = |required: bool, null_policy: &str, domain: Value| {
        let mut use_site = property("value", "text", required);
        use_site["null"] = json!(null_policy);
        use_site["accepted_domain"] = domain;
        graph(
            root("record"),
            json!([record("record", vec![use_site]), scalar("text", "string")]),
        )
    };
    let output = with_property(true, "reject", json!({"closed":["a"]}));
    let input = with_property(true, "allow", json!({"closed":["a","b"]}));
    assert_eq!(compare(&output, &input), Assignability::Yes);
    assert!(matches!(compare(&input, &output), Assignability::No(_)));
    assert!(matches!(
        compare(&with_property(false, "reject", json!("open")), &output),
        Assignability::No(_)
    ));
}

#[test]
fn rules_prove_only_identical_context_free_constraints() {
    let string = |rule: Value| {
        graph(
            root("text"),
            json!([
                {"key":"text","body":{"kind":"string","intrinsic_rules":[rule]}}
            ]),
        )
    };
    let output = string(json!({"min_length":3}));
    assert_eq!(
        compare(&output, &string(json!({"min_length":3}))),
        Assignability::Yes
    );
    assert!(matches!(
        compare(&output, &string(json!({"min_length":5}))),
        Assignability::Unknown(_)
    ));
    let contextual = graph(
        json!({"target":"text","null":"reject",
        "rules":[{"custom":"requires_runtime_context"}]}),
        json!([scalar("text", "string")]),
    );
    assert!(matches!(
        compare(&contextual, &contextual),
        Assignability::Unknown(_)
    ));
}

#[test]
fn emitted_names_match_input_read_aliases_and_refs_use_the_same_names() {
    let mut output_property = property("canonical", "text", true);
    output_property["aliases"] = json!({"write":"wire"});
    let output = graph(
        root("record"),
        json!([
            record("record", vec![output_property]),
            scalar("text", "string")
        ]),
    );
    let mut input_property = property("value", "text", true);
    input_property["aliases"] = json!({"read":["wire"]});
    let input = graph(
        root("record"),
        json!([
            record("record", vec![input_property]),
            scalar("text", "string")
        ]),
    );
    assert_eq!(compare(&output, &input), Assignability::Yes);
    assert_eq!(
        input
            .input_reference_at(&ValuePath::single("wire"))
            .unwrap()
            .canonical_path(),
        &ValuePath::single("value")
    );
    assert_eq!(
        output
            .reference_at(&ValuePath::single("wire"))
            .unwrap()
            .canonical_path(),
        &ValuePath::single("wire")
    );
    assert!(
        output
            .reference_at(&ValuePath::single("canonical"))
            .is_err()
    );
    assert_eq!(
        output
            .reference_at(&ValuePath::single("wire"))
            .unwrap()
            .explain_assignable_to(
                &input
                    .input_reference_at(&ValuePath::single("value"))
                    .unwrap()
            ),
        Assignability::Yes
    );
}

#[test]
fn input_reference_paths_canonicalize_nested_aliases_and_preserve_indices() {
    let mut outer = property("value_path", "list", true);
    outer["aliases"] = json!({"read":["wire"]});
    let mut inner = property("leaf_name", "text", true);
    inner["aliases"] = json!({"read":["read"]});
    let input = graph(
        json!({"target":"record","null":"allow"}),
        json!([
            record("record", vec![outer]),
            {"key":"list","body":{"kind":"array","element":root("item"),"min_items":1}},
            record("item", vec![inner]),
            scalar("text", "string")
        ]),
    );
    assert_eq!(
        input
            .input_reference_at(&ValuePath::from_segments(["wire", "0", "read"]))
            .unwrap()
            .canonical_path(),
        &ValuePath::from_segments(["value_path", "0", "leaf_name"])
    );
    assert_eq!(
        input
            .input_reference_at(&ValuePath::root())
            .unwrap()
            .canonical_path(),
        &ValuePath::root()
    );
    assert_eq!(
        ValuePath::from_segments(["value/path", "0", "leaf~name"]).as_str(),
        "/value~1path/0/leaf~0name"
    );
}

#[test]
fn output_successor_cannot_use_input_read_aliases() {
    let mut previous_property = property("canonical", "text", true);
    previous_property["aliases"] = json!({"read":["wire"]});
    let previous = graph(
        root("record"),
        json!([
            record("record", vec![previous_property]),
            scalar("text", "string")
        ]),
    );
    let successor = graph(
        root("record"),
        json!([
            record("record", vec![property("wire", "text", true)]),
            scalar("text", "string")
        ]),
    );
    assert_eq!(compare(&successor, &previous), Assignability::Yes);
    assert!(matches!(
        explain_graph_successor(&successor, &previous),
        Assignability::No(_)
    ));
}

#[test]
fn concrete_root_classifier_resolves_aliases_without_losing_null_policy() {
    let mut nullable_root = root("alias");
    nullable_root["null"] = json!("allow");
    let graph = graph(
        nullable_root,
        json!([
            {"key":"alias","body":{"kind":"alias","alias":{"target":"items","null":"allow"}}},
            {"key":"items","body":{"kind":"array","element":root("text")}},
            scalar("text","string")
        ]),
    );
    assert_eq!(graph.root_kind(), GraphRootKind::Array);
    assert_eq!(
        graph
            .reference_at(&ValuePath::single("0"))
            .unwrap_err()
            .code,
        "nullable_descendant"
    );
}

#[test]
fn open_and_typed_extra_keys_cannot_bypass_declared_input_constraints() {
    let output = |extra: Value, leaf: bool| {
        graph(
            root("record"),
            if leaf {
                json!([
                    {"key":"record","body":{"kind":"record","properties":[],"additional_properties":extra}},
                    scalar("number","integer")
                ])
            } else {
                json!([
                    {"key":"record","body":{"kind":"record","properties":[],"additional_properties":extra}}
                ])
            },
        )
    };
    let mut input_record = record("record", vec![property("value", "text", false)]);
    input_record["body"]["additional_properties"] = json!("open");
    let input = graph(
        root("record"),
        json!([input_record, scalar("text", "string")]),
    );
    assert!(matches!(
        compare(&output(json!({"typed":root("number")}), true), &input),
        Assignability::No(_)
    ));
    assert!(matches!(
        compare(&output(json!("open"), false), &input),
        Assignability::Unknown(_)
    ));
}

#[test]
fn extra_read_aliases_cannot_bypass_an_optional_declared_spelling() {
    let mut input_property = property("x", "text", false);
    input_property["aliases"] = json!({"read":["y"]});
    let mut input_record = record("record", vec![input_property]);
    input_record["body"]["additional_properties"] = json!("open");
    let input = graph(
        root("record"),
        json!([input_record, scalar("text", "string")]),
    );
    let output = |extra: Value, typed: bool, required: bool| {
        let mut output_record = record("record", vec![property("x", "text", required)]);
        output_record["body"]["additional_properties"] = extra;
        graph(
            root("record"),
            if typed {
                json!([
                    output_record,
                    scalar("text", "string"),
                    scalar("number", "integer")
                ])
            } else {
                json!([output_record, scalar("text", "string")])
            },
        )
    };
    // The producer can emit {"y":42} while x is absent. Input preparation
    // then reads y as x, which the consumer's string contract cannot accept.
    assert!(matches!(
        compare(
            &output(json!({"typed":root("number")}), true, false),
            &input
        ),
        Assignability::No(_)
    ));
    assert!(matches!(
        compare(&output(json!("open"), false, false), &input),
        Assignability::Unknown(_)
    ));
    // A required canonical value wins over all extra read-alias spellings.
    assert_eq!(
        compare(&output(json!({"typed":root("number")}), true, true), &input),
        Assignability::Yes
    );
}

#[test]
fn alias_occurrence_intersections_preserve_protection_and_null_constraints() {
    let aliased = graph(
        json!({"target":"alias","null":"allow"}),
        json!([
            {"key":"alias","body":{"kind":"alias","alias":{
                "target":"text","null":"reject","protection":"secret_utf8"}}},
            scalar("text","string")
        ]),
    );
    let direct = graph(
        json!({"target":"text","null":"reject","protection":"secret_utf8"}),
        json!([scalar("text", "string")]),
    );
    // The outermost occurrence decides null (an `Option<Newtype>` emits null
    // although the newtype's wrapped occurrence rejects it); protection is
    // still intersected through the alias.
    let Assignability::No(reasons) = compare(&aliased, &direct) else {
        panic!("a nullable producer must not satisfy a non-null consumer");
    };
    assert_eq!(
        format!("{reasons:?}"),
        r#"[GraphConstraintMismatch { code: "null" }]"#
    );
    assert_eq!(compare(&direct, &aliased), Assignability::Yes);
    let nonnull_aliased = graph(
        json!({"target":"alias","null":"reject"}),
        json!([
            {"key":"alias","body":{"kind":"alias","alias":{
                "target":"text","null":"reject","protection":"secret_utf8"}}},
            scalar("text","string")
        ]),
    );
    assert_eq!(compare(&nonnull_aliased, &direct), Assignability::Yes);
    let public = graph(root("text"), json!([scalar("text", "string")]));
    assert!(matches!(compare(&aliased, &public), Assignability::No(_)));
}

#[test]
fn protected_subtrees_never_collapse_into_any_or_open_extras() {
    let mut secret = property("token", "text", true);
    secret["protection"] = json!("secret_utf8");
    let output = graph(
        root("record"),
        json!([record("record", vec![secret]), scalar("text", "string")]),
    );
    let any = graph(root("opaque"), json!([scalar("opaque", "any")]));
    let open = graph(
        root("record"),
        json!([
            {"key":"record","body":{"kind":"record","properties":[],"additional_properties":"open"}}
        ]),
    );
    assert!(matches!(compare(&output, &any), Assignability::No(_)));
    assert!(matches!(compare(&output, &open), Assignability::No(_)));
    let reference = output.reference_at(&ValuePath::single("token")).unwrap();
    assert!(reference.is_protected_or_contains_protected());
    assert!(!format!("{reference:?}").contains("token"));
    assert_eq!(
        output
            .reference_at(&ValuePath::from_pointer("/token/child").unwrap())
            .unwrap_err()
            .code,
        "protected_descendant"
    );
}

#[test]
fn array_reference_paths_preserve_presence_and_reject_noncanonical_indices() {
    let output = graph(
        root("items"),
        json!([
            {"key":"items","body":{"kind":"array","element":root("text"),"min_items":1,"max_items":3}},
            scalar("text","string")
        ]),
    );
    let input = graph(root("text"), json!([scalar("text", "string")]));
    let consumer = input.input_reference_at(&ValuePath::root()).unwrap();
    assert_eq!(
        output
            .reference_at(&ValuePath::single("0"))
            .unwrap()
            .explain_assignable_to(&consumer),
        Assignability::Yes
    );
    assert!(matches!(
        output
            .reference_at(&ValuePath::single("1"))
            .unwrap()
            .explain_assignable_to(&consumer),
        Assignability::Unknown(_)
    ));
    assert!(output.reference_at(&ValuePath::single("3")).is_err());
    assert!(output.reference_at(&ValuePath::single("01")).is_err());
}

#[test]
fn union_reference_paths_retain_variant_presence_and_ambiguity() {
    let union = |variants: &[&str], tagging: Value| {
        graph(
            root("sum"),
            json!([
                {"key":"sum","body":{"kind":"union","tagging":tagging,
                    "variants":variants.iter().map(|key|json!({"key":key,"payload":root("text")})).collect::<Vec<_>>()}},
                scalar("text","string")
            ]),
        )
    };
    let input = graph(root("text"), json!([scalar("text", "string")]));
    let consumer = input.input_reference_at(&ValuePath::root()).unwrap();
    assert_eq!(
        union(&["a"], json!("external"))
            .reference_at(&ValuePath::single("a"))
            .unwrap()
            .explain_assignable_to(&consumer),
        Assignability::Yes
    );
    assert!(matches!(
        union(&["a", "b"], json!("external"))
            .reference_at(&ValuePath::single("a"))
            .unwrap()
            .explain_assignable_to(&consumer),
        Assignability::Unknown(_)
    ));
    assert_eq!(
        union(
            &["a", "b"],
            json!({"adjacent":{"tag":"kind","content":"value"}})
        )
        .reference_at(&ValuePath::single("value"))
        .unwrap_err()
        .code,
        "ambiguous_union"
    );
}

#[test]
fn diagnostics_are_bounded_and_do_not_include_closed_domain_payloads() {
    let mut output_properties = Vec::new();
    let mut input_properties = Vec::new();
    for index in 0..200 {
        let key = format!("field_{index}");
        let mut property = property(&key, "text", true);
        property["protection"] = json!("secret_utf8");
        output_properties.push(property);
        input_properties.push(self::property(&key, "text", true));
    }
    let output = graph(
        root("record"),
        json!([
            record("record", output_properties),
            scalar("text", "string")
        ]),
    );
    let input = graph(
        root("record"),
        json!([record("record", input_properties), scalar("text", "string")]),
    );
    let Assignability::No(findings) = compare(&output, &input) else {
        panic!("protection mismatch must fail");
    };
    assert!(findings.len() <= nebula_schema::definition::MAX_GRAPH_DIAGNOSTICS);
    assert!(!format!("{findings:?}").contains("field_"));
}

#[test]
fn recursive_reference_walk_exhausts_a_typed_budget_without_recursion() {
    let recursive = graph(
        root("list"),
        json!([
            {"key":"list","body":{"kind":"array","element":root("list")}}
        ]),
    );
    let path = ValuePath::from_segments(std::iter::repeat_n(
        "0",
        nebula_schema::MAX_GRAPH_COMPARISON_STEPS + 1,
    ));
    assert_eq!(
        recursive.reference_at(&path).unwrap_err().code,
        "path_budget"
    );
}

#[test]
fn selected_consumer_occurrences_retain_ancestor_domain_obligations() {
    let output = graph(root("text"), json!([scalar("text", "string")]));
    let mut input_root = root("record");
    input_root["accepted_domain"] = json!({"closed":[{"x":"a"}]});
    let input = graph(
        input_root,
        json!([
            record("record", vec![property("x", "text", true)]),
            scalar("text", "string")
        ]),
    );
    let output = output.reference_at(&ValuePath::root()).unwrap();
    let input = input.input_reference_at(&ValuePath::single("x")).unwrap();
    assert!(matches!(
        output.explain_assignable_to(&input),
        Assignability::Unknown(_)
    ));
}

#[test]
fn selected_consumer_array_elements_retain_ancestor_uniqueness() {
    let output = graph(root("text"), json!([scalar("text", "string")]));
    let input = graph(
        root("list"),
        json!([
            {"key":"list","body":{"kind":"array","element":root("text"),"min_items":2,"unique":true}},
            scalar("text", "string")
        ]),
    );
    let output = output.reference_at(&ValuePath::root()).unwrap();
    for index in ["0", "1"] {
        let input = input.input_reference_at(&ValuePath::single(index)).unwrap();
        assert!(matches!(
            output.explain_assignable_to(&input),
            Assignability::Unknown(_)
        ));
    }
}

#[test]
fn nullable_input_records_still_expose_their_exact_object_fields() {
    let output = graph(root("text"), json!([scalar("text", "string")]));
    let input = graph(
        json!({"target":"record","null":"allow"}),
        json!([
            record("record", vec![property("x", "text", true)]),
            scalar("text", "string")
        ]),
    );
    assert_eq!(
        output
            .reference_at(&ValuePath::root())
            .unwrap()
            .explain_assignable_to(&input.input_reference_at(&ValuePath::single("x")).unwrap()),
        Assignability::Yes
    );
    assert!(input.reference_at(&ValuePath::single("x")).is_err());
}
