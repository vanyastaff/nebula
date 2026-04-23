//! One integration surface: **Telegram sendMessage** (token, chat, text, optional parse mode and
//! toggles, optional nested `reply_markup` → `inline_keyboard` with callback / url / web_app).
//!
//! There is no `resource` / `operation` indirection: configuring this action *is* sending a
//! message.
//!
//! Run: `cargo run -p nebula-schema --example telegram_send_message`

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/examples_include/telegram_send_message_shared.rs"
));

use nebula_schema::FieldValues;
use serde_json::json;

fn main() {
    let schema = build_telegram_send_message_schema();
    eprintln!("Schema: {} top-level field(s)", schema.fields().len());

    let with_keyboard = json!({
        "api_key": "1234567890:AAHevabcdefghijklmnopqrstuvwxyz12",
        "chat_id": "-1001234567890",
        "text": "Hello from Nebula — pick an action:",
        "parse_mode": "HTML",
        "append_attribution": true,
        "disable_web_page_preview": false,
        "disable_notification": false,
        "reply_markup": {
            "inline_keyboard": [
                [
                    {
                        "text": "✅ OK",
                        "action": { "mode": "callback", "value": "ok" }
                    },
                    {
                        "text": "Docs",
                        "action": { "mode": "url", "value": "https://n8n.io" }
                    }
                ],
                [
                    {
                        "text": "WebApp",
                        "action": {
                            "mode": "web_app",
                            "value": { "url": "https://example.com/app" }
                        }
                    }
                ]
            ]
        }
    });

    let values = FieldValues::from_json(with_keyboard).expect("ingest");
    schema
        .validate(&values)
        .expect("message + inline keyboard should validate");

    let minimal = json!({
        "api_key": "1234567890:AAHevabcdefghijklmnopqrstuvwxyz12",
        "chat_id": "@channelusername",
        "text": "Plain text only",
    });
    let values = FieldValues::from_json(minimal).expect("ingest");
    schema
        .validate(&values)
        .expect("minimal message without options");

    eprintln!("OK: Telegram send_message payloads validated");
}
