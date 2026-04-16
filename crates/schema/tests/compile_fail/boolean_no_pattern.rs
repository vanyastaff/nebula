fn main() {
    let _ = nebula_schema::Field::boolean("enabled").pattern("yes|no");
}
