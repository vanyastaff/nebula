fn main() {
    let _ = nebula_schema::Field::secret("token").widget(nebula_schema::StringWidget::Email);
}
