// `nebula_schema::ValidValues` is a proof token mintable only via
// `ValidSchema::validate` — it must expose no public constructor.
fn main() {
    let _ = nebula_schema::ValidValues::new();
}
