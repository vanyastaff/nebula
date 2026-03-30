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

#[derive(Validator, Clone)]
pub struct ContactInfo {
    #[validate(email)]
    email: String,

    #[validate(url)]
    website: String,

    #[validate(email)]
    reply_to: Option<String>,
}

#[derive(Validator, Clone)]
pub struct NetworkConfig {
    #[validate(ipv4)]
    host_v4: String,

    #[validate(ipv6)]
    host_v6: String,

    #[validate(ip_addr)]
    host: String,

    #[validate(hostname)]
    fqdn: String,

    #[validate(uuid)]
    id: String,

    #[validate(ipv4)]
    override_ip: Option<String>,
}

#[derive(Validator, Clone)]
pub struct ScheduleConfig {
    #[validate(date)]
    start_date: String,

    #[validate(date_time)]
    created_at: String,

    #[validate(time)]
    daily_at: String,

    #[validate(date)]
    end_date: Option<String>,
}

#[derive(Validator, Clone)]
pub struct RegexConfig {
    #[validate(regex = r"^\d{4}$")]
    code: String,

    #[validate(regex = r"^[a-z]+$")]
    slug: Option<String>,
}

fn main() {
    let input = UserInput {
        username: Some("alice".to_string()),
        age: 30,
        password: "supersecret".to_string(),
    };
    let _ = input.validate_fields();
    let _ = input.validate(&input);

    let contact = ContactInfo {
        email: "user@example.com".to_string(),
        website: "https://example.com".to_string(),
        reply_to: None,
    };
    let _ = contact.validate_fields();

    let net = NetworkConfig {
        host_v4: "192.168.0.1".to_string(),
        host_v6: "::1".to_string(),
        host: "10.0.0.1".to_string(),
        fqdn: "example.com".to_string(),
        id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        override_ip: None,
    };
    let _ = net.validate_fields();

    let sched = ScheduleConfig {
        start_date: "2024-01-15".to_string(),
        created_at: "2024-01-15T10:30:00Z".to_string(),
        daily_at: "08:00:00".to_string(),
        end_date: None,
    };
    let _ = sched.validate_fields();

    let re = RegexConfig {
        code: "1234".to_string(),
        slug: Some("hello".to_string()),
    };
    let _ = re.validate_fields();
}
