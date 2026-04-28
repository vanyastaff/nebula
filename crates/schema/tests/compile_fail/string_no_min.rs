fn main() {
    let _ = nebula_schema::Field::string(nebula_schema::field_key!("name")).min(1);
}
