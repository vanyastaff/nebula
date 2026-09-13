use nebula_resource::ResourceConfigInput;

fn main() {
    let schema = nebula_schema::schema_of::<String>().unwrap();
    let values = schema.values_from_wire(serde_json::json!("value")).unwrap();
    let resolved = schema.validate(values).unwrap().resolve_data().unwrap();
    let _input = ResourceConfigInput::authored(resolved);
}
