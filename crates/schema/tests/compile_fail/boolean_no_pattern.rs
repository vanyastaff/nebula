fn main() {
    let _ = nebula_schema::Property::boolean(nebula_schema::field_key!("enabled")).pattern("yes|no");
}
