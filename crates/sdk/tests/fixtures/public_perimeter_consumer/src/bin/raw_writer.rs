use std::sync::Arc;

fn name_raw_writer(_: Option<Arc<dyn nebula_sdk::integration::credential::CredentialPersistence>>) {
}

fn main() {
    name_raw_writer(None);
}
