use nebula_schema::{FieldCollector, Schema, StringWidget};

fn main() {
    // StringWidget::Multiline is specific to StringField; NumberField takes NumberWidget.
    let _ = Schema::builder()
        .number("n", |n| n.widget(StringWidget::Multiline))
        .build();
}
