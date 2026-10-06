use super::{BindingPolicy, ReceiverKind, SlotKind, parse_slots};
use syn::{DeriveInput, parse_quote};

fn parse(source: &str, receiver: ReceiverKind) -> syn::Result<Vec<super::SlotField>> {
    let input: DeriveInput = syn::parse_str(source)?;
    let syn::Data::Struct(data) = input.data else {
        panic!("test requires a struct");
    };
    parse_slots(&data.fields, receiver)
}

#[test]
fn binding_requiredness_is_independent_of_action_absence_wrapper() {
    let slots = parse("struct A { #[slot(credential)] auth: CredentialGuard<S>, #[slot(resource)] pool: Option<ResourceHandle<R>>, #[slot(credential, binding = required, bind_when(condition(use_auth)))] conditional: Option<CredentialGuard<S>> }", ReceiverKind::Action).expect("valid slots");
    assert_eq!(slots.len(), 3);
    assert_eq!(slots[0].binding, BindingPolicy::Required);
    assert_eq!(slots[0].kind, SlotKind::Credential);
    assert!(!slots[0].optional);
    assert_eq!(slots[1].binding, BindingPolicy::Optional);
    assert!(slots[1].optional);
    assert_eq!(slots[2].binding, BindingPolicy::Required);
    assert!(slots[2].optional);
    assert!(slots[2].bind_when.is_some());
}

#[test]
fn resource_cells_carry_scheme_syntax_without_inferred_provider_identity() {
    let slots = parse("struct R { #[slot(credential)] auth: CredentialSlot<S>, #[slot(credential, key = \"second.auth\", purpose = \"secondary\", binding = optional, bind_when(eq(root(\"/enabled\"), true)))] second: nebula_resource::SlotCell<nebula_credential::CredentialGuard<S>> }", ReceiverKind::Resource).expect("valid cells");
    assert_eq!(slots[0].binding, BindingPolicy::Required);
    assert_eq!(slots[1].binding, BindingPolicy::Optional);
    assert_eq!(slots[1].key, "second.auth");
    let scheme: syn::Type = parse_quote!(S);
    assert_eq!(slots[0].inner_type, scheme);
    assert_eq!(slots[1].inner_type, scheme);
}

#[test]
fn serde_rename_does_not_readdress_slot() {
    let slots = parse(
        "struct A { #[serde(rename = \"other\")] #[slot(credential)] auth: CredentialGuard<S> }",
        ReceiverKind::Action,
    )
    .expect("valid slot");
    assert_eq!(slots[0].key, "auth");
}

#[test]
fn rejects_unknown_duplicate_conflicting_and_malformed_arguments() {
    for declaration in [
        "credential, optional",
        "credential, scheme = S",
        "credential, provider = C",
        "credential, key = \"a\", key = \"b\"",
        "credential, resource",
        "credential, binding = required, binding = optional",
        "credential, purpose = \"a\", purpose = \"b\"",
        "credential, bind_when(condition(a)), bind_when(condition(b))",
        "credential, binding = true",
        "credential, key = \"\"",
        "credential, key = some_function()",
        "credential, bind_when(condition(a), condition(b))",
        "",
    ] {
        let source =
            format!("struct A {{ #[slot({declaration})] auth: Option<CredentialGuard<S>> }}");
        assert!(
            parse(&source, ReceiverKind::Action).is_err(),
            "accepted {declaration}"
        );
    }
    for helper in [
        "property(input(required))",
        "field(label = \"x\")",
        "validate(required)",
        "credential",
        "resource",
        "slot(credential)",
    ] {
        let source =
            format!("struct A {{ #[slot(credential)] #[{helper}] auth: CredentialGuard<S> }}");
        assert!(
            parse(&source, ReceiverKind::Action).is_err(),
            "accepted conflicting {helper}"
        );
    }
}

#[test]
fn duplicate_keys_across_kinds_are_refused() {
    let source = "struct A { #[slot(credential, key = \"same\")] auth: CredentialGuard<S>, #[slot(resource, key = \"same\")] resource: ResourceHandle<R> }";
    assert!(parse(source, ReceiverKind::Action).is_err());
}

