# DataTag Registry — Complete Reference

## Core Tags (nebula-core, always available)

### Primitives

| Tag | Name | Color | Compatible With | Description |
|-----|------|-------|----------------|-------------|
| `json` | JSON | `#9E9E9E` (gray) | — | Arbitrary JSON value. Universal acceptor — all tags compatible with json. |
| `text` | Text | `#2196F3` (blue) | `json` | String value. |
| `number` | Number | `#4CAF50` (green) | `json`, `text` | Numeric value (integer or float). Coerces to text. |
| `boolean` | Boolean | `#FF9800` (orange) | `json`, `number`, `text` | True/false. Coerces to number (0/1) and text. |
| `array` | Array | `#00BCD4` (cyan) | `json` | JSON array of any elements. |
| `object` | Object | `#795548` (brown) | `json` | JSON object (key-value map). |

### Binary

| Tag | Name | Color | Compatible With | Description |
|-----|------|-------|----------------|-------------|
| `binary` | Binary | `#607D8B` (blue-gray) | `json` | Raw binary data (BinaryData). Base type for all file/media tags. |
| `file` | File | `#78909C` (gray-blue) | `binary`, `json` | Generic file with name and content_type. |

### Streaming

| Tag | Name | Color | Compatible With | Description |
|-----|------|-------|----------------|-------------|
| `stream` | Stream | `#E91E63` (pink) | `json` | StreamOutput reference. Real-time data channel. |

---

## Media Tags (nebula-media, optional plugin)

| Tag | Name | Color | Compatible With | Description |
|-----|------|-------|----------------|-------------|
| `image` | Image | `#4CAF50` (green) | `binary`, `file` | Raster image: PNG, JPEG, WebP, GIF, BMP, TIFF. |
| `image.svg` | SVG | `#66BB6A` (light-green) | `image`, `binary`, `file` | Vector image: SVG. |
| `audio` | Audio | `#FF5722` (deep-orange) | `binary`, `file` | Audio: MP3, WAV, OGG, FLAC, AAC. |
| `video` | Video | `#F44336` (red) | `binary`, `file` | Video: MP4, WebM, MOV, AVI. |
| `pdf` | PDF | `#D32F2F` (dark-red) | `binary`, `file` | PDF document. |
| `spreadsheet` | Spreadsheet | `#388E3C` (dark-green) | `binary`, `file` | Excel/CSV/ODS. |
| `document` | Document | `#1976D2` (dark-blue) | `binary`, `file` | DOCX, ODT, RTF, TXT. |
| `archive` | Archive | `#5D4037` (dark-brown) | `binary`, `file` | ZIP, TAR, GZ, 7Z. |
| `font` | Font | `#455A64` (dark-gray) | `binary`, `file` | TTF, OTF, WOFF, WOFF2. |

---

## AI / ML Tags (nebula-ai, optional plugin)

### Models & Inference

| Tag | Name | Color | Compatible With | Description |
|-----|------|-------|----------------|-------------|
| `ai.model` | AI Model | `#9C27B0` (purple) | — | Model reference (LLM, classifier, etc). Not data — a capability. |
| `ai.model.llm` | LLM | `#AB47BC` (light-purple) | `ai.model` | Large language model specifically. |
| `ai.model.vision` | Vision Model | `#8E24AA` (mid-purple) | `ai.model` | Image understanding model. |
| `ai.model.tts` | TTS Model | `#7B1FA2` (dark-purple) | `ai.model` | Text-to-speech model. |
| `ai.model.stt` | STT Model | `#6A1B9A` (deeper-purple) | `ai.model` | Speech-to-text model. |
| `ai.model.embedding` | Embedding Model | `#4A148C` (deepest-purple) | `ai.model` | Text/image embedding model. |

### Data Types

| Tag | Name | Color | Compatible With | Description |
|-----|------|-------|----------------|-------------|
| `ai.messages` | Chat Messages | `#CE93D8` (lavender) | `array`, `json` | Array of `{ role, content }` chat messages. |
| `ai.embedding` | Embedding Vector | `#FF9800` (orange) | `array`, `json` | Float vector (e.g., 1536-dim for ada-002). |
| `ai.prompt` | Structured Prompt | `#FFB74D` (light-orange) | `text`, `json` | Prompt template with variables. |
| `ai.tool_calls` | Tool Calls | `#FFA726` (mid-orange) | `array`, `json` | LLM tool/function call requests. |
| `ai.completion` | Completion | `#EF6C00` (dark-orange) | `text`, `json` | LLM completion result (text + metadata). |

### ComfyUI / Diffusion

| Tag | Name | Color | Compatible With | Description |
|-----|------|-------|----------------|-------------|
| `ai.latent` | Latent | `#E040FB` (magenta) | — | Latent space tensor (Stable Diffusion). |
| `ai.conditioning` | Conditioning | `#EA80FC` (light-magenta) | — | CLIP conditioning for diffusion. |
| `ai.clip` | CLIP | `#D500F9` (vivid-magenta) | `ai.model` | CLIP model reference. |
| `ai.vae` | VAE | `#AA00FF` (vivid-purple) | `ai.model` | VAE model reference. |
| `ai.controlnet` | ControlNet | `#7C4DFF` (deep-violet) | `ai.model` | ControlNet model reference. |
| `ai.lora` | LoRA | `#651FFF` (dark-violet) | — | LoRA adapter weights. |
| `ai.mask` | Mask | `#B0BEC5` (silver) | `image`, `binary` | Binary mask for inpainting/segmentation. |
| `ai.bbox` | Bounding Boxes | `#FFAB00` (amber) | `array`, `json` | Detection bounding boxes. |
| `ai.segmentation` | Segmentation Map | `#00E676` (neon-green) | `image`, `binary` | Pixel-level segmentation. |

