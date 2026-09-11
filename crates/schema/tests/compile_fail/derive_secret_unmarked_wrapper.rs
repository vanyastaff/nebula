use nebula_schema::{HasSchema, Schema};
use serde::Deserialize;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct UnmarkedSecret(String);

#[derive(Deserialize, Schema)]
struct CredentialInput {
    #[field(secret)]
    token: UnmarkedSecret,
}

fn main() {
    let _ = CredentialInput::schema();
}
