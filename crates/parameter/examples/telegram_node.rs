//! Telegram Bot node — parameter schema example.
//!
//! Demonstrates the **resource → operation** pattern, conditional fields,
//! required_when logic, and mode-based keyboard structures using the v3 API.
//!
//! Based on Telegram Bot API methods: `sendMessage`, `sendPhoto`,
//! `sendDocument`, `sendLocation`, `sendContact`.
//!
//! Run with: `cargo run --example telegram_node -p nebula-parameter`

use nebula_parameter::{Condition, Parameter, ParameterCollection};
use serde_json::json;

fn resource_field() -> Parameter {
    Parameter::select("resource")
        .label("Resource")
        .required()
        .default(json!("message"))
        .option(json!("message"), "Message")
        .option(json!("chat"), "Chat")
        .option(json!("callback"), "Callback Query")
}

fn operation_field() -> Parameter {
    Parameter::select("operation")
        .label("Operation")
        .required()
        .default(json!("sendMessage"))
        .searchable()
        .option(json!("sendMessage"), "Send Message")
        .option(json!("sendPhoto"), "Send Photo")
        .option(json!("sendDocument"), "Send Document")
        .option(json!("sendLocation"), "Send Location")
        .option(json!("sendContact"), "Send Contact")
        .option(json!("editMessageText"), "Edit Message Text")
}

/// Schema for the `sendMessage` operation.
fn send_message_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(resource_field())
        .add(operation_field())
        .add(
            Parameter::string("chat_id")
                .label("Chat ID")
                .description("Unique identifier for the target chat or @username")
                .required(),
        )
        .add(
            Parameter::string("text")
                .label("Text")
                .description("Text of the message (1–4096 characters)")
                .required(),
        )
        .add(
            Parameter::select("parse_mode")
                .label("Parse Mode")
                .description("Mode for parsing entities in the message text")
                .default(json!("HTML"))
                .option(json!("HTML"), "HTML")
                .option(json!("Markdown"), "Markdown")
                .option(json!("MarkdownV2"), "MarkdownV2"),
        )
        .add(
            Parameter::boolean("disable_notification")
                .label("Disable Notification")
                .description("Sends the message silently"),
        )
        .add(
            // reply_to_message_id — only shown when user wants to reply
            Parameter::integer("reply_to_message_id")
                .label("Reply To Message ID")
                .description("If set, the message is sent as a reply")
                .visible_when(Condition::set("reply_to_message_id")),
        )
}

/// Schema for `sendContact` — demonstrates `required_when`.
fn send_contact_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(resource_field())
        .add(operation_field())
        .add(Parameter::string("chat_id").label("Chat ID").required())
        .add(
            Parameter::string("phone_number")
                .label("Phone Number")
                .required(),
        )
        .add(
            Parameter::string("first_name")
                .label("First Name")
                .required(),
        )
        .add(Parameter::string("last_name").label("Last Name"))
        .add(
            // vcard is only required when last_name is filled in (demo of required_when)
            Parameter::string("vcard")
                .label("vCard")
                .description("Additional data about the contact in vCard format")
                .required_when(Condition::set("last_name")),
        )
}

/// Mode-based keyboard schema — discriminated union between inline and reply.
fn keyboard_mode_field() -> Parameter {
    Parameter::mode("keyboard")
        .label("Reply Markup")
        .variant(
            Parameter::boolean("_placeholder")
                .label("")
                .description("No keyboard"),
        )
        .variant(
            Parameter::string("inline_keyboard_json")
                .label("Inline Keyboard JSON")
                .description("JSON array of button rows"),
        )
        .variant(
            Parameter::string("reply_keyboard_json")
                .label("Reply Keyboard JSON")
                .description("JSON array of keyboard rows"),
        )
        .default_variant("_placeholder")
}

fn main() {
    let schema = send_message_schema();
    println!("sendMessage schema ({} fields):", schema.len());
    for param in &schema.parameters {
        println!(
            "  - {} ({})",
            param.id,
            param.label.as_deref().unwrap_or("")
        );
    }

    println!();
    let contact_schema = send_contact_schema();
    println!("sendContact schema ({} fields):", contact_schema.len());
    for param in &contact_schema.parameters {
        let req = if param.required { " [required]" } else { "" };
        let cond_req = if param.required_when.is_some() {
            " [required_when]"
        } else {
            ""
        };
        println!("  - {}{req}{cond_req}", param.id);
    }

    println!();
    let keyboard = keyboard_mode_field();
    let json = serde_json::to_string_pretty(&keyboard).expect("serializes");
    println!("keyboard mode field JSON:\n{json}");
}