---

## Data & Database Tags (nebula-data, optional plugin)

| Tag | Name | Color | Compatible With | Description |
|-----|------|-------|----------------|-------------|
| `data.rows` | Table Rows | `#26A69A` (teal) | `array`, `json` | Array of row objects (SQL result). |
| `data.row` | Single Row | `#00897B` (dark-teal) | `object`, `json` | Single row object. |
| `data.cursor` | Cursor | `#004D40` (deepest-teal) | `json` | Pagination cursor (opaque token or offset). |
| `data.schema` | Schema | `#80CBC4` (light-teal) | `object`, `json` | Table/collection schema definition. |
| `data.connection` | DB Connection | `#00695C` (dark-teal) | — | Database connection reference (resource handle). |

---

## Communication Tags (nebula-comm, optional plugin)

| Tag | Name | Color | Compatible With | Description |
|-----|------|-------|----------------|-------------|
| `email` | Email | `#3F51B5` (indigo) | `object`, `json` | Email message (to, subject, body, attachments). |
| `email.address` | Email Address | `#5C6BC0` (light-indigo) | `text`, `json` | Valid email address string. |
| `html` | HTML | `#0288D1` (light-blue) | `text`, `json` | HTML content string. |
| `markdown` | Markdown | `#0277BD` (mid-blue) | `text`, `json` | Markdown content string. |
| `xml` | XML | `#01579B` (dark-blue) | `text`, `json` | XML content string. |
| `url` | URL | `#039BE5` (sky-blue) | `text`, `json` | Valid URL string. |
| `datetime` | DateTime | `#00ACC1` (cyan) | `text`, `json` | ISO 8601 datetime string. |
| `cron` | Cron Expression | `#0097A7` (dark-cyan) | `text`, `json` | Cron schedule expression. |

---

## Integration-Specific Tags (registered by integration plugins)

| Tag | Name | Color | Compatible With | Registered By |
|-----|------|-------|----------------|---------------|
| `slack.message` | Slack Message | `#611F69` (slack-purple) | `object`, `json` | nebula-plugin-slack |
| `slack.channel` | Slack Channel | `#611F69` | `text`, `json` | nebula-plugin-slack |
| `github.event` | GitHub Event | `#24292E` (github-dark) | `object`, `json` | nebula-plugin-github |
| `github.pr` | Pull Request | `#24292E` | `object`, `json` | nebula-plugin-github |
| `telegram.update` | Telegram Update | `#0088CC` (telegram-blue) | `object`, `json` | nebula-plugin-telegram |
| `stripe.event` | Stripe Event | `#635BFF` (stripe-purple) | `object`, `json` | nebula-plugin-stripe |
| `gsheet.range` | Sheet Range | `#0F9D58` (sheets-green) | `data.rows`, `array`, `json` | nebula-plugin-google |
| `s3.object` | S3 Object | `#FF9900` (aws-orange) | `binary`, `file` | nebula-plugin-aws |

---

## Hierarchy (compatibility flows upward)

```
json (universal acceptor)
├── text
│   ├── html
│   ├── markdown
│   ├── xml
│   ├── url
│   ├── email.address
│   ├── datetime
│   └── cron
├── number
│   └── boolean
├── array
│   ├── data.rows
│   ├── ai.messages
│   ├── ai.embedding
│   ├── ai.tool_calls
│   └── ai.bbox
├── object
│   ├── data.row
│   ├── data.schema
│   ├── email
│   ├── slack.message
│   ├── github.event
│   ├── telegram.update
│   └── stripe.event
├── binary
│   ├── file
│   │   ├── image
│   │   │   ├── image.svg
│   │   │   ├── ai.mask
│   │   │   └── ai.segmentation
│   │   ├── audio
│   │   ├── video
│   │   ├── pdf
│   │   ├── spreadsheet
│   │   ├── document
│   │   ├── archive
│   │   ├── font
│   │   └── s3.object
│   └── ai.latent
├── stream
│   └── ai.completion (streaming)
└── (no parent — isolated types)
    ├── ai.model
    │   ├── ai.model.llm
    │   ├── ai.model.vision
    │   ├── ai.model.tts
    │   ├── ai.model.stt
    │   ├── ai.model.embedding
    │   ├── ai.clip
    │   ├── ai.vae
    │   └── ai.controlnet
    ├── ai.conditioning
    ├── ai.lora
    └── data.connection
```

## Naming Convention

```
Core:          single word              json, text, number, binary, stream
Media:         media type               image, audio, video, pdf
AI generic:    ai.{concept}             ai.model, ai.messages, ai.embedding
AI diffusion:  ai.{concept}             ai.latent, ai.conditioning, ai.vae
AI model sub:  ai.model.{type}          ai.model.llm, ai.model.vision
Data:          data.{concept}           data.rows, data.cursor
Comm:          format name              html, markdown, email, url
Integration:   {service}.{concept}      slack.message, github.event
```

**Rules:**
1. Lowercase, dot-separated hierarchy
2. Core tags = single word (no dots)
3. Domain tags = namespace.concept
4. Integration tags = service.concept
5. No version in tag name (versioning is on action, not data type)
6. New tags MUST be registered in DataTagRegistry — random strings rejected

## Count Summary

| Category | Count | Registered By |
|----------|-------|---------------|
| Core (primitives + binary + stream) | 9 | nebula-core |
| Media | 9 | nebula-media |
| AI / ML | 19 | nebula-ai |
| Data / Database | 5 | nebula-data |
| Communication | 8 | nebula-comm |
| Integration-specific | 8+ | individual plugins |
| **Total defined** | **58** | |