#[test]
fn rejects_unsupported_receiver_wrappers_and_absence_contracts() {
    for receiver in [
        "Lazy<CredentialGuard<S>>",
        "Option<Lazy<CredentialGuard<S>>>",
        "Lazy<Option<CredentialGuard<S>>>",
        "Alias<S>",
        "&CredentialGuard<S>",
        "Option<Option<CredentialGuard<S>>>",
        "CredentialGuard<S, Other>",
    ] {
        let source = format!("struct A {{ #[slot(credential)] auth: {receiver} }}");
        assert!(
            parse(&source, ReceiverKind::Action).is_err(),
            "accepted {receiver}"
        );
    }
    for declaration in [
        "credential, binding = optional",
        "credential, bind_when(condition(a))",
    ] {
        let source = format!("struct A {{ #[slot({declaration})] auth: CredentialGuard<S> }}");
        assert!(parse(&source, ReceiverKind::Action).is_err());
    }
    for receiver in [
        "Option<CredentialSlot<S>>",
        "Option<SlotCell<CredentialGuard<S>>>",
        "CredentialGuard<S>",
    ] {
        let source = format!("struct R {{ #[slot(credential)] auth: {receiver} }}");
        assert!(parse(&source, ReceiverKind::Resource).is_err());
    }
    assert!(
        parse(
            "struct R { #[slot(resource)] pool: ResourceHandle<R> }",
            ReceiverKind::Resource
        )
        .is_err()
    );
    assert!(
        parse(
            "struct Properties { #[slot(credential)] auth: CredentialGuard<S> }",
            ReceiverKind::CredentialProperties
        )
        .is_err()
    );
    assert!(
        parse(
            "struct A(#[slot(credential)] CredentialGuard<S>);",
            ReceiverKind::Action
        )
        .is_err()
    );
    for receiver in [
        "ResourceGuard<R>",
        "Option<ResourceGuard<R>>",
        "Lazy<ResourceGuard<R>>",
    ] {
        let source = format!("struct A {{ #[slot(resource)] pool: {receiver} }}");
        assert!(parse(&source, ReceiverKind::Action).is_err());
    }
}

#[test]
fn only_the_declared_condition_dsl_is_accepted() {
    for condition in [
        "condition(use_auth)",
        "eq(root(\"/auth\"), true)",
        "all(is_true(root(\"/enabled\")), not(is_false(root(\"/other\"))))",
        "any(ne(root(\"/name\"), \"anonymous\"), gte(root(\"/count\"), -1))",
        "one_of(root(\"/mode\"), [\"a\", \"b\", null, 2])",
        "lt(root(\"/limit~1value\"), 2.5)",
    ] {
        let source = format!(
            "struct A {{ #[slot(credential, bind_when({condition}))] auth: Option<CredentialGuard<S>> }}"
        );
        assert!(
            parse(&source, ReceiverKind::Action).is_ok(),
            "rejected {condition}"
        );
    }
    for condition in [
        "true",
        "|| true",
        "arbitrary()",
        "some::condition(a)",
        "condition(a, b)",
        "condition(\"name\")",
        "all()",
        "any()",
        "not()",
        "one_of(root(\"/a\"), [])",
        "eq(field(enabled), true)",
        "eq(root(\"no_slash\"), true)",
        "eq(root(\"/bad~2\"), true)",
        "eq(root(\"/*\"), true)",
        "eq(root(dynamic()), true)",
        "eq(root(\"/a\"), compute())",
        "gt(root(\"/a\"), \"number\")",
        "lt(root(\"/a\"), 1e999)",
        "eq(root(\"/a\"), 1u64)",
        "eq(root(\"/a\"), - -1)",
        "present(root(\"/a\"))",
        "empty(root(\"/a\"))",
        "ge(root(\"/a\"), 1)",
    ] {
        let source = format!(
            "struct A {{ #[slot(credential, bind_when({condition}))] auth: Option<CredentialGuard<S>> }}"
        );
        assert!(
            parse(&source, ReceiverKind::Action).is_err(),
            "accepted {condition}"
        );
    }
}

