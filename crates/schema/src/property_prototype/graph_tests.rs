use std::{any::TypeId, sync::Arc};

use super::{
    admission::{
        AdmittedBodyRef, MAX_DEFINITIONS, MAX_IDENTIFIER_UTF8_BYTES, MAX_REFERENCE_EDGES, admit,
        checked_budget_add,
    },
    graph::{
        DefinitionBody, DefinitionDraft, DefinitionKey, EdgeRole, GraphDiagnostic, GraphLocation,
        Presence, PropertyEdge, SchemaDraft, TypeRef, VariantEdge,
    },
    rust_types::RustTypeRegistry,
};
use crate::FieldKey;

fn key(value: &str) -> DefinitionKey {
    DefinitionKey::new(value).expect("test definition key must be valid")
}

fn type_ref(value: &str) -> TypeRef {
    TypeRef::new(key(value))
}

fn field_key(value: &str) -> FieldKey {
    FieldKey::new(value).expect("test property or variant key must be valid")
}

fn definition(key_name: &str, body: DefinitionBody) -> DefinitionDraft {
    DefinitionDraft {
        key: key(key_name),
        body,
    }
}

fn string_definition(key_name: &str) -> DefinitionDraft {
    definition(key_name, DefinitionBody::String)
}

fn schema(root: &str, definitions: Vec<DefinitionDraft>) -> SchemaDraft {
    SchemaDraft {
        root: type_ref(root),
        definitions,
    }
}

fn property(name: &str, presence: Presence, target: &str) -> PropertyEdge {
    PropertyEdge {
        key: field_key(name),
        presence,
        target: type_ref(target),
    }
}

fn variant(name: &str, target: Option<&str>) -> VariantEdge {
    VariantEdge {
        key: field_key(name),
        target: target.map(type_ref),
    }
}

fn diagnostic(code: &'static str, location: GraphLocation) -> GraphDiagnostic {
    GraphDiagnostic { code, location }
}

fn index(value: u32) -> usize {
    usize::try_from(value).expect("test platform must represent admitted indices")
}

#[test]
fn definition_and_registration_permutations_are_non_semantic() {
    let left = admit(schema(
        "Root",
        vec![
            definition(
                "Root",
                DefinitionBody::Record(vec![
                    property("right", Presence::Optional, "LeafB"),
                    property("left", Presence::Required, "LeafA"),
                ]),
            ),
            string_definition("LeafB"),
            string_definition("LeafA"),
        ],
    ))
    .unwrap();
    let right = admit(schema(
        "Root",
        vec![
            string_definition("LeafA"),
            definition(
                "Root",
                DefinitionBody::Record(vec![
                    property("left", Presence::Required, "LeafA"),
                    property("right", Presence::Optional, "LeafB"),
                ]),
            ),
            string_definition("LeafB"),
        ],
    ))
    .unwrap();

    assert_eq!(left.definition_names(), ["LeafA", "LeafB", "Root"]);
    assert_eq!(left.definition_names(), right.definition_names());
    assert_eq!(left.canonical_bytes(), right.canonical_bytes());
    assert_eq!(left.commitment(), right.commitment());

    let mut first_registry = RustTypeRegistry::default();
    first_registry.register::<u8>(key("LeafA")).unwrap();
    first_registry.register::<u16>(key("LeafB")).unwrap();
    let mut second_registry = RustTypeRegistry::default();
    second_registry.register::<u16>(key("LeafB")).unwrap();
    second_registry.register::<u8>(key("LeafA")).unwrap();
    assert_eq!(
        first_registry.definition_for(TypeId::of::<u8>()),
        second_registry.definition_for(TypeId::of::<u8>())
    );
    assert_eq!(
        first_registry.definition_for(TypeId::of::<u16>()),
        second_registry.definition_for(TypeId::of::<u16>())
    );
}

