use nebula_macros::Validator;
use nebula_validator::foundation::Validate;

#[derive(Validator, Clone)]
#[validator(message = "user input is invalid")]
pub struct UserInput {
    #[validate(required, min_length = 3, max_length = 32)]
    username: Option<String>,

    #[validate(min = 18, max = 120)]
    age: u8,

    #[validate(min_length = 8)]
    password: String,
}

fn main() {
    let input = UserInput {
        username: Some("alice".to_string()),
        age: 30,
        password: "supersecret".to_string(),
    };

    let _ = input.validate_fields();
    let _ = input.validate(&input);
}
