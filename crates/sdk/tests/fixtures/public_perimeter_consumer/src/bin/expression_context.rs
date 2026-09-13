use nebula_sdk::prelude;

fn accepts_context(_: &dyn prelude::ExpressionContext) {}

fn main() {
    let _ = accepts_context;
}