#[test]
fn declaration_addresses_are_preserved_without_provider_key_restrictions() {
    for address in [
        "a..b",
        "bad_",
        "привет",
        "scope/auth",
        "a long local declaration address that deliberately exceeds sixty four bytes",
    ] {
        let source = format!(
            "struct A {{ #[slot(credential, key = {address:?})] auth: CredentialGuard<S> }}"
        );
        let slots = parse(&source, ReceiverKind::Action).expect("local address");
        assert_eq!(slots[0].key, address);
    }
    let slots = parse(
        "struct A { #[slot(credential)] r#type: CredentialGuard<S> }",
        ReceiverKind::Action,
    )
    .expect("raw Rust identifier");
    assert_eq!(slots[0].key, "type");
}

#[test]
fn migration_diagnostics_survive_shared_parser_routing() {
    for wrapper in [
        "Lazy<CredentialGuard<S>>",
        "Option<Lazy<CredentialGuard<S>>>",
        "Lazy<Option<CredentialGuard<S>>>",
    ] {
        let source = format!("struct A {{ #[slot(credential)] auth: {wrapper} }}");
        let error = parse(&source, ReceiverKind::Action)
            .expect_err("lazy rejected")
            .to_string();
        assert!(error.contains("lazy credential slots are unsupported"));
        assert!(error.contains("resolved eagerly"));
    }
    for wrapper in [
        "ResourceGuard<R>",
        "Option<ResourceGuard<R>>",
        "Lazy<Option<ResourceGuard<R>>>",
        "Option<Lazy<ResourceGuard<R>>>",
    ] {
        let source = format!("struct A {{ #[slot(resource)] pool: {wrapper} }}");
        let error = parse(&source, ReceiverKind::Action)
            .expect_err("lease rejected")
            .to_string();
        assert!(error.contains("removed in 0.27.0"));
        assert!(error.contains("effect journal"));
        assert!(error.contains("ResourceHandle"));
    }
    for (wrapper, expected) in [
        ("ManagedRow<R>", "was renamed to `ResourceHandle`"),
        (
            "Option<Lazy<ResourceHandle<R>>>",
            "acquires nothing at resolution",
        ),
    ] {
        let source = format!("struct A {{ #[slot(resource)] pool: {wrapper} }}");
        assert!(
            parse(&source, ReceiverKind::Action)
                .expect_err("migration")
                .to_string()
                .contains(expected)
        );
    }
}

#[test]
fn shared_condition_syntax_distinguishes_schema_and_receiver_scope() {
    use super::{ConditionScope, validate_condition};
    let local: syn::Expr = parse_quote!(all(eq(field(mode), "active"), condition(other)));
    assert!(validate_condition(&local, ConditionScope::LocalRecord).is_ok());
    assert!(validate_condition(&local, ConditionScope::AssociatedData).is_err());
    for malformed in [
        "eq(field(\"name\"), true)",
        "eq(field(other::name), true)",
        "eq(field(), true)",
    ] {
        let expression = syn::parse_str(malformed).expect("Rust syntax");
        assert!(validate_condition(&expression, ConditionScope::LocalRecord).is_err());
    }
}

#[test]
fn condition_integer_literals_fit_json_number_representation() {
    use super::{ConditionScope, validate_condition};
    for value in ["18446744073709551615", "-9223372036854775808"] {
        let expression = syn::parse_str(&format!("eq(root(\"/n\"), {value})")).expect("syntax");
        assert!(validate_condition(&expression, ConditionScope::AssociatedData).is_ok());
    }
    for value in ["18446744073709551616", "-9223372036854775809"] {
        let expression = syn::parse_str(&format!("eq(root(\"/n\"), {value})")).expect("syntax");
        assert!(validate_condition(&expression, ConditionScope::AssociatedData).is_err());
    }
}
