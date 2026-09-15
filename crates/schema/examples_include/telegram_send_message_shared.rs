// Telegram `sendMessage`-shaped config (single integration surface): token, target chat, text,
// optional mode/toggles, optional `reply_markup.inline_keyboard` (rows × buttons with mode actions).
// No resource/operation routing — the action is the integration itself.
//
// Included from `examples/telegram_send_message.rs` and `tests/telegram_send_message.rs`.

use nebula_schema::{Property, Schema, ValidSchema, field_key};

/// One inline button: label + `action` (callback / url / web_app) in Nebula mode wire.
fn inline_button_field() -> Property {
    Property::object(field_key!("button"))
        .label("Inline button")
        .property(
            Property::string(field_key!("text"))
                .required()
                .max_length(256)
                .description("Button label (Telegram `text`)"),
        )
        .property(
            Property::mode(field_key!("action"))
                .label("Button action")
                .variant(
                    "callback",
                    "Callback",
                    Property::string(field_key!("callback_data"))
                        .required()
                        .max_length(64)
                        .description("`callback_data` (1–64 bytes in API)"),
                )
                .variant(
                    "url",
                    "Open URL",
                    Property::string(field_key!("url"))
                        .required()
                        .url()
                        .description("HTTPS link opened when the user taps the button"),
                )
                .variant(
                    "web_app",
                    "Web App",
                    Property::object(field_key!("web_app"))
                        .description(
                            "Same shape as Telegram `web_app: { url }` on InlineKeyboardButton",
                        )
                        .property(
                            Property::string(field_key!("url"))
                                .required()
                                .url()
                                .label("Web App URL"),
                        ),
                )
                .default_variant("callback"),
        )
        .into()
}

fn inline_keyboard_list() -> Property {
    let row = Property::list(field_key!("row"))
        .label("Row of buttons")
        .min_items(1)
        .max_items(8)
        .item(inline_button_field());

    Property::list(field_key!("inline_keyboard"))
        .label("Inline keyboard (rows × buttons)")
        .description("Maps to `reply_markup.inline_keyboard` in sendMessage")
        .min_items(1)
        .max_items(20)
        .item(row)
        .into()
}

/// Schema for a Telegram “send message + optional inline keyboard” action only.
pub fn build_telegram_send_message_schema() -> ValidSchema {
    Schema::builder()
        .property(
            Property::secret(field_key!("api_key"))
                .label("Bot API token")
                .description("From BotFather; never log or echo")
                .required()
                .min_length(20)
                .reveal_last(4),
        )
        .property(
            Property::string(field_key!("chat_id"))
                .label("Chat ID")
                .description("Target chat, thread id, or @username")
                .required(),
        )
        .property(
            Property::string(field_key!("text"))
                .label("Text")
                .description("Message body (max 4096, Telegram API)")
                .min_length(1)
                .max_length(4096)
                .required(),
        )
        .property(
            Property::select(field_key!("parse_mode"))
                .label("Parse mode")
                .option("none", "None (plain text)")
                .option("HTML", "HTML")
                .option("MarkdownV2", "MarkdownV2")
                .option("Markdown", "Markdown (legacy)"),
        )
        .property(
            Property::boolean(field_key!("append_attribution"))
                .label("Append “sent via automation” line")
                .description("Product choice: append a short footer to `text`"),
        )
        .property(Property::boolean(field_key!("disable_web_page_preview")).label("Disable link previews"))
        .property(
            Property::boolean(field_key!("disable_notification"))
                .label("Send silently")
                .description("If true, the message is sent without sound"),
        )
        .property(
            Property::object(field_key!("reply_markup"))
                .label("Reply markup (inline keyboard)")
                .description("Optional. Omit entirely if you do not need a keyboard")
                .property(inline_keyboard_list()),
        )
        .build()
        .expect("telegram send_message example schema lints")
}
