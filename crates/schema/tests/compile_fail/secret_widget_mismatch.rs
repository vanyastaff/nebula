fn main() {
    // A secret field's `.widget()` takes a `SecretWidget`, so passing a
    // `StringWidget` is a type mismatch (the widgets are not interchangeable).
    let _ = nebula_schema::Field::secret(nebula_schema::field_key!("token"))
        .widget(nebula_schema::StringWidget::Plain);
}
