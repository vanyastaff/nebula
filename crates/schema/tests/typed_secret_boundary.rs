//! Typed secret extraction must borrow protected text until the target owns it.

use std::fmt;

use nebula_schema::{
    AuthoredValue, Field, Schema, SecretInput, SerdeTagging, ValidSchema, field_key, schema_of,
};
use serde::de::{Unexpected, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::json;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Zeroize, ZeroizeOnDrop)]
struct BorrowedSecret(String);

impl BorrowedSecret {
    fn expose(&self) -> &str {
        &self.0
    }
}

impl SecretInput for BorrowedSecret {}

impl fmt::Debug for BorrowedSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BorrowedSecret(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for BorrowedSecret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BorrowedSecretVisitor;

        impl<'de> Visitor<'de> for BorrowedSecretVisitor {
            type Value = BorrowedSecret;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("borrowed protected text")
            }

            fn visit_borrowed_str<E: serde::de::Error>(
                self,
                value: &'de str,
            ) -> Result<Self::Value, E> {
                Ok(BorrowedSecret(value.to_owned()))
            }

            fn visit_str<E: serde::de::Error>(self, _value: &str) -> Result<Self::Value, E> {
                Err(E::custom("secret text was not borrowed"))
            }

            fn visit_string<E: serde::de::Error>(self, _value: String) -> Result<Self::Value, E> {
                Err(E::custom("secret text was materialized as an owned string"))
            }
        }

        deserializer.deserialize_str(BorrowedSecretVisitor)
    }
}

#[derive(Deserialize, Schema)]
struct CredentialInput {
    #[field(secret)]
    token: BorrowedSecret,
    #[field(secret)]
    optional_token: Option<BorrowedSecret>,
}

fn resolved_secret(plaintext: &str) -> nebula_schema::ResolvedValues {
    let schema = schema_of::<CredentialInput>().unwrap();
    schema
        .validate(AuthoredValue::from_data(json!({"token": plaintext})).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap()
}

#[test]
fn typed_secret_is_supplied_as_borrowed_text() {
    let input = resolved_secret("borrow-only-secret")
        .into_typed_exposing_secrets::<CredentialInput>()
        .unwrap();
    assert_eq!(input.token.expose(), "borrow-only-secret");
    assert!(input.optional_token.is_none());
}

struct RejectingSecret;

impl<'de> Deserialize<'de> for RejectingSecret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RejectingVisitor;

        impl<'de> Visitor<'de> for RejectingVisitor {
            type Value = RejectingSecret;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a deliberately rejected secret")
            }

            fn visit_borrowed_str<E: serde::de::Error>(
                self,
                value: &'de str,
            ) -> Result<Self::Value, E> {
                Err(E::invalid_value(Unexpected::Str(value), &self))
            }
        }

        deserializer.deserialize_str(RejectingVisitor)
    }
}

#[derive(Deserialize)]
struct RejectedInput {
    #[expect(dead_code, reason = "deserialization deliberately rejects this field")]
    token: RejectingSecret,
}

#[test]
fn typed_secret_failure_redacts_visitor_diagnostics() {
    let Err(error) =
        resolved_secret("failure-path-secret").into_typed_exposing_secrets::<RejectedInput>()
    else {
        panic!("the rejecting visitor unexpectedly accepted secret input");
    };
    let source = std::error::Error::source(&error)
        .map(ToString::to_string)
        .unwrap_or_default();
    for diagnostic in [format!("{error}"), format!("{error:?}"), source] {
        assert!(!diagnostic.contains("failure-path-secret"));
    }
}

#[derive(Deserialize)]
struct ProjectedCredentialInput {
    #[serde(rename = "wire_name")]
    name: String,
    token: BorrowedSecret,
}

#[test]
fn typed_secret_projection_honors_emit_as_and_read_alias() {
    let schema = Schema::builder()
        .add(
            Field::string(field_key!("name"))
                .read_alias("legacy_name")
                .unwrap()
                .emit_as("wire_name")
                .unwrap(),
        )
        .add(
            Field::secret(field_key!("token"))
                .read_alias("legacy_token")
                .unwrap(),
        )
        .build()
        .unwrap();
    let resolved = schema
        .validate(
            AuthoredValue::from_data(json!({
                "legacy_name": "projected",
                "legacy_token": "borrowed-secret",
            }))
            .unwrap(),
        )
        .unwrap()
        .resolve_data()
        .unwrap();

    let input = resolved
        .into_typed_exposing_secrets::<ProjectedCredentialInput>()
        .unwrap();
    assert_eq!(input.name, "projected");
    assert_eq!(input.token.expose(), "borrowed-secret");
}

#[derive(Deserialize)]
struct NestedSettings {
    #[serde(rename = "wire_name")]
    name: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", content = "body")]
enum NestedCredentialInput {
    Token {
        settings: NestedSettings,
        token: BorrowedSecret,
    },
}

#[test]
fn typed_secret_projection_recurses_through_union_payloads() {
    let schema = ValidSchema::union(
        Field::mode(field_key!("_nebula_union")).variant(
            "Token",
            "Token",
            Field::object(field_key!("token_payload"))
                .add(
                    Field::object(field_key!("settings"))
                        .read_alias("legacy_settings")
                        .unwrap()
                        .add(
                            Field::string(field_key!("name"))
                                .read_alias("legacy_name")
                                .unwrap()
                                .emit_as("wire_name")
                                .unwrap(),
                        ),
                )
                .add(
                    Field::secret(field_key!("token"))
                        .read_alias("legacy_token")
                        .unwrap(),
                ),
        ),
        SerdeTagging::Adjacent {
            tag: "kind".to_owned(),
            content: "body".to_owned(),
        },
    )
    .unwrap();
    let resolved = schema
        .validate(
            schema
                .values_from_wire(json!({
                    "kind": "Token",
                    "body": {
                        "legacy_settings": {"legacy_name": "nested"},
                        "legacy_token": "nested-borrowed-secret",
                    },
                }))
                .unwrap(),
        )
        .unwrap()
        .resolve_data()
        .unwrap();

    let input = resolved
        .into_typed_exposing_secrets::<NestedCredentialInput>()
        .unwrap();
    let NestedCredentialInput::Token { settings, token } = input;
    assert_eq!(settings.name, "nested");
    assert_eq!(token.expose(), "nested-borrowed-secret");
}

#[derive(Deserialize)]
struct NestedModeCredentialInput {
    auth: NestedModeInput,
}

#[derive(Deserialize)]
struct NestedModeInput {
    mode: String,
    value: NestedModePayload,
}

#[derive(Deserialize)]
struct NestedModePayload {
    token: BorrowedSecret,
}

#[test]
fn typed_secret_projection_recurses_through_nested_mode_payloads() {
    let schema = Schema::builder()
        .add(Field::mode(field_key!("auth")).variant(
            "token",
            "Token",
            Field::object(field_key!("token_payload")).add(Field::secret(field_key!("token"))),
        ))
        .build()
        .unwrap();
    let resolved = schema
        .validate(
            AuthoredValue::from_data(json!({
                "auth": {
                    "mode": "token",
                    "value": {"token": "nested-mode-secret"},
                },
            }))
            .unwrap(),
        )
        .unwrap()
        .resolve_data()
        .unwrap();

    let input = resolved
        .into_typed_exposing_secrets::<NestedModeCredentialInput>()
        .unwrap();
    assert_eq!(input.auth.mode, "token");
    assert_eq!(input.auth.value.token.expose(), "nested-mode-secret");
}
