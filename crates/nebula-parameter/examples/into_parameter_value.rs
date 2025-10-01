//! Example demonstrating the flexible `Into<ParameterValue>` API
//!
//! The `set_parameter_value` method now accepts any type that implements `Into<ParameterValue>`,
//! making it more convenient to use.

use nebula_parameter::prelude::*;
use nebula_parameter::types::{TextParameter, CheckboxParameter, NumberParameter};
use nebula_value::Value;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Into<ParameterValue> API Examples ===\n");

    // Example 1: TextParameter with different input types
    {
        println!("1. TextParameter - multiple input types:");
        let mut text_param = TextParameter::builder("username", "Username")
            .description("Enter your username")
            .build();

        // Can pass &str directly (converts to ParameterValue::Expression)
        text_param.set_parameter_value("alice")?;
        println!("   Set from &str: {:?}", text_param.get_value());

        // Can pass String
        text_param.set_parameter_value("bob".to_string())?;
        println!("   Set from String: {:?}", text_param.get_value());

        // Can pass nebula_value::Value directly
        text_param.set_parameter_value(Value::text("charlie"))?;
        println!("   Set from Value: {:?}", text_param.get_value());

        // Can pass ParameterValue explicitly
        text_param.set_parameter_value(ParameterValue::Value(Value::text("dave")))?;
        println!("   Set from ParameterValue: {:?}", text_param.get_value());
    }

    println!();

    // Example 2: CheckboxParameter with bool
    {
        println!("2. CheckboxParameter - bool input:");
        let mut checkbox = CheckboxParameter::builder("enabled", "Enabled")
            .description("Enable this feature")
            .build();

        // Can pass bool directly (converts to ParameterValue)
        checkbox.set_parameter_value(true)?;
        println!("   Set from bool: {:?}", checkbox.get_value());

        // Can pass nebula_value::Value::Boolean
        checkbox.set_parameter_value(Value::boolean(false))?;
        println!("   Set from Value::Boolean: {:?}", checkbox.get_value());
    }

    println!();

    // Example 3: NumberParameter with numeric types
    {
        println!("3. NumberParameter - numeric inputs:");
        let mut number_param = NumberParameter::builder("count", "Count")
            .description("Enter a count")
            .build();

        // Can pass i64 directly
        number_param.set_parameter_value(42i64)?;
        println!("   Set from i64: {:?}", number_param.get_value());

        // Can pass i32 (converts to i64)
        number_param.set_parameter_value(100i32)?;
        println!("   Set from i32: {:?}", number_param.get_value());

        // Can pass f64
        number_param.set_parameter_value(3.14f64)?;
        println!("   Set from f64: {:?}", number_param.get_value());

        // Can pass nebula_value::Integer
        let integer = nebula_value::Integer::new(999);
        number_param.set_parameter_value(integer)?;
        println!("   Set from nebula_value::Integer: {:?}", number_param.get_value());
    }

    println!();

    // Example 4: Chaining with builder pattern
    {
        println!("4. Builder pattern with direct values:");

        let mut text = TextParameter::builder("email", "Email")
            .description("Your email address")
            .default_value("user@example.com")
            .build();

        // Fluent API - set value directly with simple types
        text.set_parameter_value("alice@example.com")?;

        if let Some(value) = text.get_value() {
            println!("   Email set to: {}", value);
        }
    }

    println!();

    // Example 5: Benefits of Into<> - less boilerplate
    {
        println!("5. Less boilerplate - before vs after:");

        let mut param = TextParameter::builder("demo", "Demo").build();

        // OLD WAY (still works):
        // param.set_parameter_value(ParameterValue::Value(Value::text("old way")))?;

        // NEW WAY (simpler):
        param.set_parameter_value("new way")?;  // Just pass &str!

        println!("   ✓ Much cleaner API!");
    }

    println!("\n=== All examples completed successfully! ===");
    Ok(())
}
