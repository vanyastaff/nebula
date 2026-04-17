use nebula_schema::FieldKey;

fn main() {
    // Should not compile — From<&'static str> is removed
    let _: FieldKey = "alpha".into();
}
