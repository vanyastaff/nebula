//! Telegram Bot node — parameter schema example.
//!
//! Demonstrates the **resource → operation** pattern, conditional fields,
//! required_when logic, and nested keyboard structures using the v2 schema API.
//!
//! Based on Telegram Bot API methods: `sendMessage`, `sendPhoto`,
//! `sendDocument`, `sendLocation`, `sendContact`.
//!
//! Run with: `cargo run --example telegram_node -p nebula-parameter`

use nebula_parameter::{
    Condition, Field, FieldMetadata, ModeVariant, OptionSource, Schema, SelectOption,
};
use serde_json::json;

fn resource_field() -> Field {
    Field::Select {
        meta: FieldMetadata {
            id: "resource".to_owned(),
            label: "Resource".to_owned(),
            required: true,
            default: Some(json!("message")),
            ..FieldMetadata::default()
        },
        source: OptionSource::Static {
            options: vec![
                SelectOption::new(json!("message"), "Message"),
                SelectOption::new(json!("chat"), "Chat"),
                SelectOption::new(json!("callback"), "Callback Query"),
            ],
        },
        multiple: false,
        allow_custom: false,
        searchable: false,
    }
}

fn operation_field() -> Field {
    Field::Select {
        meta: FieldMetadata {
            id: "operation".to_owned(),
            label: "Operation".to_owned(),
            required: true,
            default: Some(json!("sendMessage")),
            ..FieldMetadata::default()
        },
        source: OptionSource::Static {
            options: vec![
                SelectOption::new(json!("sendMessage"), "Send Message"),
                SelectOption::new(json!("sendPhoto"), "Send Photo"),
                SelectOption::new(json!("sendDocument"), "Send Document"),
                SelectOption::new(json!("sendLocation"), "Send Location"),
                SelectOption::new(json!("sendContact"), "Send Contact"),
                SelectOption::new(json!("editMessageText"), "Edit Message Text"),
            ],
        },
        multiple: false,
        allow_custom: false,
        searchable: true,
    }
}

/// Schema for the `sendMessage` operation.
fn send_message_schema() -> Schema {
    Schema::new()
        .field(resource_field())
        .field(operation_field())
        .field(
            Field::text("chat_id")
                .with_label("Chat ID")
                .with_description("Unique identifier for the target chat or @username")
                .required(),
        )
        .field(
            Field::text("text")
                .with_label("Text")
                .with_description("Text of the message (1–4096 characters)")
                .required(),
        )
        .field(
            Field::Select {
                meta: {
                    let mut m = FieldMetadata::new("parse_mode");
                    m.set_label("Parse Mode");
                    m.set_description("Mode for parsing entities in the message text");
                    m.default = Some(json!("HTML"));
                    m
                },
                source: OptionSource::Static {
                    options: vec![
                        SelectOption::new(json!("HTML"), "HTML"),
                        SelectOption::new(json!("Markdown"), "Markdown"),
                        SelectOption::new(json!("MarkdownV2"), "MarkdownV2"),
                    ],
                },
                multiple: false,
                allow_custom: false,
                searchable: false,
            },
        )
        .field(
            Field::boolean("disable_notification")
                .with_label("Disable Notification")
                .with_description("Sends the message silently"),
        )
        .field(
            // reply_to_message_id — only shown when user wants to reply
            Field::integer("reply_to_message_id")
                .with_label("Reply To Message ID")
                .with_description("If set, the message is sent as a reply")
                .visible_when(Condition::Set {
                    field: "reply_to_message_id".to_owned(),
                }),
        )
}

/// Schema for `sendContact` — demonstrates `required_when`.
fn send_contact_schema() -> Schema {
    Schema::new()
        .field(resource_field())
        .field(operation_field())
        .field(
            Field::text("chat_id")
                .with_label("Chat ID")
                .required(),
        )
        .field(
            Field::text("phone_number")
                .with_label("Phone Number")
                .required(),
        )
        .field(
            Field::text("first_name")
                .with_label("First Name")
                .required(),
        )
        .field(Field::text("last_name").with_label("Last Name"))
        .field(
            // vcard is only required when last_name is filled in (demo of required_when)
            Field::text("vcard")
                .with_label("vCard")
                .with_description("Additional data about the contact in vCard format")
                .required_when(Condition::Set {
                    field: "last_name".to_owned(),
                }),
        )
}

/// Mode-based keyboard schema — discriminated union between inline and reply.
fn keyboard_mode_field() -> Field {
    Field::Mode {
        meta: {
            let mut m = FieldMetadata::new("keyboard");
            m.set_label("Reply Markup");
            m
        },
        variants: vec![
            ModeVariant {
                key: "none".to_owned(),
                label: "None".to_owned(),
                description: Some("No keyboard".to_owned()),
                content: Box::new(Field::boolean("_placeholder").with_label("")),
            },
            ModeVariant {
                key: "inline".to_owned(),
                label: "Inline Keyboard".to_owned(),
                description: Some("Buttons appear inline below the message".to_owned()),
                content: Box::new(
                    Field::text("inline_keyboard_json")
                        .with_label("Inline Keyboard JSON")
                        .with_description("JSON array of button rows"),
                ),
            },
            ModeVariant {
                key: "reply".to_owned(),
                label: "Reply Keyboard".to_owned(),
                description: Some("Custom reply keyboard replaces the standard keyboard".to_owned()),
                content: Box::new(
                    Field::text("reply_keyboard_json")
                        .with_label("Reply Keyboard JSON")
                        .with_description("JSON array of keyboard rows"),
                ),
            },
        ],
        default_variant: Some("none".to_owned()),
    }
}

fn main() {
    let schema = send_message_schema();
    println!(
        "sendMessage schema ({} fields):",
        schema.fields.len()
    );
    for field in &schema.fields {
        println!("  - {} ({})", field.meta().id, field.meta().label);
    }

    println!();
    let contact_schema = send_contact_schema();
    println!(
        "sendContact schema ({} fields):",
        contact_schema.fields.len()
    );
    for field in &contact_schema.fields {
        let meta = field.meta();
        let req = if meta.required { " [required]" } else { "" };
        let cond_req = if meta.required_when.is_some() {
            " [required_when]"
        } else {
            ""
        };
        println!("  - {}{req}{cond_req}", meta.id);
    }

    println!();
    let keyboard = keyboard_mode_field();
    let json = serde_json::to_string_pretty(&keyboard).expect("serializes");
    println!("keyboard mode field JSON:\n{json}");
}
