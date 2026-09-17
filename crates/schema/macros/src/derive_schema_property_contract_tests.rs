use super::expand;

fn rejects(source: &str, diagnostic: &str) {
    let result = expand(syn::parse_str(source).expect("test declaration"));
    let error = result.expect_err("invalid declaration must fail expansion");
    assert!(error.to_string().contains(diagnostic), "{error}");
}

#[test]
fn duplicate_rule_entries_are_rejected() {
    for rule in [
        "length(min = 8), length(max = 64)",
        "length(min = 8, min = 1)",
        "range(1..=8), range(..=64)",
        "pattern = \"^a\", pattern = \".*\"",
        "url, url",
        "non_empty, non_empty",
    ] {
        rejects(
            &format!("struct C {{ #[property(validate({rule}))] x: String }}"),
            "duplicate",
        );
    }
}

#[test]
fn duplicate_display_and_input_entries_are_rejected() {
    for section in [
        "display(label = \"a\", label = \"b\")",
        "display(hidden, hidden)",
        "input(secret, secret)",
        "input(required, required)",
        "input(expressions = forbidden, expressions = allowed)",
    ] {
        rejects(
            &format!("struct C {{ #[property({section})] x: String }}"),
            "duplicate",
        );
    }
}

#[test]
fn mixed_namespace_duplicates_are_rejected() {
    for attrs in [
        "#[field(secret)] #[property(input(secret))]",
        "#[validate(required)] #[property(input(required))]",
        "#[validate(url)] #[property(validate(url))]",
        "#[validate(length(min = 8))] #[property(validate(length(max = 64)))]",
        "#[field(label = \"a\", label = \"b\")]",
        "#[validate(pattern = \"^a\", pattern = \".*\")]",
    ] {
        rejects(&format!("struct C {{ {attrs} x: String }}"), "duplicate");
    }
}

#[test]
fn mixed_namespace_mode_conflicts_are_rejected() {
    for attrs in [
        "#[field(no_expression)] #[property(input(expressions = allowed))]",
        "#[field(expression_required)] #[property(input(expressions = allowed))]",
        "#[field(multiline)] #[property(display(widget = text))]",
    ] {
        rejects(&format!("struct C {{ {attrs} x: String }}"), "conflict");
    }
}

#[test]
fn property_flags_do_not_accept_arguments() {
    for section in [
        "input(required(false))",
        "input(secret(false))",
        "display(hidden(false))",
    ] {
        rejects(
            &format!("struct C {{ #[property({section})] x: String }}"),
            "arguments",
        );
    }
}

#[test]
fn empty_rule_bounds_are_rejected() {
    for rule in ["length()", "range(..)"] {
        rejects(
            &format!("struct C {{ #[property(validate({rule}))] x: String }}"),
            "bound",
        );
    }
}

#[test]
fn incompatible_value_rules_are_rejected() {
    for (rule, ty) in [
        ("range(1..=3)", "String"),
        ("pattern = \"^a\"", "u32"),
        ("url", "bool"),
        ("email", "Vec<String>"),
        ("length(min = 1)", "bool"),
    ] {
        rejects(
            &format!("struct C {{ #[property(validate({rule}))] x: {ty} }}"),
            "applies only",
        );
    }
}

#[test]
fn container_and_variant_secret_intent_is_rejected() {
    for source in [
        "#[property(input(secret))] struct C { x: String }",
        "#[field(secret)] struct C;",
        "enum C { #[property(input(secret))] Token { x: String } }",
        "enum C { Token(#[property(input(secret))] Payload) }",
        "enum C { Token(#[field(secret)] Payload) }",
    ] {
        rejects(source, "not supported");
    }
}

#[test]
fn skipped_secret_intent_is_rejected() {
    for source in [
        "struct C { #[serde(skip)] #[property(input(secret))] x: String }",
        "struct C { #[field(skip, secret)] x: String }",
        "enum C { #[serde(skip)] #[property(input(secret))] Token { x: String }, Other }",
        "enum C { #[serde(skip)] Token { #[property(input(secret))] x: String }, Other }",
        "enum C { Token { #[serde(skip_deserializing)] #[field(secret)] x: String } }",
    ] {
        rejects(source, "skipped");
    }
}

