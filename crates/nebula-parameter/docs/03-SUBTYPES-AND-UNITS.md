# Subtypes and Units

Complete reference for node subtypes and the unit system.

---

## Subtype + Unit Pattern

Inspired by Blender, Nebula separates **semantic meaning** (subtype) from **measurement system** (unit):

```rust
Number {
    subtype: NumberSubtype::Distance,  // WHAT it represents
    unit: NumberUnit::Length,          // HOW to measure/display
}
```

**Benefits:**
- Automatic unit conversion (meters <-> feet)
- User preference support (metric vs imperial)
- Localization-friendly display
- 20 subtypes x 30 units = 600+ combinations without enum explosion

---

## Composition Table

All specialized behaviors are built from **Base Type + Subtype + Flags**:

### Text-Based Specializations

| Specialization | Base | Subtype | Flags |
|----------------|------|---------|-------|
| Password | Text | Secret | SENSITIVE, WRITE_ONLY |
| API Key | Text | Secret | SENSITIVE, WRITE_ONLY |
| Email | Text | Email | - |
| URL | Text | Url | - |
| File Path | Text | FilePath | - |
| DateTime | Text | DateTime | - |
| Date | Text | Date | - |
| Time | Text | Time | - |
| Code (Python) | Text | Code(Python) | - |
| Code (SQL) | Text | Code(Sql) | - |
| JSON Editor | Text | Json | - |
| UUID | Text | Uuid | - |
| Multi-line | Text | MultiLine | - |

### Number-Based Specializations

| Specialization | Base | Subtype | Unit | Constraints |
|----------------|------|---------|------|-------------|
| Percentage | Number<f64> | Percentage | - | 0-100 |
| Port | Number<i64> | Port | - | 0-65535 |
| Rating | Number<i64> | Rating | - | 0-5 |
| Temperature | Number<f64> | Temperature | Temperature | - |
| Distance | Number<f64> | Distance | Length | - |
| Duration | Number<f64> | Duration | Time | - |
| Angle | Number<f64> | Angle | Rotation | - |

### Vector-Based Specializations

| Specialization | Base | Subtype |
|----------------|------|---------|
| Color RGB | Vector<f64, 3> | ColorRgb |
| Color RGBA | Vector<f64, 4> | ColorRgba |
| Color HSV | Vector<f64, 3> | ColorHsv |
| Position 2D | Vector<f64, 2> | Position2D |
| Position 3D | Vector<f64, 3> | Position |
| Scale | Vector<f64, 3> | Scale |
| Euler Angles | Vector<f64, 3> | Euler |
| Quaternion | Vector<f64, 4> | Quaternion |

---

## TextSubtype Reference

### Basic Text
| Subtype | Description | Validation | Placeholder |
|---------|-------------|------------|-------------|
| `Plain` | Generic text | None | - |
| `MultiLine` | Multi-line text area | None | - |

### Network & Identifiers
| Subtype | Description | Validation Pattern | Placeholder |
|---------|-------------|-------------------|-------------|
| `Email` | Email address | RFC 5322 | `user@example.com` |
| `Url` | HTTP(S) URL | URL format | `https://example.com` |
| `Uri` | Generic URI | URI format | - |
| `Domain` | Domain name | Domain format | `example.com` |
| `IpAddressV4` | IPv4 address | `\d{1,3}(\.\d{1,3}){3}` | `192.168.1.1` |
| `IpAddressV6` | IPv6 address | IPv6 format | `2001:0db8::1` |
| `MacAddress` | MAC address | `([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}` | `00:1B:44:11:3A:B7` |
| `Hostname` | Hostname | Hostname format | `server.example.com` |

### Paths & Files
| Subtype | Description | Placeholder |
|---------|-------------|-------------|
| `FilePath` | Full file path | `/path/to/file.txt` |
| `DirPath` | Directory path | `/path/to/dir` |
| `FileName` | File name only | `document.pdf` |
| `FileExtension` | Extension only | `.json` |
| `MimeType` | MIME type | `application/json` |

### Security
| Subtype | Description | Behavior |
|---------|-------------|----------|
| `Password` | Password input | Masked display |
| `Secret` | Secret token | Masked + sensitive flag |
| `ApiKey` | API key | Masked + validation |
| `BearerToken` | Bearer token | Masked |
| `SshKey` | SSH key | Multi-line + masked |
| `Certificate` | SSL certificate | PEM validation |