#[test]
fn property_and_variant_order_is_non_semantic_but_definition_rename_is_semantic() {
    let ordered = schema(
        "Root",
        vec![
            definition(
                "Root",
                DefinitionBody::Record(vec![
                    property("alpha", Presence::Required, "Choice"),
                    property("omega", Presence::Optional, "Text"),
                ]),
            ),
            definition(
                "Choice",
                DefinitionBody::Union(vec![variant("none", None), variant("some", Some("Text"))]),
            ),
            string_definition("Text"),
        ],
    );
    let reordered = schema(
        "Root",
        vec![
            string_definition("Text"),
            definition(
                "Choice",
                DefinitionBody::Union(vec![variant("some", Some("Text")), variant("none", None)]),
            ),
            definition(
                "Root",
                DefinitionBody::Record(vec![
                    property("omega", Presence::Optional, "Text"),
                    property("alpha", Presence::Required, "Choice"),
                ]),
            ),
        ],
    );
    let renamed = schema(
        "Root",
        vec![
            definition(
                "Root",
                DefinitionBody::Record(vec![
                    property("alpha", Presence::Required, "Selection"),
                    property("omega", Presence::Optional, "Text"),
                ]),
            ),
            definition(
                "Selection",
                DefinitionBody::Union(vec![variant("none", None), variant("some", Some("Text"))]),
            ),
            string_definition("Text"),
        ],
    );

    let ordered = admit(ordered).unwrap();
    let reordered = admit(reordered).unwrap();
    let renamed = admit(renamed).unwrap();
    assert_eq!(ordered.canonical_bytes(), reordered.canonical_bytes());
    assert_eq!(ordered.commitment(), reordered.commitment());
    assert_ne!(ordered.canonical_bytes(), renamed.canonical_bytes());
    assert_ne!(ordered.commitment(), renamed.commitment());
}

#[test]
fn canonical_bytes_and_commitment_have_an_exact_private_golden() {
    let admitted = admit(schema("Root", vec![string_definition("Root")])).unwrap();
    let expected =
        b"nebula-property-graph-prototype\x00\x01\x10\x00\x04Root\x00\x00\x00\x01\x30\x00\x04Root\x20";

    assert_eq!(admitted.canonical_bytes(), expected);
    assert_eq!(
        hex::encode(admitted.commitment().as_bytes()),
        "fa7bdad984838063ab13ab2e5fe2b8b5784c9467ca228ae7a260ddbce7dca430"
    );
    assert_eq!(
        admitted.commitment().as_bytes(),
        blake3::hash(admitted.canonical_bytes()).as_bytes()
    );
}

fn complex_canonical_graph() -> SchemaDraft {
    schema(
        "Root",
        vec![
            definition(
                "Root",
                DefinitionBody::Record(vec![
                    property("required", Presence::Required, "Array"),
                    property("optional", Presence::Optional, "Choice"),
                ]),
            ),
            definition(
                "Array",
                DefinitionBody::Array {
                    item: type_ref("Alias"),
                    min_items: 1,
                    max_items: Some(3),
                    unique: true,
                },
            ),
            definition("Alias", DefinitionBody::Alias(type_ref("Text"))),
            definition(
                "Choice",
                DefinitionBody::Union(vec![
                    variant("payload", Some("Text")),
                    variant("unit", None),
                ]),
            ),
            string_definition("Text"),
        ],
    )
}

#[test]
fn complex_canonical_grammar_has_an_exact_private_golden() {
    let admitted = admit(complex_canonical_graph()).unwrap();
    let expected = b"nebula-property-graph-prototype\
\x00\x01\
\x10\x00\x04Root\
\x00\x00\x00\x05\
\x30\x00\x05Alias\x24\x16\x00\x04Text\
\x30\x00\x05Array\x22\x13\x00\x05Alias\x00\x00\x00\x01\x01\x00\x00\x00\x03\x01\
\x30\x00\x06Choice\x23\x00\x00\x00\x02\
\x14\x00\x04Text\x00\x07payload\
\x15\x00\x04unit\
\x30\x00\x04Root\x21\x00\x00\x00\x02\
\x12\x00\x06Choice\x00\x08optional\
\x11\x00\x05Array\x00\x08required\
\x30\x00\x04Text\x20";

    assert_eq!(admitted.canonical_bytes(), expected);
    assert_eq!(
        admitted.commitment().as_bytes(),
        blake3::hash(expected).as_bytes()
    );
}