#[test]
fn non_empty_combines_with_an_explicit_minimum() {
    let input = syn::parse_quote! {
        struct C {
            #[property(validate(non_empty, length(min = 8, max = 64)))]
            x: Option<String>,
        }
    };
    expand(input).expect("compatible constraints must expand");
}

#[test]
fn collection_constraints_accept_lists_and_optional_lists() {
    for ty in ["Vec<u8>", "Option<Vec<u8>>", "Vec<Payload>"] {
        for rules in [
            "items(min = 1, max = 8), unique",
            "items(min = 0)",
            "items(max = 4294967295)",
            "unique",
        ] {
            let input = syn::parse_str(&format!(
                "struct C {{ #[property(validate({rules}))] values: {ty} }}"
            ))
            .expect("test declaration");
            expand(input).expect("supported collection constraints must expand");
        }
    }
}

#[test]
fn collection_constraints_reject_duplicate_settings() {
    for rules in [
        "items(min = 1), items(max = 8)",
        "items(min = 1, min = 0)",
        "items(max = 8, max = 9)",
        "unique, unique",
    ] {
        rejects(
            &format!("struct C {{ #[property(validate({rules}))] values: Vec<u8> }}"),
            "duplicate",
        );
    }
}

#[test]
fn collection_constraints_reject_non_list_types() {
    for ty in ["String", "Option<String>", "u8", "bool", "Payload"] {
        for rules in ["items(min = 1)", "unique"] {
            rejects(
                &format!("struct C {{ #[property(validate({rules}))] values: {ty} }}"),
                "applies only to list properties",
            );
        }
    }
}

#[test]
fn collection_counts_reject_invalid_bounds() {
    for (bounds, diagnostic) in [
        ("", "requires at least one bound"),
        ("min = 2, max = 1", "minimum exceeds maximum"),
        ("min = 4294967296", "u32"),
        ("max = 4294967296", "u32"),
        ("min = -1", "non-negative integer literal"),
        ("max = 1.5", "non-negative integer literal"),
        ("other = 1", "must be `min` or `max`"),
    ] {
        rejects(
            &format!("struct C {{ #[property(validate(items({bounds})))] values: Vec<u8> }}"),
            diagnostic,
        );
    }
    rejects(
        "struct C { #[property(validate(unique(false)))] values: Vec<u8> }",
        "does not accept arguments",
    );
}

#[test]
fn union_container_schema_attributes_are_not_ignored() {
    for attribute in ["reserved(\"Old\")", "custom = \"validate\"", "unknown", ""] {
        rejects(
            &format!("#[schema({attribute})] enum C {{ Old, New }}"),
            "not supported on enums",
        );
    }
}

#[test]
fn decorator_emission_order_matches_pipeline() {
    // Source-attribute order is deliberately REVERSED relative to the
    // pipeline: the derive parses `#[field]`/`#[property]` into a struct, so
    // emitted decorator order must be the fixed pipeline order regardless of
    // how the attributes are written.
    let input = syn::parse_str(
        "struct C { \
         #[property(display(hidden, group = \"G\", hint = \"text\", \
         placeholder = \"P\", description = \"D\", label = \"L\"))] \
         #[field(default = \"x\")] \
         x: String }",
    )
    .expect("test declaration");
    let output = expand(input).expect("decorated field must expand");
    // `quote!` separates tokens with spaces; collapse them so method-call
    // markers are searchable as written.
    let rendered = output.to_string().replace(' ', "");
    let mut cursor = 0;
    let mut positions = Vec::new();
    for marker in [
        ".label(",
        ".description(",
        ".placeholder(",
        ".default(",
        ".hint(",
        ".group(",
        ".visible(",
    ] {
        let start = rendered[cursor..]
            .find(marker)
            .unwrap_or_else(|| panic!("marker `{marker}` not found in generated tokens"));
        positions.push(cursor + start);
        cursor += start + marker.len();
    }
    assert!(
        positions.windows(2).all(|window| window[0] < window[1]),
        "decorator markers are not in strictly increasing order: {positions:?}"
    );
}