### Identifiers
| Subtype | Description | Validation | Placeholder |
|---------|-------------|------------|-------------|
| `Uuid` | UUID v4 | UUID format | `123e4567-e89b-12d3-a456-426614174000` |
| `Ulid` | ULID | ULID format | `01ARZ3NDEKTSV4RRFFQ69G5FAV` |
| `Slug` | URL slug | `^[a-z0-9]+(?:-[a-z0-9]+)*$` | `my-blog-post` |
| `Handle` | Username handle | `^@?[a-zA-Z0-9_]+$` | `@username` |

### Versioning
| Subtype | Description | Validation | Placeholder |
|---------|-------------|------------|-------------|
| `SemVer` | Semantic version | SemVer format | `1.2.3` |
| `GitRef` | Git reference | - | `main`, `v1.0.0` |
| `GitCommit` | Git commit SHA | Hex, 40 chars | - |

### Date/Time (ISO 8601)
| Subtype | Description | Placeholder |
|---------|-------------|-------------|
| `DateTime` | Full datetime | `2024-01-15T14:30:00Z` |
| `Date` | Date only | `2024-01-15` |
| `Time` | Time only | `14:30:00` |
| `Duration` | ISO duration | `P3Y6M4DT12H30M5S` |
| `Timestamp` | Unix timestamp | - |

### Locale & i18n
| Subtype | Description | Example |
|---------|-------------|---------|
| `LanguageCode` | ISO 639-1 | `en`, `ru`, `ja` |
| `CountryCode` | ISO 3166-1 alpha-2 | `US`, `RU`, `JP` |
| `LocaleCode` | Full locale | `en-US`, `ru-RU` |
| `CurrencyCode` | ISO 4217 | `USD`, `EUR`, `RUB` |
| `TimezoneId` | IANA timezone | `America/New_York` |

### Structured Data
| Subtype | Description | Syntax Highlighting |
|---------|-------------|---------------------|
| `Json` | JSON string | Yes |
| `Yaml` | YAML string | Yes |
| `Toml` | TOML string | Yes |
| `Xml` | XML string | Yes |
| `Csv` | CSV string | Yes |

### Code & Expressions
| Subtype | Description |
|---------|-------------|
| `Code` | Source code (language auto-detect) |
| `CodeWithLanguage(CodeLanguage)` | Source code with specific language |
| `Sql` | SQL query |
| `Regex` | Regular expression |
| `Glob` | Glob pattern (`**/*.rs`) |
| `JsonPath` | JSONPath expression |
| `XPath` | XPath expression |
| `CssSelector` | CSS selector |
| `Expression` | Template expression (`{{$json.data}}`) |

### CodeLanguage Enum (35+ Languages)

```rust
pub enum CodeLanguage {
    // === Systems Programming ===
    Rust,
    C,
    Cpp,        // C++
    Zig,
    
    // === JVM Languages ===
    Java,
    Kotlin,
    Scala,
    
    // === .NET Languages ===
    CSharp,     // C#
    FSharp,     // F#
    
    // === Web Languages ===
    JavaScript,
    TypeScript,
    
    // === Scripting Languages ===
    Python,
    Ruby,
    PHP,
    Perl,
    Lua,
    
    // === Functional Languages ===
    Haskell,
    OCaml,
    Elixir,
    Erlang,
    Clojure,
    
    // === Modern Languages ===
    Go,
    Swift,
    Dart,
    V,
    Nim,
    Crystal,
    
    // === Scientific ===
    R,
    Matlab,
    
    // === Shell ===
    Shell,      // Bash, sh, zsh
    PowerShell,
    
    // === Database ===
    Sql,
    
    // === Legacy ===
    Fortran,
    Cobol,
    
    // === Low-Level ===
    Assembly,
    WebAssembly,
    
    // === Custom ===
    Custom(String),
}

impl CodeLanguage {
    /// Get file extension for this language
    pub fn file_extension(&self) -> &'static str;
    
    /// Get language name as string
    pub fn as_str(&self) -> &str;
}
```

