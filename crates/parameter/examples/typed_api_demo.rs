//! Example demonstrating the typed trait-based generic parameter API.
//!
//! The typed API provides type-safe, extensible parameter definitions with
//! robust serde serialization.

use nebula_parameter::typed::{
    Distance, Email, EmailParam, Factor, Number, Password, Percentage, Plain, Port, PortParam,
    Text, Timestamp, Url,
};

fn main() {
    println!("=== Typed API: Generic Parameters with Type-Safe Subtypes ===\n");

    println!("1. Text Parameters\n");

    let name = Text::<Plain>::builder("user_name")
        .label("Full Name")
        .description("User's full name")
        .required()
        .min_length(2)
        .max_length(100)
        .build();

    println!("Plain text parameter:");
    println!("{}\n", serde_json::to_string_pretty(&name).unwrap());

    let email = Text::<Email>::builder("email")
        .label("Email Address")
        .description("Primary email for notifications")
        .required()
        .default_value("user@example.com")
        .build();

    println!("Email parameter (auto-validation enabled):");
    println!("{}\n", serde_json::to_string_pretty(&email).unwrap());

    let homepage = Text::<Url>::builder("homepage")
        .label("Website")
        .description("Personal or company website")
        .build();

    println!("URL parameter:");
    println!("{}\n", serde_json::to_string_pretty(&homepage).unwrap());

    let password = Text::<Password>::builder("api_key")
        .label("API Key")
        .description("Secret API authentication key")
        .required()
        .min_length(32)
        .build();

    println!("Password parameter (auto-marked as sensitive):");
    println!("{}\n", serde_json::to_string_pretty(&password).unwrap());

    println!("\n2. Number Parameters\n");

    let port = Number::<Port>::builder("server_port")
        .label("Server Port")
        .description("Port number for the server")
        .default_value(8080)
        .build();

    println!("Port parameter (auto-range 1-65535):");
    println!("{}\n", serde_json::to_string_pretty(&port).unwrap());

    let opacity = Number::<Percentage>::builder("opacity")
        .label("Opacity")
        .description("Element opacity")
        .default_value(100.0)
        .step(1.0)
        .build();

    println!("Percentage parameter (auto-range 0-100):");
    println!("{}\n", serde_json::to_string_pretty(&opacity).unwrap());

    let scale = Number::<Factor>::builder("scale")
        .label("Scale Factor")
        .description("Scaling multiplier")
        .default_value(1.0)
        .step(0.1)
        .precision(2)
        .build();

    println!("Factor parameter (auto-range 0.0-1.0):");
    println!("{}\n", serde_json::to_string_pretty(&scale).unwrap());

    let created_at = Number::<Timestamp>::builder("created_at")
        .label("Created At")
        .description("Unix timestamp of creation")
        .build();

    println!("Timestamp parameter:");
    println!("{}\n", serde_json::to_string_pretty(&created_at).unwrap());

    let distance = Number::<Distance>::builder("distance")
        .label("Distance")
        .description("Distance in meters")
        .min(0.0)
        .step(0.1)
        .precision(2)
        .build();

    println!("Distance parameter:");
    println!("{}\n", serde_json::to_string_pretty(&distance).unwrap());

    println!("\n3. Using Type Aliases\n");

    let email_alias = EmailParam::builder("contact_email")
        .label("Contact Email")
        .required()
        .build();

    let port_alias = PortParam::builder("database_port")
        .label("Database Port")
        .default_value(5432)
        .build();

    println!("Type alias usage:");
    println!(
        "EmailParam: {}",
        serde_json::to_string(&email_alias).unwrap()
    );
    println!("PortParam: {}", serde_json::to_string(&port_alias).unwrap());

    println!("\n4. Compile-Time Type Safety\n");

    let email_param: Text<Email> = Text::builder("email").build();
    let url_param: Text<Url> = Text::builder("url").build();
    let port_param: Number<Port> = Number::builder("port").build();
    let pct_param: Number<Percentage> = Number::builder("pct").build();

    println!("✓ Type safety enforced at compile time");
    println!("✓ Text<Email> ≠ Text<Url>");
    println!("✓ Number<Port> ≠ Number<Percentage>");

    println!("\nSerialized subtypes:");
    println!(
        "  Email: {}",
        serde_json::to_value(&email_param).unwrap()["subtype"]
    );
    println!(
        "  Url: {}",
        serde_json::to_value(&url_param).unwrap()["subtype"]
    );
    println!(
        "  Port: {}",
        serde_json::to_value(&port_param).unwrap()["subtype"]
    );
    println!(
        "  Percentage: {}",
        serde_json::to_value(&pct_param).unwrap()["subtype"]
    );
}