fn assert_semantic_change(left: SchemaDraft, right: SchemaDraft) {
    let left = admit(left).unwrap();
    let right = admit(right).unwrap();
    assert_ne!(left.canonical_bytes(), right.canonical_bytes());
    assert_ne!(left.commitment(), right.commitment());
}

#[test]
fn every_encoded_graph_field_changes_bytes_and_commitment() {
    let record = |property_name, presence, target| {
        schema(
            "Root",
            vec![
                definition(
                    "Root",
                    DefinitionBody::Record(vec![property(property_name, presence, target)]),
                ),
                string_definition(target),
            ],
        )
    };
    assert_semantic_change(
        record("field", Presence::Required, "Text"),
        record("renamed", Presence::Required, "Text"),
    );
    assert_semantic_change(
        record("field", Presence::Required, "Text"),
        record("field", Presence::Optional, "Text"),
    );
    let property_targets = |first, second| {
        schema(
            "Root",
            vec![
                definition(
                    "Root",
                    DefinitionBody::Record(vec![
                        property("a", Presence::Optional, first),
                        property("b", Presence::Optional, second),
                    ]),
                ),
                string_definition("TextA"),
                string_definition("TextB"),
            ],
        )
    };
    assert_semantic_change(
        property_targets("TextA", "TextB"),
        property_targets("TextB", "TextA"),
    );
    assert_semantic_change(
        schema(
            "A",
            vec![
                definition(
                    "A",
                    DefinitionBody::Record(vec![property("b", Presence::Optional, "B")]),
                ),
                definition(
                    "B",
                    DefinitionBody::Record(vec![property("a", Presence::Optional, "A")]),
                ),
            ],
        ),
        schema(
            "B",
            vec![
                definition(
                    "A",
                    DefinitionBody::Record(vec![property("b", Presence::Optional, "B")]),
                ),
                definition(
                    "B",
                    DefinitionBody::Record(vec![property("a", Presence::Optional, "A")]),
                ),
            ],
        ),
    );
    assert_semantic_change(
        schema("Root", vec![string_definition("Root")]),
        schema(
            "Root",
            vec![definition("Root", DefinitionBody::Record(Vec::new()))],
        ),
    );

    let array = |item, min_items, max_items, unique| {
        schema(
            "Root",
            vec![
                definition(
                    "Root",
                    DefinitionBody::Array {
                        item: type_ref(item),
                        min_items,
                        max_items,
                        unique,
                    },
                ),
                string_definition(item),
            ],
        )
    };
    assert_semantic_change(array("Text", 0, None, false), array("Text", 1, None, false));
    assert_semantic_change(
        array("Text", 0, None, false),
        array("Text", 0, Some(3), false),
    );
    assert_semantic_change(
        array("Text", 0, Some(3), false),
        array("Text", 0, Some(4), false),
    );
    assert_semantic_change(array("Text", 0, None, false), array("Text", 0, None, true));
    let array_item = |target| {
        schema(
            "Root",
            vec![
                definition(
                    "Root",
                    DefinitionBody::Record(vec![
                        property("array", Presence::Required, "Array"),
                        property("a", Presence::Optional, "TextA"),
                        property("b", Presence::Optional, "TextB"),
                    ]),
                ),
                definition(
                    "Array",
                    DefinitionBody::Array {
                        item: type_ref(target),
                        min_items: 0,
                        max_items: None,
                        unique: false,
                    },
                ),
                string_definition("TextA"),
                string_definition("TextB"),
            ],
        )
    };
    assert_semantic_change(array_item("TextA"), array_item("TextB"));

    let union = |variant_name, target| {
        schema(
            "Root",
            vec![
                definition(
                    "Root",
                    DefinitionBody::Union(vec![variant(variant_name, target)]),
                ),
                string_definition("Text"),
            ],
        )
    };
    assert_semantic_change(union("value", Some("Text")), union("renamed", Some("Text")));
    let union_targets = |first, second| {
        schema(
            "Root",
            vec![
                definition(
                    "Root",
                    DefinitionBody::Union(vec![
                        variant("a", Some(first)),
                        variant("b", Some(second)),
                    ]),
                ),
                string_definition("TextA"),
                string_definition("TextB"),
            ],
        )
    };
    assert_semantic_change(
        union_targets("TextA", "TextB"),
        union_targets("TextB", "TextA"),
    );
    assert_semantic_change(
        union("value", Some("Text")),
        schema(
            "Root",
            vec![definition(
                "Root",
                DefinitionBody::Union(vec![variant("value", None)]),
            )],
        ),
    );

    let alias = |target| {
        schema(
            "Root",
            vec![
                definition(
                    "Root",
                    DefinitionBody::Record(vec![
                        property("alias", Presence::Required, "Alias"),
                        property("a", Presence::Optional, "TextA"),
                        property("b", Presence::Optional, "TextB"),
                    ]),
                ),
                definition("Alias", DefinitionBody::Alias(type_ref(target))),
                string_definition("TextA"),
                string_definition("TextB"),
            ],
        )
    };
    assert_semantic_change(alias("TextA"), alias("TextB"));
}

