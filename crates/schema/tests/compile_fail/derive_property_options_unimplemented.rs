use nebula_schema::Schema;

#[derive(Schema)]
struct Bad {
    #[property(options(source = crate::load_regions))]
    region: String,
}

fn main() {}
