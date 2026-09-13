use nebula_resource::ResourceConfigInput;

fn main() {
    let schema = nebula_schema::schema_of::<String>().unwrap();
    let values = schema.values_from_wire(serde_json::json!("value")).unwrap();
    let valid = schema.validate(values).unwrap();
    let _input = ResourceConfigInput::authored(valid);
}