**File Extensions:**
| Language | Extension |
|----------|-----------|
| Rust | `.rs` |
| Python | `.py` |
| JavaScript | `.js` |
| TypeScript | `.ts` |
| Go | `.go` |
| Java | `.java` |
| C | `.c` |
| C++ | `.cpp` |
| C# | `.cs` |
| Ruby | `.rb` |
| PHP | `.php` |
| Swift | `.swift` |
| Kotlin | `.kt` |
| Scala | `.scala` |
| Haskell | `.hs` |
| Elixir | `.ex` |
| Erlang | `.erl` |
| Clojure | `.clj` |
| Lua | `.lua` |
| R | `.r` |
| MATLAB | `.m` |
| SQL | `.sql` |
| Shell | `.sh` |
| PowerShell | `.ps1` |
| Perl | `.pl` |
| Dart | `.dart` |
| Zig | `.zig` |
| V | `.v` |
| Nim | `.nim` |
| Crystal | `.cr` |
| OCaml | `.ml` |
| F# | `.fs` |
| Fortran | `.f90` |
| COBOL | `.cob` |
| Assembly | `.asm` |
| WebAssembly | `.wat` |

**Usage:**
```rust
// Code with specific language
Text::builder("script")
    .subtype(TextSubtype::Code(CodeLanguage::Python))
    .build()

// Or use convenience constructor
Text::builder("query")
    .subtype(TextSubtype::Code(CodeLanguage::Sql))
    .build()
```

### Templates
| Subtype | Description | Example |
|---------|-------------|---------|
| `Mustache` | Mustache template | `Hello {{name}}!` |
| `Handlebars` | Handlebars template | `{{#if active}}...{{/if}}` |
| `Jinja2` | Jinja2 template | `{{ user.name }}` |

---

## NumberSubtype Reference

### Basic
| Subtype | Description | Default Range |
|---------|-------------|---------------|
| `Integer` | Whole numbers | - |
| `Float` | Decimal numbers | - |
| `Unsigned` | Non-negative | >= 0 |

### Normalized
| Subtype | Description | Range |
|---------|-------------|-------|
| `Factor` | Normalized value | 0.0 - 1.0 |
| `Percentage` | Percentage | 0 - 100 |

### Angular
| Subtype | Description | Unit |
|---------|-------------|------|
| `Angle` | Rotation angle | Degrees |
| `AngleDegrees` | Explicit degrees | ° |
| `AngleRadians` | Radians | rad |

### Spatial
| Subtype | Description |
|---------|-------------|
| `Distance` | Linear distance |
| `Area` | 2D area |
| `Volume` | 3D volume |

### Temporal
| Subtype | Description |
|---------|-------------|
| `Time` | Point in time |
| `Duration` | Time span |
| `TimeAbsolute` | Absolute timestamp |

### Motion
| Subtype | Description |
|---------|-------------|
| `Velocity` | Speed |
| `Acceleration` | Rate of velocity change |
| `AngularVelocity` | Rotation speed |

### Physical
| Subtype | Description |
|---------|-------------|
| `Mass` | Mass/weight |
| `Force` | Force |
| `Energy` | Energy |
| `Power` | Power/wattage |
| `Pressure` | Pressure |
| `Temperature` | Temperature |

### Data
| Subtype | Description |
|---------|-------------|
| `Bytes` | Data size |
| `BitRate` | Data rate |

### Finance
| Subtype | Description |
|---------|-------------|
| `Currency` | Money value |

### Visual
| Subtype | Description |
|---------|-------------|
| `Pixel` | Pixel count |
| `ColorChannel` | Color component (0-1 or 0-255) |
| `Luminance` | Light intensity |

---

## NumberUnit Reference

Units provide **measurement system** selection and automatic conversion.

### Length Units
| Unit | Description | Base | Suffix |
|------|-------------|------|--------|
| `Length` | User preference | m | auto |
| `LengthMetric` | Metric system | m | mm/cm/m/km |
| `LengthImperial` | Imperial system | m | in/ft/yd/mi |

**Auto-selection (metric):**
- < 0.01m -> mm
- < 1.0m -> cm
- < 1000m -> m
- >= 1000m -> km

### Rotation Units
| Unit | Description | Base | Suffix |
|------|-------------|------|--------|
| `Rotation` | User preference | rad | ° or rad |
| `RotationDegrees` | Always degrees | ° | ° |
| `RotationRadians` | Always radians | rad | rad |

### Time Units
| Unit | Description | Suffix |
|------|-------------|--------|
| `Time` | User preference | auto |
| `TimeSeconds` | Seconds | s/ms/us/ns |
| `TimeMinutes` | Minutes | min/h/d |
| `TimeFrames` | Animation frames | frames |

