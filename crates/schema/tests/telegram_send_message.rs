//! See `examples_include/telegram_send_message_shared.rs`.

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/examples_include/telegram_send_message_shared.rs"
));

use nebula_schema::FieldValues;
use serde_json::json;

#[test]
fn telegram_send_message_schema_builds() {
    let s = build_telegram_send_message_schema();
    assert_eq!(s.fields().len(), 8);
}

#[test]
fn message_with_inline_keyboard_validates() {
    let schema = build_telegram_send_message_schema();
    let v = json!({
        "api_key": "1234567890:AAHevabcdefghijklmnopqrstuvwxyz12",
        "chat_id": "12345",
        "text": "Keyboard demo",
        "parse_mode": "none",
        "append_attribution": false,
        "disable_web_page_preview": true,
        "disable_notification": true,
        "reply_markup": {
            "inline_keyboard": [
                [ { "text": "Ping", "action": { "mode": "callback", "value": "p" } } ]
            ]
        }
    });
    assert!(schema.validate(&FieldValues::from_json(v).unwrap()).is_ok());
}

#[test]
fn text_only_message_validates() {
    let schema = build_telegram_send_message_schema();
    let v = json!({
        "api_key": "1234567890:AAHevabcdefghijklmnopqrstuvwxyz12",
        "chat_id": "x",
        "text": "ok",
    });
    assert!(schema.validate(&FieldValues::from_json(v).unwrap()).is_ok());
}