#[test]
fn admitted_edges_retain_authored_refs_indices_and_explicit_roles() {
    let admitted = admit(schema(
        "Root",
        vec![
            definition(
                "Root",
                DefinitionBody::Record(vec![property("child", Presence::Required, "Leaf")]),
            ),
            string_definition("Leaf"),
        ],
    ))
    .unwrap();

    assert_eq!(admitted.root().target(), &type_ref("Root"));
    assert_eq!(admitted.root().index_position(), index(1));
    assert_eq!(admitted.root().role(), EdgeRole::Root);
    let AdmittedBodyRef::Record(properties) = admitted.definitions()[1].body() else {
        panic!("root must remain a record");
    };
    assert_eq!(properties[0].target().target(), &type_ref("Leaf"));
    assert_eq!(properties[0].target().index_position(), index(0));
    assert_eq!(properties[0].target().role(), EdgeRole::RequiredProperty);
}

#[test]
fn optional_self_and_mutual_recursion_are_productive() {
    let self_recursive = schema(
        "Node",
        vec![definition(
            "Node",
            DefinitionBody::Record(vec![property("next", Presence::Optional, "Node")]),
        )],
    );
    let mutual = schema(
        "A",
        vec![
            definition(
                "A",
                DefinitionBody::Record(vec![property("b", Presence::Optional, "B")]),
            ),
            definition(
                "B",
                DefinitionBody::Record(vec![property("a", Presence::Optional, "A")]),
            ),
        ],
    );

    assert!(admit(self_recursive).is_ok());
    assert!(admit(mutual).is_ok());
}

#[test]
fn required_self_and_mutual_recursion_reject_the_first_sorted_definition() {
    let self_recursive = schema(
        "Node",
        vec![definition(
            "Node",
            DefinitionBody::Record(vec![property("next", Presence::Required, "Node")]),
        )],
    );
    let mutual = schema(
        "B",
        vec![
            definition(
                "B",
                DefinitionBody::Record(vec![property("a", Presence::Required, "A")]),
            ),
            definition(
                "A",
                DefinitionBody::Record(vec![property("b", Presence::Required, "B")]),
            ),
        ],
    );

    assert_eq!(
        admit(self_recursive).unwrap_err(),
        diagnostic(
            "graph.nonproductive_recursion",
            GraphLocation::Definition { index: index(0) }
        )
    );
    assert_eq!(
        admit(mutual).unwrap_err(),
        diagnostic(
            "graph.nonproductive_recursion",
            GraphLocation::Definition { index: index(0) }
        )
    );
}

