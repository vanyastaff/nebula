use nebula_schema::{FieldCollector, Schema, StringWidget};

fn main() {
    // StringWidget::Multiline is specific to StringField; NumberField takes NumberWidget.
    let _ = Schema::builder()
        .number(nebula_schema::field_key!("n"), |n| n.widget(StringWidget::Multiline))
        .build();
}
