use nebula_schema::{AuthoredValue, Property, PropertyRef, Schema, field_key};
use serde_json::json;

#[test]
fn property_entrypoints_share_the_admitted_field_contract() {
    let name: Property = Property::string(field_key!("name")).required().into();
    let contact: Property = Property::object(field_key!("contact"))
        .add(Property::string(field_key!("email")).email())
        .into();
    let email_ref = PropertyRef::parse("contact.email").unwrap();

    let schema = Schema::builder()
        .property(name)
        .properties([contact])
        .build()
        .unwrap();

    let valid = schema
        .validate(
            AuthoredValue::from_data(json!({
                "name": "Ada",
                "contact": { "email": "ada@example.com" }
            }))
            .unwrap(),
        )
        .unwrap();

    assert_eq!(
        schema.find_by_path(&email_ref).unwrap().key().as_str(),
        "email"
    );
    assert_eq!(valid.to_wire_json()["name"], json!("Ada"));
}
