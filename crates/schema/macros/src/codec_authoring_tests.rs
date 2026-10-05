//! Expansion contracts independent of runtime graph admission tests.

use quote::quote;

#[test]
fn nonrecord_reserved_tables_and_custom_alias_payloads_are_never_discarded() {
    for input in [
        quote!(
            #[schema(reserved("old"))]
            struct Unit;
        ),
        quote!(
            #[schema(reserved("old"))]
            struct Wrapper(String);
        ),
        quote!(
            #[serde(transparent)]
            #[schema(reserved("old"))]
            struct Wrapper {
                value: String,
            }
        ),
        quote!(
            struct Wrapper(#[serde(skip)] String);
        ),
        quote!(
            struct Wrapper(#[serde(skip_serializing)] String);
        ),
        quote!(
            enum Wrapper {
                Value(#[serde(skip_serializing)] String),
            }
        ),
        quote!(
            struct Wrapper(#[serde(with = "custom")] String);
        ),
        quote!(
            #[serde(transparent)]
            struct Wrapper {
                #[serde(deserialize_with = "custom")]
                value: String,
            }
        ),
    ] {
        let parsed = syn::parse2(input.clone()).unwrap();
        assert!(crate::derive_property_type::expand(&parsed).is_err());
        assert!(owned(quote!(both), input).is_err());
    }
}

#[test]
fn standalone_schema_preserves_original_refusals_before_graph_generation() {
    for input in [
        quote!(
            struct Bad<T> {
                field: T,
            }
        ),
        quote!(
            enum Bad<T> {
                Data(T),
            }
        ),
        quote!(
            enum Bad {
                Data(u32),
            }
        ),
        quote!(
            enum Bad {
                #[serde(alias = "Alt")]
                Primary(Cfg),
            }
        ),
        quote!(
            #[schema(reserved("Old"))]
            enum Bad {
                Old,
                New,
            }
        ),
        quote!(
            #[schema(custom = "rule")]
            enum Bad {
                Old,
                New,
            }
        ),
        quote!(
            struct Bad {
                #[field(emit_as = "wire")]
                a: String,
                #[serde(alias = "wire")]
                b: String,
            }
        ),
        quote!(
            struct Bad {
                #[serde(flatten)]
                inner: Cfg,
            }
        ),
    ] {
        let input = syn::parse2(input).unwrap();
        assert!(crate::expand_legacy_schema(&input).is_err());
    }
    let generic: syn::DeriveInput = syn::parse_quote!(
        struct Generic<T> {
            value: T,
        }
    );
    assert!(crate::derive_property_type::expand(&generic).is_ok());
    assert!(
        owned(
            quote!(both),
            quote!(
                struct Generic<T> {
                    value: T,
                }
            )
        )
        .is_ok()
    );
    let reserved: syn::DeriveInput = syn::parse_quote!(
        #[schema(reserved("Old"))]
        enum Bad {
            Old,
            New,
        }
    );
    assert!(crate::derive_property_type::expand(&reserved).is_err());
    assert!(
        owned(
            quote!(both),
            quote!(
                #[schema(reserved("Old"))]
                enum Bad {
                    Old,
                    New,
                }
            )
        )
        .is_err()
    );
}

#[test]
fn cached_legacy_secret_marker_has_one_diagnostic_owner() {
    let input: syn::DeriveInput = syn::parse_quote! {
        struct SecretRecord { #[field(secret)] token: SecretWrapper }
    };
    let expansion = crate::expand_legacy_schema(&input).unwrap().to_string();
    assert_eq!(
        expansion.matches("fn __nebula_assert_secret_input").count(),
        1
    );
    let recursive: syn::DeriveInput = syn::parse_quote! {
        struct SecretRecord {
            #[field(secret)] token: SecretWrapper,
            next: Option<Box<SecretRecord>>,
        }
    };
    let expansion = crate::expand_legacy_schema(&recursive).unwrap().to_string();
    assert_eq!(
        expansion.matches("fn __nebula_assert_secret_input").count(),
        1
    );
    assert!(!expansion.contains("OnceLock"));
}

#[test]
fn structural_catalog_and_secret_omission_keep_legacy_wrapper_authority() {
    let input: syn::DeriveInput = syn::parse_quote! {
        struct Input {
            #[field(enum_select)]
            #[serde(skip_serializing_if = "Option::is_none")]
            choice: Option<CatalogOnly>,
            #[field(secret)]
            #[serde(skip_serializing_if = "Option::is_none")]
            secret: Option<SecretOnly>,
        }
    };
    let structural = crate::derive_property_type::expand(&input)
        .unwrap()
        .to_string();
    assert!(structural.contains("define_variant"));
    assert!(structural.contains("HasSelectOptions"));
    assert!(structural.contains("SecretInput"));
    assert!(structural.contains("Input => __use . allow_null"));
    assert!(structural.contains("Output => __use"));
    assert!(!structural.contains("< CatalogOnly as"));
    assert!(!structural.contains("< SecretOnly as"));
    assert!(structural.contains("fn (Option < CatalogOnly >)"));
    let owned = crate::derive_property_type::expand_owned(&input)
        .unwrap()
        .to_string();
    assert!(owned.contains("Output => < CatalogOnly as"));
    assert!(owned.contains("Output => < SecretOnly as"));
    assert!(!owned.contains("define_variant"));
}

#[test]
fn null_and_empty_array_defaults_own_typed_generic_factories_and_graph_values() {
    let expansion = owned(
        quote!(both),
        quote! {
            struct Input<T> {
                #[property(input(default = null))]
                nullable: Option<T>,
                #[property(input(default = []))]
                rows: Vec<T>,
            }
        },
    )
    .unwrap();
    let output = expansion.to_string();
    assert!(output.contains("Option :: None"));
    assert!(output.contains("fn __nebula_schema_"));
    assert!(output.contains("< __NebulaDefault >"));
    assert!(output.contains("__default : Option < T >"));
    assert!(output.contains("__default : Vec < T >"));
    assert!(output.contains("Value :: Null"));
    assert!(output.contains("Value :: Array"));
    assert!(!output.contains("to_value (__default)"));
    syn::parse2::<syn::File>(expansion).unwrap();
    for input in [
        quote!(
            struct Input {
                #[property(input(default = null))]
                value: String,
            }
        ),
        quote!(
            struct Input {
                #[property(input(default = []))]
                value: bool,
            }
        ),
        quote!(
            struct Input {
                #[property(input(default = [1]))]
                value: Vec<u32>,
            }
        ),
        quote!(
            struct Input {
                #[property(input(default = make_default()))]
                value: Option<u32>,
            }
        ),
        quote!(
            struct Input {
                #[property(input(secret, default = null))]
                value: Option<String>,
            }
        ),
    ] {
        assert!(owned(quote!(input), input).is_err());
    }
}

#[test]
fn option_omission_uses_only_the_present_inner_domain_outbound() {
    let expansion = owned(
        quote!(both),
        quote! {
            struct Input {
                #[serde(skip_serializing_if = "Option::is_none")]
                value: Option<Option<String>>,
            }
        },
    )
    .unwrap()
    .to_string();
    assert!(expansion.contains(
        "fn (Option < Option < String > >) -> :: core :: option :: Option < Option < String > >"
    ));
    assert!(expansion.contains("Output => < Option < String > as"));
    assert!(expansion.contains("Input => < Option < Option < String > > as"));
    assert!(expansion.contains("\"optional\""));
    for input in [
        quote!(
            struct Input {
                #[serde(skip_serializing_if = "custom")]
                value: Option<String>,
            }
        ),
        quote!(
            struct Input {
                #[serde(skip_serializing_if = "Option::is_none")]
                value: String,
            }
        ),
    ] {
        assert!(owned(quote!(output), input).is_err());
    }
}

#[test]
fn owned_select_and_secret_fields_retain_the_actual_child_graph() {
    let input: syn::DeriveInput = syn::parse_quote! {
        struct Input {
            #[field(enum_select)]
            choice: Choice,
            #[field(secret)]
            secret: SecretWrapper,
        }
    };
    let owned = crate::derive_property_type::expand_owned(&input)
        .unwrap()
        .to_string();
    assert!(owned.contains("< Choice as"));
    assert!(owned.contains("< SecretWrapper as"));
    assert!(!owned.contains("define_variant"));
    let structural = crate::derive_property_type::expand(&input)
        .unwrap()
        .to_string();
    assert!(structural.contains("define_variant"));
    assert!(!structural.contains("< SecretWrapper as"));
}

#[test]
fn owned_helpers_are_consumed_and_enum_catalog_labels_survive() {
    let expansion = owned(
        quote!(both),
        quote! {
            #[derive(Debug, EnumSelect)]
            #[serde(rename_all = "snake_case")]
            enum Choice {
                #[field(label = "First choice")]
                One,
            }
        },
    )
    .unwrap();
    let output = expansion.to_string();
    assert!(output.contains("First choice"));
    assert!(output.contains("HasSelectOptions"));
    assert!(output.contains("PropertyType for Choice"));
    assert!(!output.contains("EnumSelect"));
    assert!(!output.contains("# [field"));
    syn::parse2::<syn::File>(expansion).unwrap();
    let record = owned(
        quote!(input),
        quote! {
            #[schema(condition(enabled, is_true(field(ok))))]
            struct Input {
                #[property(input(expressions = forbidden))]
                ok: bool,
            }
        },
    )
    .unwrap()
    .to_string();
    assert!(!record.contains("# [property"));
    assert!(!record.contains("# [schema"));
    assert!(record.contains("named_condition"));
}

#[test]
fn generated_newtypes_make_only_the_alias_edge_expression_neutral() {
    for input in [
        quote!(
            struct Nullable(Option<String>);
        ),
        quote!(
            #[serde(transparent)]
            struct Array {
                value: Vec<String>,
            }
        ),
    ] {
        let input = syn::parse2(input).unwrap();
        let expansion = crate::derive_property_type::expand(&input)
            .unwrap()
            .to_string();
        assert!(expansion.contains("__alias [\"expression\"]"));
        assert_eq!(expansion.matches("json ! (\"allowed\")").count(), 1);
        assert!(expansion.contains("\"alias\" : __alias"));
    }
}

#[test]
fn named_conditions_resolve_inbound_field_keys_and_checked_local_references() {
    let input: syn::DeriveInput = syn::parse_quote! {
        #[schema(condition(auth, is_true(field(authenticated))))]
        #[schema(condition(send, all(condition(auth), ne(root("/channel"), ""))))]
        struct Input {
            #[serde(rename(deserialize = "auth", serialize = "authenticated"))]
            authenticated: bool,
            channel: String,
        }
    };
    let expansion = crate::derive_property_type::expand(&input)
        .unwrap()
        .to_string();
    assert!(expansion.contains("\"/auth\""));
    assert!(expansion.contains("named_condition"));
    assert!(expansion.contains("Condition :: try_from"));
}

#[test]
fn named_conditions_reject_duplicates_missing_fields_and_recursive_references() {
    for input in [
        quote! { #[schema(condition(a, is_true(field(ok))), condition(a, is_false(field(ok))))] struct Input { ok: bool } },
        quote! { #[schema(condition(a, is_true(field(absent))))] struct Input { ok: bool } },
        quote! { #[schema(condition(a, condition(missing)))] struct Input { ok: bool } },
        quote! { #[schema(condition(a, condition(b)), condition(b, condition(a)))] struct Input { ok: bool } },
        quote! { #[schema(condition(a, custom_rust(field(ok))))] struct Input { ok: bool } },
        quote! { #[schema(condition(a, is_true(field(ok))))] struct Input { #[serde(skip_deserializing)] ok: bool } },
    ] {
        let input = syn::parse2(input).unwrap();
        assert!(crate::derive_property_type::expand(&input).is_err());
    }
}

#[test]
fn select_catalog_field_does_not_grant_the_enum_structural_or_codec_traits() {
    let input: syn::DeriveInput = syn::parse_quote! {
        struct Catalog {
            #[field(enum_select)]
            choice: Option<Choice>,
        }
    };
    let expansion = crate::derive_property_type::expand(&input)
        .unwrap()
        .to_string();
    assert!(expansion.contains("define_variant"));
    assert!(expansion.contains("HasSelectOptions"));
    assert!(expansion.contains("allow_null"));
    assert!(!expansion.contains("Choice > as"));
    assert!(!expansion.contains("Codec"));
}

#[test]
fn annotated_collection_facets_do_not_retrieve_legacy_child_schemas() {
    let input: syn::DeriveInput = syn::parse_quote! {
        struct RecursiveCollections {
            #[validate(min_items = 1)]
            children: Vec<RecursiveCollections>,
            #[field(label = "Nested")]
            nested: Vec<Vec<String>>,
        }
    };
    let expansion = crate::derive_property_type::expand(&input)
        .unwrap()
        .to_string();
    assert!(expansion.contains("Property :: list"));
    assert!(!expansion.contains("HasSchema"));
    assert!(expansion.contains("PropertyType"));
}

fn owned(
    options: proc_macro2::TokenStream,
    input: proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    crate::schema_type::expand(options, syn::parse2(input)?)
}

#[test]
fn owned_direction_emits_only_requested_serde_and_codec() {
    let input = owned(
        quote!(input),
        quote!(
            struct Input {
                value: String,
            }
        ),
    )
    .unwrap()
    .to_string();
    assert!(input.contains("Deserialize"));
    assert!(input.contains("InputCodec"));
    assert!(!input.contains("OutputCodec"));
    assert!(!input.contains(":: Serialize"));
    let output = owned(
        quote!(output),
        quote!(
            struct Output {
                value: String,
            }
        ),
    )
    .unwrap()
    .to_string();
    assert!(output.contains("Serialize"));
    assert!(output.contains("OutputCodec"));
    assert!(!output.contains("InputCodec"));
    assert!(!output.contains("Deserialize"));
}

#[test]
fn owned_direction_rejects_separate_witness_derives() {
    for name in ["Serialize", "Deserialize", "Schema", "PropertyType"] {
        let derive: syn::Ident = syn::parse_str(name).unwrap();
        assert!(
            owned(
                quote!(both),
                quote!(
                    #[derive(#derive)]
                    struct Value {
                        value: String,
                    }
                )
            )
            .is_err()
        );
    }
}

#[test]
fn generic_struct_and_enum_expansions_have_per_instantiation_bounds() {
    for input in [
        quote!(
            struct Wrapper<T> {
                value: T,
            }
        ),
        quote!(
            enum Message<T> {
                Empty,
                Value(T),
                Record { value: T },
            }
        ),
        quote!(
            struct Fixed<T, const N: usize>([T; N]);
        ),
    ] {
        let expansion = owned(quote!(both), input).unwrap();
        syn::parse2::<syn::File>(expansion.clone()).unwrap();
        let text = expansion.to_string();
        assert!(text.contains("InputCodec"));
        assert!(text.contains("OutputCodec"));
        assert!(!text.contains("OnceLock"));
        assert!(text.contains("__nebula_require_codec"));
    }
}

#[test]
fn directional_names_are_independent_and_aliases_are_inbound() {
    let input = syn::parse2(quote! {
        #[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
        struct Value {
            #[serde(rename(serialize = "outgoing", deserialize = "incoming"), alias = "old")]
            actual: String,
        }
    })
    .unwrap();
    let text = crate::derive_property_type::expand(&input)
        .unwrap()
        .to_string();
    assert!(text.contains("incoming"));
    assert!(text.contains("outgoing"));
    assert!(text.contains("old"));
}

#[test]
fn projection_collisions_and_unsupported_serde_fail_at_expansion() {
    for input in [
        quote!(
            struct Value {
                #[serde(rename = "same")]
                first: String,
                #[serde(rename = "same")]
                second: String,
            }
        ),
        quote!(
            struct Value {
                #[serde(alias = "second")]
                first: String,
                second: String,
            }
        ),
        quote!(
            struct Value {
                #[serde(flatten)]
                value: String,
            }
        ),
        quote!(
            struct Value {
                #[serde(with = "adapter")]
                value: String,
            }
        ),
        quote!(
            struct Value {
                #[serde(default)]
                value: String,
            }
        ),
        quote!(
            #[serde(untagged)]
            enum Value {
                A(String),
                B(bool),
            }
        ),
    ] {
        assert!(owned(quote!(both), input).is_err());
    }
}

#[test]
fn root_newtype_option_array_and_primitive_enum_payloads_expand() {
    for input in [
        quote!(
            struct Optional(Option<String>);
        ),
        quote!(
            struct Array(Vec<String>);
        ),
        quote!(
            struct Nothing;
        ),
        quote!(
            enum Payload {
                Number(u32),
                Optional(Option<String>),
                Array(Vec<bool>),
                Empty,
            }
        ),
    ] {
        let expansion = owned(quote!(both), input).unwrap();
        syn::parse2::<syn::File>(expansion).unwrap();
    }
}

#[test]
fn schema_only_description_emits_no_codec_witness() {
    let input = syn::parse2(quote!(
        struct Description {
            value: String,
        }
    ))
    .unwrap();
    let text = crate::derive_property_type::expand(&input)
        .unwrap()
        .to_string();
    assert!(text.contains("PropertyType"));
    assert!(!text.contains("InputCodec"));
    assert!(!text.contains("OutputCodec"));
}

#[test]
fn direct_recursive_schema_uses_graph_lowering_instead_of_global_cache() {
    for input in [
        quote!(
            struct Node {
                next: Option<Box<Node>>,
            }
        ),
        quote!(
            struct Node {
                next: Option<Box<Self>>,
            }
        ),
    ] {
        let input = syn::parse2(input).unwrap();
        assert!(crate::has_direct_recursion(&input));
        let projection = crate::derive_property_type::graph_has_schema(&input).to_string();
        assert!(projection.contains("ValidSchema :: from_graph"));
        assert!(!projection.contains("OnceLock"));
    }
}

#[test]
fn literal_defaults_own_the_factory_and_actual_graph_value() {
    let expansion = owned(
        quote!(input),
        quote!(
            struct Value {
                #[property(input(default = "literal"))]
                value: Option<String>,
                #[field(default = 0.1)]
                precise: f32,
            }
        ),
    )
    .unwrap();
    syn::parse2::<syn::File>(expansion.clone()).unwrap();
    assert!(
        expansion
            .to_string()
            .contains("__nebula_schema_56616c7565_value_0_default")
    );
    let input = syn::parse2(quote!(
        struct Value {
            #[field(default = 0.1)]
            precise: f32,
        }
    ))
    .unwrap();
    let graph = crate::derive_property_type::expand(&input)
        .unwrap()
        .to_string();
    assert!(graph.contains("let __default : f32"));
    assert!(graph.contains("to_value (__default)"));
}

#[test]
fn schema_only_emit_as_does_not_forge_owned_codec() {
    assert!(
        owned(
            quote!(both),
            quote!(
                struct Value {
                    #[field(emit_as = "output")]
                    value: String,
                }
            )
        )
        .is_err()
    );
}
