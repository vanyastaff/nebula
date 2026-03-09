# Telegram Bot — Node Form Examples (Parameter Schema v2)

Real-world examples using the **v2 parameter schema** to model a Telegram Bot node.
Based on Telegram Bot API methods: `sendMessage`, `sendPhoto`, `sendDocument`,
`sendLocation`, `sendContact`, `editMessageText`.

These examples validate the schema design against a complex, widely-used API
with resource/operation patterns, conditional fields, and nested structures
(inline keyboards, reply keyboards).

---

## Table of Contents

1. [Top-Level Resource/Operation](#1-top-level-resourceoperation)
2. [sendMessage — Text with Keyboard](#2-sendmessage--text-with-keyboard)
3. [sendPhoto — Media with Caption](#3-sendphoto--media-with-caption)
4. [sendLocation — Simple Fields](#4-sendlocation--simple-fields)
5. [sendContact — Conditional Fields](#5-sendcontact--conditional-fields)
6. [Inline Keyboard — Deep Nesting](#6-inline-keyboard--deep-nesting)
7. [Reply Keyboard — Deep Nesting](#7-reply-keyboard--deep-nesting)
8. [editMessageText — Context-Dependent Fields](#8-editmessagetext--context-dependent-fields)
9. [Full Action Schema — sendMessage](#9-full-action-schema--sendmessage)
10. [Rust Builder — sendMessage](#10-rust-builder--sendmessage)

---

## 1. Top-Level Resource/Operation

The Telegram node follows the **resource → operation** pattern, like n8n.
The `resource` field determines which Telegram entity the user is working with,
and `operation` limits to valid actions for that resource.

```json
{
  "fields": [
    {
      "id": "resource",
      "type": "select",
      "label": "Resource",
      "required": true,
      "default": "message",
      "source": "static",
      "options": [
        { "value": "message", "label": "Message" },
        { "value": "chat", "label": "Chat" },
        { "value": "callback", "label": "Callback Query" }
      ]
    },
    {
      "id": "operation",
      "type": "select",
      "label": "Operation",
      "required": true,
      "default": "send_text",
      "source": "static",
      "options": [
        { "value": "send_text", "label": "Send Text" },
        { "value": "send_photo", "label": "Send Photo" },
        { "value": "send_document", "label": "Send Document" },
        { "value": "send_location", "label": "Send Location" },
        { "value": "send_contact", "label": "Send Contact" },
        { "value": "edit_text", "label": "Edit Message Text" }
      ],
      "visible_when": { "op": "eq", "field": "resource", "value": "message" }
    }
  ]
}
```

The operation select itself is conditioned on resource. When `resource` changes,
different operation options appear. This could also be modeled as `OptionSource::Dynamic`
with `depends_on: ["resource"]`.

---

## 2. sendMessage — Text with Keyboard

Full form for `sendMessage`. Demonstrates: required fields, select with static options,
boolean toggles, conditional show/hide for reply markup, and a code editor for
JSON-based custom markup.

### JSON Schema

```json
{
  "fields": [
    {
      "id": "chat_id",
      "type": "text",
      "label": "Chat ID",
      "description": "Unique identifier for the target chat or @channel_username",
      "placeholder": "123456789 or @channelname",
      "required": true,
      "expression": true
    },
    {
      "id": "text",
      "type": "text",
      "label": "Text",
      "description": "Message text, 1-4096 characters",
      "required": true,
      "multiline": true,
      "expression": true,
      "rules": [
        { "rule": "min_length", "min": 1, "message": "Text cannot be empty" },
        { "rule": "max_length", "max": 4096, "message": "Text must be at most 4096 characters" }
      ]
    },
    {
      "id": "parse_mode",
      "type": "select",
      "label": "Parse Mode",
      "description": "Formatting mode for the message text",
      "source": "static",
      "options": [
        { "value": "none", "label": "None (plain text)" },
        { "value": "Markdown", "label": "Markdown" },
        { "value": "MarkdownV2", "label": "MarkdownV2" },
        { "value": "HTML", "label": "HTML" }
      ],
      "default": "none"
    },
    {
      "id": "disable_notification",
      "type": "boolean",
      "label": "Silent Message",
      "description": "Send the message silently. Users will receive a notification with no sound.",
      "default": false
    },
    {
      "id": "protect_content",
      "type": "boolean",
      "label": "Protect Content",
      "description": "Protects the message from forwarding and saving",
      "default": false
    },
    {
      "id": "reply_markup_type",
      "type": "select",
      "label": "Reply Markup",
      "description": "Attach keyboard or other interface below the message",
      "source": "static",
      "options": [
        { "value": "none", "label": "None" },
        { "value": "inline_keyboard", "label": "Inline Keyboard" },
        { "value": "reply_keyboard", "label": "Reply Keyboard" },
        { "value": "remove_keyboard", "label": "Remove Keyboard" },
        { "value": "force_reply", "label": "Force Reply" }
      ],
      "default": "none"
    },
    {
      "id": "inline_keyboard",
      "type": "list",
      "label": "Inline Keyboard Rows",
      "description": "Each row is a list of buttons displayed under the message",
      "max_items": 20,
      "visible_when": { "op": "eq", "field": "reply_markup_type", "value": "inline_keyboard" },
      "item": {
        "id": "_row",
        "type": "list",
        "label": "Row",
        "max_items": 8,
        "item": {
          "id": "_button",
          "type": "object",
          "label": "Button",
          "fields": [
            {
              "id": "text",
              "type": "text",
              "label": "Label",
              "required": true,
              "placeholder": "Button text"
            },
            {
              "id": "action_type",
              "type": "select",
              "label": "Action",
              "source": "static",
              "options": [
                { "value": "url", "label": "Open URL" },
                { "value": "callback_data", "label": "Callback Data" },
                { "value": "switch_inline", "label": "Switch Inline Query" }
              ],
              "default": "callback_data"
            },
            {
              "id": "url",
              "type": "text",
              "label": "URL",
              "placeholder": "https://example.com",
              "format": "url",
              "visible_when": { "op": "eq", "field": "action_type", "value": "url" },
              "required_when": { "op": "eq", "field": "action_type", "value": "url" }
            },
            {
              "id": "callback_data",
              "type": "text",
              "label": "Callback Data",
              "description": "1-64 bytes sent in callback query when pressed",
              "placeholder": "action:value",
              "visible_when": { "op": "eq", "field": "action_type", "value": "callback_data" },
              "required_when": { "op": "eq", "field": "action_type", "value": "callback_data" },
              "rules": [
                { "rule": "max_length", "max": 64, "message": "Callback data must be at most 64 bytes" }
              ]
            },
            {
              "id": "switch_inline_query",
              "type": "text",
              "label": "Inline Query",
              "description": "Prompts user to select a chat and pre-fills inline query",
              "visible_when": { "op": "eq", "field": "action_type", "value": "switch_inline" }
            }
          ]
        }
      }
    },
    {
      "id": "reply_keyboard",
      "type": "list",
      "label": "Reply Keyboard Rows",
      "description": "Custom keyboard shown below the input field",
      "max_items": 20,
      "visible_when": { "op": "eq", "field": "reply_markup_type", "value": "reply_keyboard" },
      "item": {
        "id": "_row",
        "type": "list",
        "label": "Row",
        "max_items": 12,
        "item": {
          "id": "_button",
          "type": "object",
          "label": "Button",
          "fields": [
            {
              "id": "text",
              "type": "text",
              "label": "Label",
              "required": true,
              "placeholder": "Button text"
            }
          ]
        }
      }
    },
    {
      "id": "reply_keyboard_resize",
      "type": "boolean",
      "label": "Resize Keyboard",
      "description": "Fit keyboard to the number of buttons",
      "default": true,
      "visible_when": { "op": "eq", "field": "reply_markup_type", "value": "reply_keyboard" }
    },
    {
      "id": "reply_keyboard_one_time",
      "type": "boolean",
      "label": "One-Time Keyboard",
      "description": "Hide keyboard after a button is pressed",
      "default": false,
      "visible_when": { "op": "eq", "field": "reply_markup_type", "value": "reply_keyboard" }
    },
    {
      "id": "reply_to_message_id",
      "type": "number",
      "label": "Reply To Message ID",
      "description": "ID of the original message to reply to",
      "integer": true
    }
  ],
  "groups": [
    { "label": "Target", "fields": ["chat_id"] },
    { "label": "Content", "fields": ["text", "parse_mode"] },
    { "label": "Keyboard", "fields": [
      "reply_markup_type",
      "inline_keyboard",
      "reply_keyboard",
      "reply_keyboard_resize",
      "reply_keyboard_one_time"
    ] },
    { "label": "Options", "fields": [
      "disable_notification",
      "protect_content",
      "reply_to_message_id"
    ], "collapsed": true }
  ]
}
```

### Value (user fills the form)

```json
{
  "chat_id": "123456789",
  "text": "Hello! Choose an option:",
  "parse_mode": "HTML",
  "disable_notification": false,
  "protect_content": false,
  "reply_markup_type": "inline_keyboard",
  "inline_keyboard": [
    [
      { "text": "Visit Site", "action_type": "url", "url": "https://example.com" },
      { "text": "Get Info", "action_type": "callback_data", "callback_data": "info:1" }
    ],
    [
      { "text": "Cancel", "action_type": "callback_data", "callback_data": "cancel" }
    ]
  ]
}
```

### Validation Errors

```json
{
  "errors": [
    { "path": "chat_id", "code": "required", "message": "Chat ID is required" },
    { "path": "text", "code": "max_length", "message": "Text must be at most 4096 characters" },
    { "path": "inline_keyboard.0.1.callback_data", "code": "max_length", "message": "Callback data must be at most 64 bytes" },
    { "path": "inline_keyboard.1.0.text", "code": "required", "message": "Label is required" }
  ]
}
```

---

## 3. sendPhoto — Media with Caption

Demonstrates: `Mode` field for photo source selection (upload/URL/file_id),
`File` type for binary upload, optional caption with parse_mode.

### JSON Schema

```json
{
  "fields": [
    {
      "id": "chat_id",
      "type": "text",
      "label": "Chat ID",
      "required": true,
      "expression": true,
      "placeholder": "123456789 or @channelname"
    },
    {
      "id": "photo",
      "type": "mode",
      "label": "Photo",
      "required": true,
      "default_variant": "upload",
      "variants": [
        {
          "key": "upload",
          "label": "Upload File",
          "content": {
            "id": "file",
            "type": "file",
            "label": "Photo File",
            "accept": "image/*",
            "max_size": 10485760
          }
        },
        {
          "key": "url",
          "label": "URL",
          "content": {
            "id": "url",
            "type": "text",
            "label": "Photo URL",
            "placeholder": "https://example.com/photo.jpg",
            "format": "url"
          }
        },
        {
          "key": "file_id",
          "label": "File ID (from Telegram)",
          "content": {
            "id": "id",
            "type": "text",
            "label": "File ID",
            "placeholder": "AgACAgIAAxkBAAI..."
          }
        }
      ]
    },
    {
      "id": "caption",
      "type": "text",
      "label": "Caption",
      "description": "Photo caption, 0-1024 characters",
      "multiline": true,
      "expression": true,
      "rules": [
        { "rule": "max_length", "max": 1024, "message": "Caption must be at most 1024 characters" }
      ]
    },
    {
      "id": "parse_mode",
      "type": "select",
      "label": "Parse Mode",
      "source": "static",
      "options": [
        { "value": "none", "label": "None" },
        { "value": "Markdown", "label": "Markdown" },
        { "value": "MarkdownV2", "label": "MarkdownV2" },
        { "value": "HTML", "label": "HTML" }
      ],
      "default": "none",
      "visible_when": { "op": "set", "field": "caption" }
    },
    {
      "id": "show_caption_above_media",
      "type": "boolean",
      "label": "Caption Above Photo",
      "description": "Show the caption above the photo instead of below",
      "default": false,
      "visible_when": { "op": "set", "field": "caption" }
    },
    {
      "id": "disable_notification",
      "type": "boolean",
      "label": "Silent Message",
      "default": false
    }
  ],
  "groups": [
    { "label": "Target", "fields": ["chat_id"] },
    { "label": "Photo", "fields": ["photo"] },
    { "label": "Caption", "fields": ["caption", "parse_mode", "show_caption_above_media"] },
    { "label": "Options", "fields": ["disable_notification"], "collapsed": true }
  ]
}
```

### Value

```json
{
  "chat_id": "@mychannel",
  "photo": { "mode": "url", "value": "https://example.com/sunset.jpg" },
  "caption": "<b>Beautiful sunset</b> over the mountains",
  "parse_mode": "HTML",
  "show_caption_above_media": false,
  "disable_notification": false
}
```

---

## 4. sendLocation — Simple Fields

A simpler node demonstrating numeric fields with ranges and optional parameters.

### JSON Schema

```json
{
  "fields": [
    {
      "id": "chat_id",
      "type": "text",
      "label": "Chat ID",
      "required": true
    },
    {
      "id": "latitude",
      "type": "number",
      "label": "Latitude",
      "required": true,
      "min": -90,
      "max": 90,
      "step": 0.000001
    },
    {
      "id": "longitude",
      "type": "number",
      "label": "Longitude",
      "required": true,
      "min": -180,
      "max": 180,
      "step": 0.000001
    },
    {
      "id": "live_period",
      "type": "number",
      "label": "Live Period (seconds)",
      "description": "Period for which the location will be updated (60-86400 seconds, or 0x7FFFFFFF for indefinite)",
      "integer": true,
      "min": 60,
      "max": 86400
    },
    {
      "id": "heading",
      "type": "number",
      "label": "Heading",
      "description": "Direction in degrees (1-360)",
      "integer": true,
      "min": 1,
      "max": 360,
      "visible_when": { "op": "set", "field": "live_period" }
    },
    {
      "id": "proximity_alert_radius",
      "type": "number",
      "label": "Proximity Alert Radius (m)",
      "description": "Maximum distance for proximity alerts (1-100000 meters)",
      "integer": true,
      "min": 1,
      "max": 100000,
      "visible_when": { "op": "set", "field": "live_period" }
    },
    {
      "id": "disable_notification",
      "type": "boolean",
      "label": "Silent Message",
      "default": false
    }
  ],
  "groups": [
    { "label": "Target", "fields": ["chat_id"] },
    { "label": "Location", "fields": ["latitude", "longitude"] },
    { "label": "Live Location", "fields": ["live_period", "heading", "proximity_alert_radius"], "collapsed": true },
    { "label": "Options", "fields": ["disable_notification"], "collapsed": true }
  ]
}
```

### Value

```json
{
  "chat_id": "123456789",
  "latitude": 48.8584,
  "longitude": 2.2945,
  "live_period": 3600,
  "heading": 90,
  "proximity_alert_radius": 500,
  "disable_notification": false
}
```

### Validation Errors

```json
{
  "errors": [
    { "path": "latitude", "code": "required", "message": "Latitude is required" },
    { "path": "latitude", "code": "min", "message": "Minimum is -90" },
    { "path": "longitude", "code": "max", "message": "Maximum is 180" }
  ]
}
```

---

## 5. sendContact — Conditional Fields

Demonstrates optional fields that appear conditionally and the use of
`visible_when` to simplify the form.

### JSON Schema

```json
{
  "fields": [
    {
      "id": "chat_id",
      "type": "text",
      "label": "Chat ID",
      "required": true
    },
    {
      "id": "phone_number",
      "type": "text",
      "label": "Phone Number",
      "required": true,
      "placeholder": "+1234567890"
    },
    {
      "id": "first_name",
      "type": "text",
      "label": "First Name",
      "required": true
    },
    {
      "id": "last_name",
      "type": "text",
      "label": "Last Name"
    },
    {
      "id": "vcard",
      "type": "text",
      "label": "vCard",
      "description": "Additional data about the contact in vCard format, 0-2048 bytes",
      "multiline": true,
      "rules": [
        { "rule": "max_length", "max": 2048, "message": "vCard must be at most 2048 bytes" }
      ]
    },
    {
      "id": "disable_notification",
      "type": "boolean",
      "label": "Silent Message",
      "default": false
    }
  ],
  "groups": [
    { "label": "Target", "fields": ["chat_id"] },
    { "label": "Contact", "fields": ["phone_number", "first_name", "last_name"] },
    { "label": "Advanced", "fields": ["vcard", "disable_notification"], "collapsed": true }
  ]
}
```

---

## 6. Inline Keyboard — Deep Nesting

This section focuses on the **inline keyboard** structure.
In Telegram, it's `List<List<InlineKeyboardButton>>` — rows of columns.

Each button has a `text` label and exactly one action (URL, callback_data,
switch_inline_query, etc.). In our schema this is modeled as:

```
List (rows)
  └── List (buttons in row)
        └── Object (button)
              ├── text: Text (required)
              ├── action_type: Select (url | callback_data | switch_inline | login_url)
              ├── url: Text (visible when action_type == "url")
              ├── callback_data: Text (visible when action_type == "callback_data")
              ├── switch_inline_query: Text (visible when action_type == "switch_inline")
              └── login_url: Text (visible when action_type == "login_url")
```

### JSON Schema (isolated)

```json
{
  "id": "inline_keyboard",
  "type": "list",
  "label": "Inline Keyboard",
  "max_items": 20,
  "item": {
    "id": "_row",
    "type": "list",
    "label": "Row",
    "max_items": 8,
    "item": {
      "id": "_btn",
      "type": "object",
      "label": "Button",
      "fields": [
        {
          "id": "text",
          "type": "text",
          "label": "Label",
          "required": true
        },
        {
          "id": "action_type",
          "type": "select",
          "label": "Action",
          "required": true,
          "source": "static",
          "options": [
            { "value": "url", "label": "Open URL" },
            { "value": "callback_data", "label": "Callback Data" },
            { "value": "switch_inline", "label": "Switch Inline Query" },
            { "value": "login_url", "label": "Login URL" }
          ],
          "default": "callback_data"
        },
        {
          "id": "url",
          "type": "text",
          "label": "URL",
          "format": "url",
          "visible_when": { "op": "eq", "field": "action_type", "value": "url" },
          "required_when": { "op": "eq", "field": "action_type", "value": "url" }
        },
        {
          "id": "callback_data",
          "type": "text",
          "label": "Callback Data",
          "visible_when": { "op": "eq", "field": "action_type", "value": "callback_data" },
          "required_when": { "op": "eq", "field": "action_type", "value": "callback_data" },
          "rules": [
            { "rule": "max_length", "max": 64 }
          ]
        },
        {
          "id": "switch_inline_query",
          "type": "text",
          "label": "Inline Query",
          "visible_when": { "op": "eq", "field": "action_type", "value": "switch_inline" }
        },
        {
          "id": "login_url_value",
          "type": "text",
          "label": "Login URL",
          "format": "url",
          "description": "HTTPS URL for automatic user authorization",
          "visible_when": { "op": "eq", "field": "action_type", "value": "login_url" },
          "required_when": { "op": "eq", "field": "action_type", "value": "login_url" }
        }
      ]
    }
  }
}
```

### Value

```json
{
  "inline_keyboard": [
    [
      { "text": "Open Website", "action_type": "url", "url": "https://example.com" },
      { "text": "Details", "action_type": "callback_data", "callback_data": "details:42" }
    ],
    [
      { "text": "Share", "action_type": "switch_inline", "switch_inline_query": "share 42" }
    ],
    [
      { "text": "Login", "action_type": "login_url", "login_url_value": "https://auth.example.com/tg" }
    ]
  ]
}
```

### Error Paths

```json
[
  { "path": "inline_keyboard.0.0.text", "code": "required", "message": "Label is required" },
  { "path": "inline_keyboard.0.1.callback_data", "code": "max_length", "message": "Must be at most 64 bytes" },
  { "path": "inline_keyboard.2.0.login_url_value", "code": "required", "message": "Login URL is required" }
]
```

---

## 7. Reply Keyboard — Deep Nesting

Reply keyboard is simpler — each button is just a text label.
The keyboard options (resize, one_time, selective) are top-level booleans.

```
List (rows)
  └── List (buttons in row)
        └── Object (button)
              └── text: Text (required)
```

### JSON Schema (isolated)

```json
{
  "id": "reply_keyboard",
  "type": "list",
  "label": "Reply Keyboard",
  "max_items": 20,
  "item": {
    "id": "_row",
    "type": "list",
    "label": "Row",
    "max_items": 12,
    "item": {
      "id": "_btn",
      "type": "object",
      "label": "Button",
      "fields": [
        {
          "id": "text",
          "type": "text",
          "label": "Label",
          "required": true,
          "placeholder": "Button text"
        }
      ]
    }
  }
}
```

### Value

```json
{
  "reply_keyboard": [
    [
      { "text": "Yes" },
      { "text": "No" }
    ],
    [
      { "text": "Maybe" },
      { "text": "Cancel" }
    ]
  ]
}
```

### Error Paths

```json
[
  { "path": "reply_keyboard.0.0.text", "code": "required", "message": "Label is required" },
  { "path": "reply_keyboard", "code": "max_items", "message": "At most 20 rows allowed" }
]
```

---

## 8. editMessageText — Context-Dependent Fields

Demonstrates mutually exclusive required fields: either `chat_id` + `message_id`
or `inline_message_id`. Uses conditions to show/hide and require the right set.

### JSON Schema

```json
{
  "fields": [
    {
      "id": "target_type",
      "type": "select",
      "label": "Target",
      "source": "static",
      "required": true,
      "options": [
        { "value": "chat_message", "label": "Chat Message (chat_id + message_id)" },
        { "value": "inline_message", "label": "Inline Message (inline_message_id)" }
      ],
      "default": "chat_message"
    },
    {
      "id": "chat_id",
      "type": "text",
      "label": "Chat ID",
      "visible_when": { "op": "eq", "field": "target_type", "value": "chat_message" },
      "required_when": { "op": "eq", "field": "target_type", "value": "chat_message" }
    },
    {
      "id": "message_id",
      "type": "number",
      "label": "Message ID",
      "integer": true,
      "visible_when": { "op": "eq", "field": "target_type", "value": "chat_message" },
      "required_when": { "op": "eq", "field": "target_type", "value": "chat_message" }
    },
    {
      "id": "inline_message_id",
      "type": "text",
      "label": "Inline Message ID",
      "visible_when": { "op": "eq", "field": "target_type", "value": "inline_message" },
      "required_when": { "op": "eq", "field": "target_type", "value": "inline_message" }
    },
    {
      "id": "text",
      "type": "text",
      "label": "New Text",
      "required": true,
      "multiline": true,
      "rules": [
        { "rule": "min_length", "min": 1 },
        { "rule": "max_length", "max": 4096 }
      ]
    },
    {
      "id": "parse_mode",
      "type": "select",
      "label": "Parse Mode",
      "source": "static",
      "options": [
        { "value": "none", "label": "None" },
        { "value": "Markdown", "label": "Markdown" },
        { "value": "MarkdownV2", "label": "MarkdownV2" },
        { "value": "HTML", "label": "HTML" }
      ],
      "default": "none"
    },
    {
      "id": "disable_web_page_preview",
      "type": "boolean",
      "label": "Disable Link Preview",
      "default": false
    }
  ],
  "groups": [
    { "label": "Target", "fields": ["target_type", "chat_id", "message_id", "inline_message_id"] },
    { "label": "Content", "fields": ["text", "parse_mode", "disable_web_page_preview"] }
  ]
}
```

### Value (chat message target)

```json
{
  "target_type": "chat_message",
  "chat_id": "123456789",
  "message_id": 42,
  "text": "Updated message content",
  "parse_mode": "HTML",
  "disable_web_page_preview": false
}
```

### Value (inline message target)

```json
{
  "target_type": "inline_message",
  "inline_message_id": "CAADBBRnx...",
  "text": "Updated inline text",
  "parse_mode": "none",
  "disable_web_page_preview": true
}
```

---

## 9. Full Action Schema — sendMessage

A complete `TelegramSendMessage` action schema combining multiple v2 features.
In nebula each action is **atomic** (one struct = one operation), so there is no
resource/operation grouping. Instead, **Mode** is used at the field level —
here it switches the reply markup between none, inline keyboard, and reply keyboard.

### JSON Schema

```json
{
  "fields": [
    {
      "id": "chat_id",
      "type": "text",
      "label": "Chat ID",
      "required": true,
      "expression": true,
      "placeholder": "123456789 or @channelname"
    },
    {
      "id": "text",
      "type": "text",
      "label": "Text",
      "required": true,
      "multiline": true,
      "expression": true,
      "rules": [
        { "rule": "min_length", "min": 1 },
        { "rule": "max_length", "max": 4096 }
      ]
    },
    {
      "id": "parse_mode",
      "type": "select",
      "label": "Parse Mode",
      "source": "static",
      "options": [
        { "value": "none", "label": "None" },
        { "value": "MarkdownV2", "label": "MarkdownV2" },
        { "value": "HTML", "label": "HTML" }
      ],
      "default": "none"
    },
    {
      "id": "reply_markup",
      "type": "mode",
      "label": "Reply Markup",
      "default_variant": "none",
      "variants": [
        {
          "key": "none",
          "label": "None",
          "content": {
            "id": "_",
            "type": "hidden"
          }
        },
        {
          "key": "inline_keyboard",
          "label": "Inline Keyboard",
          "content": {
            "id": "rows",
            "type": "list",
            "label": "Rows",
            "max_items": 20,
            "item": {
              "id": "_row",
              "type": "list",
              "label": "Row",
              "max_items": 8,
              "item": {
                "id": "_btn",
                "type": "object",
                "label": "Button",
                "fields": [
                  {
                    "id": "text",
                    "type": "text",
                    "label": "Label",
                    "required": true
                  },
                  {
                    "id": "action_type",
                    "type": "select",
                    "label": "Action",
                    "source": "static",
                    "options": [
                      { "value": "url", "label": "Open URL" },
                      { "value": "callback_data", "label": "Callback Data" }
                    ],
                    "default": "callback_data"
                  },
                  {
                    "id": "url",
                    "type": "text",
                    "label": "URL",
                    "format": "url",
                    "visible_when": { "op": "eq", "field": "action_type", "value": "url" },
                    "required_when": { "op": "eq", "field": "action_type", "value": "url" }
                  },
                  {
                    "id": "callback_data",
                    "type": "text",
                    "label": "Callback Data",
                    "visible_when": { "op": "eq", "field": "action_type", "value": "callback_data" },
                    "required_when": { "op": "eq", "field": "action_type", "value": "callback_data" },
                    "rules": [
                      { "rule": "max_length", "max": 64 }
                    ]
                  }
                ]
              }
            }
          }
        },
        {
          "key": "reply_keyboard",
          "label": "Reply Keyboard",
          "content": {
            "id": "config",
            "type": "object",
            "label": "Keyboard Config",
            "fields": [
              {
                "id": "rows",
                "type": "list",
                "label": "Rows",
                "max_items": 20,
                "item": {
                  "id": "_row",
                  "type": "list",
                  "label": "Row",
                  "max_items": 12,
                  "item": {
                    "id": "_button",
                    "type": "object",
                    "label": "Button",
                    "fields": [
                      {
                        "id": "text",
                        "type": "text",
                        "label": "Label",
                        "required": true
                      }
                    ]
                  }
                }
              },
              {
                "id": "resize",
                "type": "boolean",
                "label": "Resize Keyboard",
                "default": true
              },
              {
                "id": "one_time",
                "type": "boolean",
                "label": "One-Time Keyboard",
                "default": false
              }
            ]
          }
        }
      ]
    },
    {
      "id": "disable_notification",
      "type": "boolean",
      "label": "Silent Message",
      "default": false
    },
    {
      "id": "protect_content",
      "type": "boolean",
      "label": "Protect Content",
      "default": false
    },
    {
      "id": "reply_to_message_id",
      "type": "number",
      "label": "Reply To Message ID",
      "integer": true
    }
  ],
  "groups": [
    { "label": "Target", "fields": ["chat_id"] },
    { "label": "Content", "fields": ["text", "parse_mode"] },
    { "label": "Keyboard", "fields": ["reply_markup"] },
    { "label": "Options", "fields": ["disable_notification", "protect_content", "reply_to_message_id"], "collapsed": true }
  ]
}
```

### Frontend rendering for Mode

```
// When user selects a mode variant:
const variant = field.variants.find(v => v.key === selected_mode);
render widget for variant.content
```

### Value (no keyboard)

```json
{
  "chat_id": "123456789",
  "text": "Hello from Nebula!",
  "parse_mode": "HTML",
  "reply_markup": { "mode": "none", "value": null },
  "disable_notification": false
}
```

### Value (inline keyboard)

```json
{
  "chat_id": "123456789",
  "text": "Choose an option:",
  "parse_mode": "HTML",
  "reply_markup": {
    "mode": "inline_keyboard",
    "value": [
      [
        { "text": "Visit Site", "action_type": "url", "url": "https://example.com" },
        { "text": "Get Info", "action_type": "callback_data", "callback_data": "info:1" }
      ],
      [
        { "text": "Cancel", "action_type": "callback_data", "callback_data": "cancel" }
      ]
    ]
  },
  "disable_notification": false
}
```

### Value (reply keyboard)

```json
{
  "chat_id": "123456789",
  "text": "Pick one:",
  "parse_mode": "none",
  "reply_markup": {
    "mode": "reply_keyboard",
    "value": {
      "rows": [
        [{ "text": "Option A" }, { "text": "Option B" }],
        [{ "text": "Option C" }]
      ],
      "resize": true,
      "one_time": false
    }
  }
}
```

### Error paths with Mode

| Path | Meaning |
|---|---|
| `reply_markup.value.0.1.text` | Inline keyboard, row 0, button 1, label missing |
| `reply_markup.value.0.0.callback_data` | Inline keyboard, row 0, button 0, callback data too long |
| `reply_markup.value.rows.0.0.text` | Reply keyboard, row 0, button 0, label missing |

---

## 10. Rust Builder — sendMessage

How the `TelegramSendMessage` action schema looks using the Rust builder API.
Mode is used at the field level for reply markup switching.

```rust
let send_message = Schema::builder()
    // ── Target ──────────────────────────────────────────
    .field(
        Field::text("chat_id")
            .label("Chat ID")
            .required()
            .expression()
            .placeholder("123456789 or @channelname")
    )
    // ── Content ─────────────────────────────────────────
    .field(
        Field::text("text")
            .label("Text")
            .required()
            .multiline()
            .expression()
            .rule(Rule::min_length(1, None))
            .rule(Rule::max_length(4096, None))
    )
    .field(
        Field::select("parse_mode")
            .label("Parse Mode")
            .default("none")
            .option("none", "None")
            .option("MarkdownV2", "MarkdownV2")
            .option("HTML", "HTML")
    )
    // ── Reply Markup (Mode) ─────────────────────────────
    .field(
        Field::mode("reply_markup")
            .label("Reply Markup")
            .default_variant("none")
            // No keyboard
            .variant("none", "None",
                Field::hidden("_").build()
            )
            // Inline keyboard → List<List<Object>>
            .variant("inline_keyboard", "Inline Keyboard",
                Field::list("rows",
                    Field::list("_row",
                        Field::object("_btn")
                            .label("Button")
                            .fields(vec![
                                Field::text("text")
                                    .label("Label")
                                    .required()
                                    .build(),
                                Field::select("action_type")
                                    .label("Action")
                                    .default("callback_data")
                                    .option("url", "Open URL")
                                    .option("callback_data", "Callback Data")
                                    .build(),
                                Field::text("url")
                                    .label("URL")
                                    .format("url")
                                    .visible_when(Condition::eq("action_type", "url"))
                                    .required_when(Condition::eq("action_type", "url"))
                                    .build(),
                                Field::text("callback_data")
                                    .label("Callback Data")
                                    .visible_when(Condition::eq(
                                        "action_type", "callback_data",
                                    ))
                                    .required_when(Condition::eq(
                                        "action_type", "callback_data",
                                    ))
                                    .rule(Rule::max_length(64, None))
                                    .build(),
                            ])
                            .build()
                    )
                    .label("Row")
                    .max_items(8)
                    .build()
                )
                .label("Rows")
                .max_items(20)
                .build()
            )
            // Reply keyboard → Object { rows, resize, one_time }
            .variant("reply_keyboard", "Reply Keyboard",
                Field::object("config")
                    .label("Keyboard Config")
                    .fields(vec![
                        Field::list("rows",
                            Field::list("_row",
                                Field::object("_button")
                                    .label("Button")
                                    .fields(vec![
                                        Field::text("text")
                                            .label("Label")
                                            .required()
                                            .build(),
                                    ])
                                    .build()
                            )
                            .label("Row")
                            .max_items(12)
                            .build()
                        )
                        .label("Rows")
                        .max_items(20)
                        .build(),
                        Field::boolean("resize")
                            .label("Resize Keyboard")
                            .default(true)
                            .build(),
                        Field::boolean("one_time")
                            .label("One-Time Keyboard")
                            .default(false)
                            .build(),
                    ])
                    .build()
            )
    )
    // ── Options ─────────────────────────────────────────
    .field(
        Field::boolean("disable_notification")
            .label("Silent Message")
            .default(false)
    )
    .field(
        Field::boolean("protect_content")
            .label("Protect Content")
            .default(false)
    )
    .field(
        Field::number("reply_to_message_id")
            .label("Reply To Message ID")
            .integer()
    )
    // ── Layout ──────────────────────────────────────────
    .group("Target", &["chat_id"])
    .group("Content", &["text", "parse_mode"])
    .group("Keyboard", &["reply_markup"])
    .group_collapsed("Options", &[
        "disable_notification",
        "protect_content",
        "reply_to_message_id",
    ])
    .build()?;
```

---

## Design Observations

1. **Mode as field-level input switcher**: `Mode` replaces the
   select + visible_when pattern for mutually exclusive field groups.
   `sendPhoto.photo` switches between upload/URL/file_id; `sendMessage.reply_markup`
   switches between none/inline keyboard/reply keyboard. Each variant has one
   `content` node, and the value shape is `{ "mode": "key", "value": ... }`.

2. **Inline keyboard = List<List<Object>>**: Two levels of nesting
   (rows → buttons) works naturally. Condition paths like
   `action_type` (relative path inside the `_btn` Object) let buttons conditionally
   show/hide fields based on the user's selected action.

3. **Atomic actions**: Each Telegram operation (`sendMessage`, `sendPhoto`,
   `sendLocation`) is its own action struct with its own parameter schema.
   Shared fields like `chat_id` can be extracted into reusable field snippets
   or presets at the builder layer.

4. **Mutually exclusive fields**: The `editMessageText` example shows
   how `visible_when` + `required_when` with a discriminating select
   cleanly handles "either A or B is required" patterns.

5. **Error paths in nested lists**: Paths like `inline_keyboard.0.1.text`
   precisely identify which button in which row has the error,
   matching the frontend's DOM structure. For Mode fields the path goes
   through `.value`: `reply_markup.value.0.1.text`.

6. **File type for uploads**: `sendPhoto` and `sendDocument` use the `File` field
   type with `accept` and `max_size`, replacing the previous text-with-format workaround.

7. **Expression flag**: Fields like `chat_id` and `text` set `expression: true`,
   enabling `{{ $json.chatId }}` template interpolation in the frontend editor.

8. **Relative condition paths**: Inside an Object, conditions use bare field names
   (`action_type`) instead of full paths (`inline_keyboard._row._btn.action_type`),
   keeping schemas readable and scope-safe.
   The schema is general enough for any API.

