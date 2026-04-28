fn main() {
    let _ = nebula_schema::Field::secret(nebula_schema::field_key!("token")).widget(nebula_schema::StringWidget::Email);
}
