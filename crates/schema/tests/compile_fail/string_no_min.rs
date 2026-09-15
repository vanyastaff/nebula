fn main() {
    let _ = nebula_schema::Property::string(nebula_schema::field_key!("name")).min(1);
}