### Temperature Units
| Unit | Description | Suffix |
|------|-------------|--------|
| `Temperature` | User preference | auto |
| `TemperatureCelsius` | Celsius | °C |
| `TemperatureFahrenheit` | Fahrenheit | °F |
| `TemperatureKelvin` | Kelvin | K |

### Mass Units
| Unit | Description | Suffix |
|------|-------------|--------|
| `Mass` | User preference | auto |
| `MassMetric` | Metric | g/kg/ton |
| `MassImperial` | Imperial | oz/lb |

### Data Units
| Unit | Description | Suffix |
|------|-------------|--------|
| `BytesDecimal` | 1000-based | KB/MB/GB |
| `BytesBinary` | 1024-based | KiB/MiB/GiB |

---

## VectorSubtype Reference

### Spatial
| Subtype | Components | Description |
|---------|------------|-------------|
| `Position` | 3 | 3D position (XYZ) |
| `Translation` | 3 | Translation vector |
| `Direction` | 3 | Normalized direction |
| `Normal` | 3 | Surface normal |
| `Scale` | 3 | Scale factors |

### Rotation
| Subtype | Components | Description |
|---------|------------|-------------|
| `Euler` | 3 | Euler angles (XYZ) |
| `EulerRadians` | 3 | Euler in radians |
| `Quaternion` | 4 | Quaternion (XYZW) |
| `AxisAngle` | 4 | Axis-angle |

### Color
| Subtype | Components | Range |
|---------|------------|-------|
| `Color` | 3 | RGB (0-1) |
| `ColorRgba` | 4 | RGBA (0-1) |
| `ColorHsv` | 3 | HSV |
| `ColorHsl` | 3 | HSL |

### Texture
| Subtype | Components | Description |
|---------|------------|-------------|
| `Uv` | 2 | Texture coordinates |
| `Uvw` | 3 | 3D texture coordinates |

### 2D
| Subtype | Components | Description |
|---------|------------|-------------|
| `Position2D` | 2 | 2D position |
| `Size2D` | 2 | Width/height |

---

## User Preferences

Units can adapt to user preferences:

```rust
pub struct UserPreferences {
    pub unit_system: UnitSystem,      // Metric | Imperial
    pub angle_unit: AngleUnit,        // Degrees | Radians
    pub temperature_unit: TemperatureUnit,  // Celsius | Fahrenheit | Kelvin
}
```

**Example: Same data, different display:**
```
American user:  "5.9 ft"
European user:  "1.8 m"
Stored value:   1.8 (meters)
```

---

## Convenience Constructors

```rust
// Distance with user-preferred units
Number::distance("height")
    .range(0.0, 10.0)
    .default(1.8)
    .build()

// Angle always in degrees
Number::angle_degrees("rotation")
    .range(0.0, 360.0)
    .build()

// Temperature with user preference
Number::temperature("cpu_temp")
    .range(0.0, 100.0)
    .build()

// Duration in seconds
Number::duration_seconds("timeout")
    .range(1.0, 300.0)
    .default(30.0)
    .build()

// Position vector
Vector::<f64, 3>::position("pos")
    .component_units(NumberUnit::Length)
    .build()

// RGBA color
Vector::<f64, 4>::color_rgba("tint")
    .default([1.0, 1.0, 1.0, 1.0])
    .build()
```

---

## Unit Conversion

Units handle automatic conversion between display and storage:

```rust
impl NumberUnit {
    /// Convert from base unit to display unit
    pub fn from_base(&self, value: f64, target: &str) -> f64;
    
    /// Convert from display unit to base unit
    pub fn to_base(&self, value: f64, source: &str) -> f64;
    
    /// Get display suffix based on value magnitude
    pub fn display_suffix(&self, value: f64, prefs: &UserPreferences) -> &str;
}
```

**Conversion examples:**
```rust
// Length: meters <-> feet
LengthMetric.from_base(1.8, "m")   // 1.8
LengthImperial.from_base(1.8, "ft") // 5.9

// Temperature: Celsius <-> Fahrenheit
TemperatureCelsius.to_base(20.0, "°C")    // 20.0
TemperatureFahrenheit.to_base(68.0, "°F") // 20.0

// Rotation: radians <-> degrees
RotationRadians.from_base(PI, "rad")   // 3.14159
RotationDegrees.from_base(PI, "°")     // 180.0
```