#[test]
fn finite_shape_escape_cases_and_terminal_edges_are_productive() {
    let recursive_array = schema(
        "Items",
        vec![definition(
            "Items",
            DefinitionBody::Array {
                item: type_ref("Items"),
                min_items: 0,
                max_items: Some(0),
                unique: true,
            },
        )],
    );
    let union_escape = schema(
        "Choice",
        vec![definition(
            "Choice",
            DefinitionBody::Union(vec![
                variant("again", Some("Choice")),
                variant("stop", None),
            ]),
        )],
    );
    let terminal_edges = schema(
        "Root",
        vec![
            definition(
                "Root",
                DefinitionBody::Record(vec![property("terminal", Presence::Required, "Alias")]),
            ),
            definition("Alias", DefinitionBody::Alias(type_ref("Text"))),
            string_definition("Text"),
        ],
    );

    assert!(admit(recursive_array).is_ok());
    assert!(admit(union_escape).is_ok());
    assert!(admit(terminal_edges).is_ok());
}

#[test]
fn positive_min_recursive_array_and_alias_only_cycle_are_nonproductive() {
    let recursive_array = schema(
        "Items",
        vec![definition(
            "Items",
            DefinitionBody::Array {
                item: type_ref("Items"),
                min_items: 1,
                max_items: Some(5),
                unique: false,
            },
        )],
    );
    let aliases = schema(
        "A",
        vec![
            definition("A", DefinitionBody::Alias(type_ref("B"))),
            definition("B", DefinitionBody::Alias(type_ref("A"))),
        ],
    );

    for draft in [recursive_array, aliases] {
        assert_eq!(
            admit(draft).unwrap_err().code,
            "graph.nonproductive_recursion"
        );
    }
}

#[test]
fn duplicate_definitions_properties_and_variants_have_exact_locations() {
    let duplicate_definitions = schema("A", vec![string_definition("A"), string_definition("A")]);
    let duplicate_properties = schema(
        "Root",
        vec![
            definition(
                "Root",
                DefinitionBody::Record(vec![
                    property("same", Presence::Required, "Text"),
                    property("same", Presence::Optional, "Text"),
                ]),
            ),
            string_definition("Text"),
        ],
    );
    let duplicate_variants = schema(
        "Root",
        vec![definition(
            "Root",
            DefinitionBody::Union(vec![variant("same", None), variant("same", None)]),
        )],
    );

    assert_eq!(
        admit(duplicate_definitions).unwrap_err(),
        diagnostic(
            "graph.duplicate_definition",
            GraphLocation::Definition { index: index(1) }
        )
    );
    assert_eq!(
        admit(duplicate_properties).unwrap_err(),
        diagnostic(
            "graph.duplicate_property",
            GraphLocation::Edge {
                definition: index(0),
                ordinal: 1,
                role: EdgeRole::OptionalProperty,
            }
        )
    );
    assert_eq!(
        admit(duplicate_variants).unwrap_err(),
        diagnostic(
            "graph.duplicate_variant",
            GraphLocation::Edge {
                definition: index(0),
                ordinal: 1,
                role: EdgeRole::UnionUnit,
            }
        )
    );
}

#[test]
fn dangling_root_and_every_edge_role_have_exact_locations() {
    assert_eq!(
        admit(schema("Missing", vec![string_definition("Present")])).unwrap_err(),
        diagnostic("graph.dangling_root", GraphLocation::Root)
    );

    let cases = [
        (
            schema(
                "Root",
                vec![definition(
                    "Root",
                    DefinitionBody::Record(vec![property("child", Presence::Required, "Missing")]),
                )],
            ),
            "graph.dangling_property",
            EdgeRole::RequiredProperty,
        ),
        (
            schema(
                "Root",
                vec![definition(
                    "Root",
                    DefinitionBody::Array {
                        item: type_ref("Missing"),
                        min_items: 0,
                        max_items: None,
                        unique: false,
                    },
                )],
            ),
            "graph.dangling_array_item",
            EdgeRole::ArrayItem,
        ),
        (
            schema(
                "Root",
                vec![definition(
                    "Root",
                    DefinitionBody::Union(vec![variant("child", Some("Missing"))]),
                )],
            ),
            "graph.dangling_union_payload",
            EdgeRole::UnionPayload,
        ),
        (
            schema(
                "Root",
                vec![definition(
                    "Root",
                    DefinitionBody::Alias(type_ref("Missing")),
                )],
            ),
            "graph.dangling_alias",
            EdgeRole::Alias,
        ),
    ];

    for (draft, code, role) in cases {
        assert_eq!(
            admit(draft).unwrap_err(),
            diagnostic(
                code,
                GraphLocation::Edge {
                    definition: index(0),
                    ordinal: 0,
                    role,
                }
            )
        );
    }
}

