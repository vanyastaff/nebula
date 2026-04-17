use nebula_schema::field_key;

fn main() {
    // digit start is invalid
    let _ = field_key!("1bad");
}
