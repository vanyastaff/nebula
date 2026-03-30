use nebula_macros::Validator;
use nebula_validator::foundation::{Validate, ValidationError};

fn must_be_even(value: &u32) -> Result<(), ValidationError> {
    if value % 2 == 0 {
        Ok(())
    } else {
        Err(ValidationError::new("not_even", "value must be even"))
    }
}

#[derive(Validator, Clone)]
struct NestedItem {
    #[validate(min_length = 2)]
    name: String,
}

#[derive(Validator, Clone)]
struct CollectionRules {
    #[validate(each(email))]
    emails: Vec<String>,

    #[validate(each(not_empty))]
    non_empty: Vec<String>,

    #[validate(each(exact_length = 3))]
    short_codes: Vec<String>,

    #[validate(each(contains = "-"))]
    dashed: Vec<String>,

    #[validate(each(starts_with = "ab", ends_with = "c"))]
    prefixed_and_suffixed: Vec<String>,

    #[validate(each(min = 1, max = 10))]
    scores: Vec<u32>,

    #[validate(each(url))]
    webhooks: Option<Vec<String>>,

    #[validate(each(nested))]
    nested: Vec<NestedItem>,

    #[validate(each(custom = must_be_even))]
    even_numbers: Vec<u32>,
}

fn main() {
    let input = CollectionRules {
        emails: vec!["user@example.com".to_string()],
        non_empty: vec!["x".to_string()],
        short_codes: vec!["abc".to_string()],
        dashed: vec!["a-b".to_string()],
        prefixed_and_suffixed: vec!["abzzc".to_string()],
        scores: vec![2, 4, 8],
        webhooks: Some(vec!["https://example.com/hook".to_string()]),
        nested: vec![NestedItem {
            name: "ok".to_string(),
        }],
        even_numbers: vec![2, 4, 6],
    };

    let _ = input.validate_fields();
    let _ = input.validate(&input);
}