#[test]
fn independent_structural_rejections_precede_productivity() {
    assert_eq!(
        admit(schema("Missing", Vec::new())).unwrap_err(),
        diagnostic("graph.empty_definitions", GraphLocation::Root)
    );
    assert_eq!(
        admit(schema(
            "Root",
            vec![definition("Root", DefinitionBody::Union(Vec::new()))],
        ))
        .unwrap_err(),
        diagnostic(
            "graph.empty_union",
            GraphLocation::Definition { index: index(0) }
        )
    );
    assert_eq!(
        admit(schema(
            "Root",
            vec![definition(
                "Root",
                DefinitionBody::Array {
                    item: type_ref("Root"),
                    min_items: 2,
                    max_items: Some(1),
                    unique: false,
                },
            )],
        ))
        .unwrap_err(),
        diagnostic(
            "graph.invalid_array_bounds",
            GraphLocation::Definition { index: index(0) }
        )
    );
}

#[test]
fn admission_stages_follow_the_documented_precedence() {
    let duplicate_before_local = schema(
        "A",
        vec![
            definition("A", DefinitionBody::Union(Vec::new())),
            definition("A", DefinitionBody::Union(Vec::new())),
        ],
    );
    assert_eq!(
        admit(duplicate_before_local).unwrap_err().code,
        "graph.duplicate_definition"
    );

    let mut local_before_budget = record_with_reference_count(MAX_REFERENCE_EDGES + 1);
    local_before_budget
        .definitions
        .push(definition("A", DefinitionBody::Union(Vec::new())));
    assert_eq!(
        admit(local_before_budget).unwrap_err().code,
        "graph.empty_union"
    );

    let mut budget_before_root = record_with_reference_count(MAX_REFERENCE_EDGES + 1);
    budget_before_root.root = type_ref("MissingRoot");
    assert_eq!(
        admit(budget_before_root).unwrap_err().code,
        "graph.reference_limit"
    );

    let root_before_edges = schema(
        "MissingRoot",
        vec![definition(
            "Root",
            DefinitionBody::Alias(type_ref("MissingAlias")),
        )],
    );
    assert_eq!(
        admit(root_before_edges).unwrap_err().code,
        "graph.dangling_root"
    );

    let sorted_dangling_before_closure = schema(
        "Root",
        vec![
            definition("Root", DefinitionBody::Alias(type_ref("MissingRootTarget"))),
            definition("A", DefinitionBody::Alias(type_ref("MissingFirst"))),
        ],
    );
    assert_eq!(
        admit(sorted_dangling_before_closure).unwrap_err(),
        diagnostic(
            "graph.dangling_alias",
            GraphLocation::Edge {
                definition: index(0),
                ordinal: 0,
                role: EdgeRole::Alias,
            }
        )
    );

    let unreachable_before_productivity = schema(
        "Root",
        vec![
            string_definition("Root"),
            definition("Cycle", DefinitionBody::Alias(type_ref("Cycle"))),
        ],
    );
    assert_eq!(
        admit(unreachable_before_productivity).unwrap_err().code,
        "graph.unreachable_definition"
    );
}

fn alias_chain(count: usize) -> SchemaDraft {
    let definitions = (0..count)
        .map(|position| {
            let current = format!("d{position:04}");
            if position + 1 == count {
                string_definition(&current)
            } else {
                definition(
                    &current,
                    DefinitionBody::Alias(type_ref(&format!("d{:04}", position + 1))),
                )
            }
        })
        .collect();
    schema("d0000", definitions)
}

