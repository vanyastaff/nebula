//! Example demonstrating the typed parameter API with various subtypes.
//!
//! Run with: `cargo run --example typed_subtypes`

use nebula_parameter::subtype::traits::{BooleanSubtype, NumberSubtype, TextSubtype};
use nebula_parameter::typed::prelude::*;

fn main() {
    println!("=== Typed Parameter API - Subtype Examples ===\n");

    // ── Text-based subtypes ──────────────────────────────────────────────────

    println!("📝 Text Subtypes:");

    let email = Text::<Email>::builder("user_email")
        .label("Email Address")
        .required()
        .build();
    println!("  Email: {:?}", email.metadata.key);

    let url = Text::<Url>::builder("homepage")
        .label("Website URL")
        .build();
    println!(
        "  URL: {:?} (placeholder: {:?})",
        url.metadata.key,
        Url::placeholder()
    );

    let password = Text::<Password>::builder("api_key")
        .label("API Key")
        .build();
    println!(
        "  Password: {:?} (sensitive: {})",
        password.metadata.key,
        Password::is_sensitive()
    );

    // ── Code subtypes (as text) ──────────────────────────────────────────────

    println!("\n💻 Code Subtypes (as Text):");

    let js_code = Text::<JavaScript>::builder("transform_fn")
        .label("Transform Function")
        .default_value("(x) => x * 2")
        .build();
    println!(
        "  JavaScript: {:?} (multiline: {})",
        js_code.metadata.key,
        JavaScript::is_multiline()
    );

    let python_code = Text::<Python>::builder("handler")
        .label("Python Handler")
        .build();
    println!("  Python: {:?}", python_code.metadata.key);

    let sql = Text::<Sql>::builder("query")
        .label("Database Query")
        .default_value("SELECT * FROM users")
        .build();
    println!("  SQL: {:?}", sql.metadata.key);

    let yaml = Text::<Yaml>::builder("config")
        .label("YAML Configuration")
        .build();
    println!("  YAML: {:?}", yaml.metadata.key);

    // ── Color subtypes (as text) ─────────────────────────────────────────────

    println!("\n🎨 Color Subtypes (as Text):");

    let hex = Text::<HexColor>::builder("brand_color")
        .label("Brand Color")
        .default_value("#FF5733")
        .build();
    println!(
        "  Hex: {:?} (pattern: {})",
        hex.metadata.key,
        HexColor::pattern().unwrap_or("none")
    );

    let rgb = Text::<RgbColor>::builder("text_color")
        .label("Text Color")
        .build();
    println!(
        "  RGB: {:?} (placeholder: {:?})",
        rgb.metadata.key,
        RgbColor::placeholder()
    );

    let hsl = Text::<HslColor>::builder("bg_color")
        .label("Background Color")
        .build();
    println!(
        "  HSL: {:?} (placeholder: {:?})",
        hsl.metadata.key,
        HslColor::placeholder()
    );

    // ── Date/Time subtypes (as text) ─────────────────────────────────────────

    println!("\n📅 Date/Time Subtypes (as Text):");

    let date = Text::<IsoDate>::builder("start_date")
        .label("Start Date")
        .default_value("2026-03-06")
        .build();
    println!(
        "  ISO Date: {:?} (pattern: {})",
        date.metadata.key,
        IsoDate::pattern().unwrap_or("none")
    );

    let datetime = Text::<IsoDateTime>::builder("created_at")
        .label("Created At")
        .build();
    println!("  ISO DateTime: {:?}", datetime.metadata.key);

    let time = Text::<Time>::builder("meeting_time")
        .label("Meeting Time")
        .build();
    println!(
        "  Time: {:?} (placeholder: {:?})",
        time.metadata.key,
        Time::placeholder()
    );

    let birthday = Text::<Birthday>::builder("birth_date")
        .label("Date of Birth")
        .build();
    println!("  Birthday: {:?}", birthday.metadata.key);

    let expiry = Text::<ExpiryDate>::builder("card_expiry")
        .label("Card Expiry")
        .build();
    println!(
        "  Expiry Date: {:?} (placeholder: {:?})",
        expiry.metadata.key,
        ExpiryDate::placeholder()
    );

    // ── Number subtypes ──────────────────────────────────────────────────────

    println!("\n🔢 Number Subtypes:");

    let port = Number::<Port>::builder("server_port")
        .label("Server Port")
        .default_value(8080)
        .build();
    println!(
        "  Port: {:?} (range: {:?})",
        port.metadata.key,
        Port::default_range()
    );

    let percentage = Number::<Percentage>::builder("completion")
        .label("Completion %")
        .default_value(75.0)
        .build();
    println!(
        "  Percentage: {:?} (is_percentage: {})",
        percentage.metadata.key,
        Percentage::is_percentage()
    );

    let factor = Number::<Factor>::builder("opacity")
        .label("Opacity")
        .default_value(0.8)
        .build();
    println!(
        "  Factor: {:?} (range: {:?})",
        factor.metadata.key,
        Factor::default_range()
    );

    // ── Boolean subtypes ─────────────────────────────────────────────────────

    println!("\n✅ Boolean Subtypes:");

    let toggle = Checkbox::<Toggle>::builder("enable_feature")
        .label("Enable Feature")
        .build();
    println!(
        "  Toggle: {:?} (default: {:?})",
        toggle.metadata.key,
        Toggle::default_value()
    );

    let feature_flag = Checkbox::<FeatureFlag>::builder("beta_access").build();
    println!(
        "  Feature Flag: {:?} (label: {:?})",
        feature_flag.metadata.key,
        FeatureFlag::label()
    );

    let consent = Checkbox::<Consent>::builder("terms_accepted").build();
    println!(
        "  Consent: {:?} (help: {:?})",
        consent.metadata.key,
        Consent::help_text()
    );

    // ── Type aliases for convenience ─────────────────────────────────────────

    println!("\n🔖 Using Type Aliases:");

    let _email_param = EmailParam::builder("contact_email").build();
    let _js_param = JavaScriptParam::builder("code").build();
    let _hex_param = HexColorParam::builder("color").build();
    let _date_param = IsoDateParam::builder("date").build();
    let _port_param = PortParam::builder("port").build();
    let _checkbox_param = CheckboxParam::builder("enabled").build();

    println!("  Type aliases provide ergonomic API:");
    println!("  - EmailParam = Text<Email>");
    println!("  - JavaScriptParam = Text<JavaScript>");
    println!("  - HexColorParam = Text<HexColor>");
    println!("  - IsoDateParam = Text<IsoDate>");
    println!("  - PortParam = Number<Port>");
    println!("  - CheckboxParam = Checkbox<Toggle>");

    println!("\n✅ All subtypes use existing generic parameters (Text, Number, Checkbox)");
    println!("✅ No separate Code/Color/Date parameter types needed - just subtypes!");
}
