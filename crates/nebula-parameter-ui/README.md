# Nebula Parameter UI

[![Crates.io](https://img.shields.io/crates/v/nebula-parameter-ui.svg)](https://crates.io/crates/nebula-parameter-ui)
[![Documentation](https://docs.rs/nebula-parameter-ui/badge.svg)](https://docs.rs/nebula-parameter-ui)

**Professional UI components for nebula-parameter using egui**

This crate provides a complete set of beautifully designed, type-safe widgets for rendering and editing parameters from the `nebula-parameter` crate. Built with egui, it offers a responsive, cross-platform UI solution with comprehensive theme support.

## Features

✨ **Complete Widget Set**: All parameter types are supported with dedicated widgets  
🎨 **Theme System**: Built-in dark/light themes with customizable styling  
✅ **Validation Display**: Visual feedback for validation errors and required fields  
📱 **Adaptive Layout**: Responsive grid system that adapts to screen size  
🚀 **Performance**: Efficient rendering with minimal allocations  
🔧 **Type-Safe**: Full type safety with Rust's type system  
🎯 **Smart Sizing**: Widgets automatically adapt to container size and available space

## Supported Widgets

### Basic Input Widgets
- **TextWidget** - Single-line text input with validation
- **TextareaWidget** - Multi-line text input
- **NumberWidget** - Numeric input with drag value control
- **SliderWidget** - Numeric slider with range control
- **CheckboxWidget** - Boolean toggle with help text
- **SecretWidget** - Password field with show/hide toggle

### Selection Widgets
- **SelectWidget** - Dropdown selection from options
- **RadioWidget** - Radio button group for single selection
- **MultiSelectWidget** - Checkbox list for multiple selections

### Date/Time Widgets
- **DateWidget** - Date picker input
- **TimeWidget** - Time picker input
- **DateTimeWidget** - Combined date and time picker

### Specialized Widgets
- **ColorWidget** - Color picker with hex display
- **CodeWidget** - Code editor with syntax highlighting support
- **FileWidget** - File picker with upload support

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
nebula-parameter = { path = "../nebula-parameter" }
nebula-parameter-ui = { path = "../nebula-parameter-ui" }
eframe = "0.33"
```

### Basic Usage

```rust
use nebula_parameter::{TextParameter, ParameterMetadata};
use nebula_parameter_ui::{ParameterWidget, TextWidget};

// Create a parameter
let param = TextParameter {
    metadata: ParameterMetadata::builder()
        .key("username")
        .name("Username")
        .description("Enter your username")
        .required(true)
        .build()
        .unwrap(),
    value: None,
    default: None,
    options: None,
    display: None,
    validation: None,
};

// Create widget
let mut widget = TextWidget::new(param);

// In your egui app
widget.render(ui);
```

### Using Themes

```rust
use nebula_parameter_ui::ParameterTheme;

// Create a theme
let theme = ParameterTheme::dark();

// Or light theme
// let theme = ParameterTheme::light();

// Render with theme
widget.render_with_theme(ui, &theme);
```

### Handling Changes

```rust
if widget.has_changed() {
    println!("Value changed!");
    widget.reset_changed();
}

// Get the parameter with updated value
let parameter = widget.into_parameter();
```

## Examples

### Running the Comprehensive Demo

The crate includes a comprehensive demo showcasing all widgets:

```bash
cargo run --example comprehensive_demo
```

This demo features:
- All parameter widget types
- Theme switching (dark/light)
- Validation examples
- Required field handling
- Interactive examples

### Simple Example

See `examples/demo.rs` for a basic example:

```bash
cargo run --example demo
```

## Architecture

### Adaptive Layout System

The adaptive layout system automatically adjusts widget placement based on available space:

```rust
pub struct LayoutConfig {
    pub max_width: Option<f32>,
    pub min_width: Option<f32>,
    pub columns: usize,
    pub column_spacing: f32,
    pub responsive: bool,
}
```

**Layout Features:**
- **Responsive Grid**: Automatically calculates optimal number of columns
- **Smart Sizing**: Widgets adapt to available space with min/max constraints
- **Flexible Configuration**: Customizable spacing and column behavior
- **Multiple Layouts**: Single, two, three column, or fully responsive

### Theme System

The theme system provides consistent styling across all widgets:

```rust
pub struct ParameterTheme {
    pub colors: ThemeColors,
    pub fonts: ThemeFonts,
    pub spacing: ThemeSpacing,
    pub visuals: ThemeVisuals,
}
```

**Available Themes:**
- `ParameterTheme::dark()` - Professional dark theme
- `ParameterTheme::light()` - Clean light theme

### Validation Display

All widgets support automatic validation display:

```rust
pub enum ValidationState {
    Valid,
    Required,
    Error(String),
    Warning(String),
}
```

Validation messages are automatically shown based on:
- Required field state
- Custom validation errors
- Parameter-specific rules

### Widget Trait

All widgets implement the `ParameterWidget` trait:

```rust
pub trait ParameterWidget {
    fn render(&mut self, ui: &mut Ui) -> Response;
    fn render_with_theme(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> Response;
    fn has_changed(&self) -> bool;
    fn reset_changed(&mut self);
    fn validation_state(&self) -> ValidationState;
}
```

## Customization

### Custom Themes

Create your own theme by customizing the `ParameterTheme`:

```rust
let mut theme = ParameterTheme::dark();
theme.colors.label = Color32::from_rgb(255, 200, 100);
theme.fonts.label = FontId::proportional(16.0);
```

### Custom Styling

Use the helper functions for custom rendering:

```rust
use nebula_parameter_ui::helpers::{
    render_label, render_description, render_validation
};

// Custom parameter field
render_label(ui, &metadata, &theme);
// ... your custom input ...
render_validation(ui, &validation, &theme);
```

## Design Principles

1. **Consistency**: All widgets follow the same design patterns and styling
2. **Accessibility**: Clear labels, descriptions, and error messages
3. **Responsiveness**: Widgets automatically expand to fill container width (`desired_width(f32::INFINITY)`)
4. **Performance**: Minimal allocations and efficient rendering
5. **Type Safety**: Full compile-time type checking
6. **Adaptive Layout**: All input fields adapt to their container size for better UX

## Contributing

Contributions are welcome! Areas for improvement:

- **Container Widgets**: Group, Object, List, and Panel widgets
- **Advanced Date Pickers**: Integration with date picker libraries
- **Syntax Highlighting**: Enhanced code widget with syntax highlighting
- **File Upload**: Native file picker integration
- **Accessibility**: Enhanced keyboard navigation and screen reader support

## License

This project is licensed under the MIT License - see the [LICENSE](../../LICENSE) file for details.

## Related Crates

- [`nebula-parameter`](../nebula-parameter) - Core parameter types and validation
- [`nebula-value`](../nebula-value) - Value types used by parameters
- [`egui`](https://github.com/emilk/egui) - Immediate mode GUI framework

## Screenshots

### Dark Theme
All widgets with professional dark theme styling, validation indicators, and consistent spacing.

### Light Theme
Clean light theme with excellent contrast and readability.

---

**Built with ❤️ for the Nebula workflow engine**