#[test]
fn definition_budget_accepts_the_boundary_and_rejects_one_over() {
    assert!(admit(alias_chain(MAX_DEFINITIONS)).is_ok());
    assert_eq!(
        admit(alias_chain(MAX_DEFINITIONS + 1)).unwrap_err(),
        diagnostic("graph.definition_limit", GraphLocation::Root)
    );
}

fn record_with_reference_count(reference_count: usize) -> SchemaDraft {
    let mut properties = vec![
        property("required", Presence::Required, "Array"),
        property("optional", Presence::Optional, "Choice"),
    ];
    properties.extend(
        (0..reference_count.saturating_sub(6))
            .map(|position| property(&format!("p{position}"), Presence::Optional, "Text")),
    );
    schema(
        "Root",
        vec![
            definition("Root", DefinitionBody::Record(properties)),
            definition(
                "Array",
                DefinitionBody::Array {
                    item: type_ref("Alias"),
                    min_items: 0,
                    max_items: None,
                    unique: false,
                },
            ),
            definition(
                "Choice",
                DefinitionBody::Union(vec![
                    variant("payload", Some("Text")),
                    variant("unit", None),
                ]),
            ),
            definition("Alias", DefinitionBody::Alias(type_ref("Text"))),
            string_definition("Text"),
        ],
    )
}

fn identifier_boundary_graph(one_over: bool) -> SchemaDraft {
    let root = format!("R{}", "r".repeat(63));
    let text = format!("T{}", "t".repeat(63));
    let mut properties = vec![
        property("required", Presence::Required, "Array"),
        property("optional", Presence::Optional, "Choice"),
        property("alias_link", Presence::Optional, "Alias"),
    ];
    properties.extend((0..1020).map(|position| {
        property(
            &fixed_width_key("p", position, 64),
            Presence::Optional,
            &text,
        )
    }));
    properties.push(property(
        &fixed_width_key("z", 0, if one_over { 55 } else { 54 }),
        Presence::Optional,
        &text,
    ));

    schema(
        &root,
        vec![
            definition(&root, DefinitionBody::Record(properties)),
            definition(
                "Array",
                DefinitionBody::Array {
                    item: type_ref("Alias"),
                    min_items: 0,
                    max_items: None,
                    unique: false,
                },
            ),
            definition(
                "Choice",
                DefinitionBody::Union(vec![variant("payload", Some(&text)), variant("unit", None)]),
            ),
            definition("Alias", DefinitionBody::Alias(type_ref(&text))),
            string_definition(&text),
        ],
    )
}

#[test]
fn reference_budget_counts_root_and_accepts_the_boundary() {
    // Root, required/optional properties, array item, union payload, and alias are all present;
    // omitting any role from accounting makes the one-over graph pass.
    assert!(admit(record_with_reference_count(MAX_REFERENCE_EDGES)).is_ok());
    assert_eq!(
        admit(record_with_reference_count(MAX_REFERENCE_EDGES + 1)).unwrap_err(),
        diagnostic("graph.reference_limit", GraphLocation::Root)
    );
}

fn fixed_width_key(prefix: &str, position: usize, width: usize) -> String {
    let start = format!("{prefix}{position}_");
    format!("{start}{}", "x".repeat(width - start.len()))
}

#[test]
fn identifier_budget_counts_all_names_at_the_exact_boundary() {
    // The exact sum includes every definition/property/variant and every root, property, array,
    // union, and alias target; omitting any category makes the one-over graph pass.
    assert_eq!(MAX_IDENTIFIER_UTF8_BYTES, 128 * 1024);
    assert!(admit(identifier_boundary_graph(false)).is_ok());
    assert_eq!(
        admit(identifier_boundary_graph(true)).unwrap_err(),
        diagnostic("graph.identifier_bytes_limit", GraphLocation::Root)
    );
}

