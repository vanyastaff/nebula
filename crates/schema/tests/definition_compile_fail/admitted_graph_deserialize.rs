use nebula_schema::AdmittedSchemaGraph;

fn main() {
    let _: AdmittedSchemaGraph = serde_json::from_str("{}").unwrap();
}
