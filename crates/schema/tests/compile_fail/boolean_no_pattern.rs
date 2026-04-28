fn main() {
    let _ = nebula_schema::Field::boolean(nebula_schema::field_key!("enabled")).pattern("yes|no");
}
