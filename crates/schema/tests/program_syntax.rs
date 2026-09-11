//! Authored syntax identity and schema admission remain distinct contracts.

use nebula_expression::{EvaluationContext, ExpressionEngine};
use nebula_schema::{
    AuthoredValue, CompiledValue, EngineExpressionContext, Expression, ExpressionMode, Field,
    ProgramSyntax, Schema, ValuePath, field_key,
};
use serde_json::json;

#[test]
fn exact_source_and_syntax_define_expression_identity() {
    let source = "'{{ 7 }}'";
    let mut expressions = Vec::new();
    let mut ids = Vec::new();
    for (syntax, tag) in [
        (ProgramSyntax::Auto, 0),
        (ProgramSyntax::Expression, 1),
        (ProgramSyntax::Template, 2),
    ] {
        let expression = Expression::with_syntax(source, syntax);
        assert!(!expressions.contains(&expression));
        let tree = AuthoredValue::Expression(expression.clone());
        let mut expected = b"nbschema-value-v\x00\x02\x08".to_vec();
        expected.extend_from_slice(&[tag, 9]);
        expected.extend_from_slice(source.as_bytes());
        assert_eq!(tree.canonical_bytes().unwrap(), expected);
        let id = tree.content_id().unwrap();
        assert!(!ids.contains(&id));
        ids.push(id);
        expressions.push(expression);
    }
    assert_ne!(
        Expression::template(source),
        Expression::template(format!(" {source} "))
    );
}

#[test]
fn clones_share_compilation_without_sharing_different_syntax() {
    for syntax in [
        ProgramSyntax::Auto,
        ProgramSyntax::Expression,
        ProgramSyntax::Template,
    ] {
        let expression = Expression::with_syntax("'{{ 7 }}'", syntax);
        let clone = expression.clone();
        assert!(std::ptr::eq(
            expression.parse().unwrap(),
            clone.parse().unwrap()
        ));
        assert_eq!(clone.parse().unwrap().syntax(), syntax);
    }
    let auto = Expression::new("'{{ incomplete'");
    let template = Expression::template(auto.source());
    let clone = template.clone();
    let error = template
        .parse_at(&ValuePath::root().push("one"))
        .unwrap_err();
    assert_eq!(error.path().to_string(), "/one");
    let error = clone.parse_at(&ValuePath::root().push("two")).unwrap_err();
    assert_eq!(error.path().to_string(), "/two");
    assert_eq!(template.parse().unwrap_err().path(), &ValuePath::root());
    assert_eq!(auto.parse().unwrap().syntax(), ProgramSyntax::Auto);
}

#[tokio::test]
async fn required_templates_keep_string_semantics_through_preparation_and_wire() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("value")).expression_mode(ExpressionMode::Required))
        .build()
        .unwrap();
    let mut authored = AuthoredValue::object();
    authored
        .insert(
            "value",
            AuthoredValue::Expression(Expression::template("{{ 7 }}")),
        )
        .unwrap();
    let prepared = schema.validate(authored).unwrap();
    let CompiledValue::Expression(program) = prepared.get(&field_key!("value")).unwrap() else {
        panic!("preparation must retain the admitted program");
    };
    assert_eq!(program.syntax(), ProgramSyntax::Template);
    let decoded: AuthoredValue =
        serde_json::from_slice(&serde_json::to_vec(prepared.values()).unwrap()).unwrap();
    let context = EngineExpressionContext::with_input(json!(null));
    for prepared in [prepared, schema.validate(decoded).unwrap()] {
        assert_eq!(
            prepared
                .resolve(&context)
                .await
                .unwrap()
                .get(&field_key!("value")),
            Some(&json!("7"))
        );
    }
    let error = schema
        .validate(AuthoredValue::from_data(json!({"value": "7"})).unwrap())
        .unwrap_err();
    assert!(
        error
            .errors()
            .any(|error| error.code() == "expression.required")
    );
}

#[tokio::test]
async fn template_result_still_obeys_final_type_validation() {
    let schema = Schema::builder()
        .add(Field::number(field_key!("value")))
        .build()
        .unwrap();
    let context = EngineExpressionContext::with_input(json!(null));
    for expression in [Expression::new("{{ 7 }}"), Expression::template("{{ 7 }}")] {
        let syntax = expression.syntax();
        let mut authored = AuthoredValue::object();
        authored
            .insert("value", AuthoredValue::Expression(expression))
            .unwrap();
        let result = schema.validate(authored).unwrap().resolve(&context).await;
        if syntax == ProgramSyntax::Template {
            let error = result.unwrap_err();
            assert!(
                error
                    .errors()
                    .any(|error| error.code() == "expression.type_mismatch"
                        && error.path().to_string() == "/value")
            );
        } else {
            assert_eq!(result.unwrap().get(&field_key!("value")), Some(&json!(7)));
        }
    }
}

#[test]
fn every_syntax_obeys_forbidden_and_data_only_boundaries() {
    let forbidden = Schema::builder()
        .add(Field::string(field_key!("value")).no_expression())
        .build()
        .unwrap();
    let allowed = Schema::builder()
        .add(Field::string(field_key!("value")))
        .build()
        .unwrap();
    for syntax in [
        ProgramSyntax::Auto,
        ProgramSyntax::Expression,
        ProgramSyntax::Template,
    ] {
        let mut authored = AuthoredValue::object();
        authored
            .insert(
                "value",
                AuthoredValue::Expression(Expression::with_syntax("'text'", syntax)),
            )
            .unwrap();
        let error = forbidden.validate(authored.clone()).unwrap_err();
        assert!(
            error
                .errors()
                .any(|error| error.code() == "expression.forbidden")
        );
        let error = allowed
            .validate(authored)
            .unwrap()
            .resolve_data()
            .unwrap_err();
        assert!(
            error
                .errors()
                .any(|error| error.code() == "expression.forbidden")
        );
    }
}

#[test]
fn template_shorthand_keeps_auto_and_data_ingress_keeps_literal() {
    let engine = ExpressionEngine::new();
    for data in [json!("{{ 7 }}"), json!({"$expr": "{{ 7 }}"})] {
        let AuthoredValue::Expression(expression) =
            AuthoredValue::from_template_json(data.clone()).unwrap()
        else {
            panic!("explicit shorthand must author an AUTO program");
        };
        assert_eq!(expression.syntax(), ProgramSyntax::Auto);
        assert_eq!(
            engine
                .evaluate_compiled(expression.parse().unwrap(), &EvaluationContext::new())
                .unwrap(),
            json!(7)
        );
        assert_eq!(
            AuthoredValue::from_data(data.clone()).unwrap().to_json(),
            data
        );
    }
}

#[tokio::test]
async fn template_results_remain_data_instead_of_reentering_authoring() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("value")))
        .build()
        .unwrap();
    let mut authored = AuthoredValue::object();
    authored
        .insert(
            "value",
            AuthoredValue::Expression(Expression::template("{{ $input }}")),
        )
        .unwrap();
    let resolved = schema
        .validate(authored)
        .unwrap()
        .resolve(&EngineExpressionContext::with_input(json!("{{ 7 }}")))
        .await
        .unwrap();
    assert_eq!(resolved.get(&field_key!("value")), Some(&json!("{{ 7 }}")));
}