#[test]
fn checked_budget_arithmetic_reports_static_failures() {
    assert_eq!(
        checked_budget_add(usize::MAX, 1).unwrap_err(),
        diagnostic("graph.budget_overflow", GraphLocation::Root)
    );
}

#[test]
fn reachability_follows_all_roles_and_rejects_the_first_extra_definition() {
    let complete = schema(
        "Root",
        vec![
            definition(
                "Root",
                DefinitionBody::Record(vec![
                    property("required", Presence::Required, "Array"),
                    property("optional", Presence::Optional, "Union"),
                ]),
            ),
            definition(
                "Array",
                DefinitionBody::Array {
                    item: type_ref("Alias"),
                    min_items: 0,
                    max_items: None,
                    unique: false,
                },
            ),
            definition("Alias", DefinitionBody::Alias(type_ref("Text"))),
            definition(
                "Union",
                DefinitionBody::Union(vec![variant("payload", Some("Text"))]),
            ),
            string_definition("Text"),
        ],
    );
    assert!(admit(complete.clone()).is_ok());

    let mut with_extra = complete;
    with_extra.definitions.push(string_definition("Extra"));
    assert_eq!(
        admit(with_extra).unwrap_err(),
        diagnostic(
            "graph.unreachable_definition",
            GraphLocation::Definition { index: index(2) }
        )
    );
}

#[test]
fn rust_type_registry_is_a_build_local_bijection() {
    let mut registry = RustTypeRegistry::default();
    assert_eq!(registry.register::<Vec<u8>>(key("Bytes")), Ok(()));
    assert_eq!(registry.register::<Vec<u8>>(key("Bytes")), Ok(()));
    assert_eq!(
        registry.register::<Vec<u8>>(key("Other")),
        Err(diagnostic("graph.rust_type_conflict", GraphLocation::Root))
    );
    assert_eq!(
        registry.register::<Vec<u16>>(key("Bytes")),
        Err(diagnostic(
            "graph.definition_key_conflict",
            GraphLocation::Root
        ))
    );
    assert_eq!(registry.register::<Vec<u16>>(key("Words")), Ok(()));
    assert_eq!(
        registry.definition_for(TypeId::of::<Vec<u8>>()),
        Some(&key("Bytes"))
    );
    assert_eq!(
        registry.definition_for(TypeId::of::<Vec<u16>>()),
        Some(&key("Words"))
    );
}

#[test]
fn representation_keeps_array_refs_and_unit_variants_structural() {
    let array = DefinitionBody::Array {
        item: type_ref("Item"),
        min_items: 0,
        max_items: None,
        unique: false,
    };
    let DefinitionBody::Array { item, .. } = array else {
        panic!("array body shape changed");
    };
    assert_eq!(item, type_ref("Item"));
    assert_eq!(
        serde_json::to_value(&item).unwrap(),
        serde_json::json!("Item")
    );

    let unit = variant("unit", None);
    assert_eq!(unit.target, None);
}

#[test]
fn equal_independent_admissions_share_structure_but_not_arc_identity() {
    let draft = schema("Root", vec![string_definition("Root")]);
    let first = admit(draft.clone()).unwrap();
    let second = admit(draft).unwrap();
    assert_eq!(first.canonical_bytes(), second.canonical_bytes());
    assert_eq!(first.commitment(), second.commitment());
    assert!(!Arc::ptr_eq(first.identity(), second.identity()));
}

#[test]
fn diagnostic_debug_never_leaks_graph_payloads() {
    let secret_like_definition = "PrivateCustomerToken";
    let secret_like_property = "secret_property";
    let result = admit(schema(
        "Root",
        vec![definition(
            "Root",
            DefinitionBody::Record(vec![property(
                secret_like_property,
                Presence::Required,
                secret_like_definition,
            )]),
        )],
    ));
    let rendered = format!("{:?}", result.unwrap_err());

    assert!(rendered.contains("graph.dangling_property"));
    assert!(!rendered.contains(secret_like_definition));
    assert!(!rendered.contains(secret_like_property));
    assert!(!rendered.contains("SchemaDraft"));
    assert!(!rendered.contains("TypeId"));
}
