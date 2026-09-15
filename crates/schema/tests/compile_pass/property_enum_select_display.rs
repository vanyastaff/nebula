use nebula_schema::{EnumSelect, HasSelectOptions};

#[derive(EnumSelect)]
enum Choice {
    #[property(display(label = "First choice", description = "A labeled option"))]
    First,
}

fn main() {
    let options = Choice::select_options();
    assert_eq!(options[0].label, "First choice");
    assert_eq!(options[0].description.as_deref(), Some("A labeled option"));
}
