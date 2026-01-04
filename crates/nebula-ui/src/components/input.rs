//! Text and number input components.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Vec2, Widget};

/// Text input component with label, hint, and validation
///
/// # Example
///
/// ```rust,ignore
/// let mut name = String::new();
/// TextInput::new(&mut name)
///     .label("Name")
///     .placeholder("Enter your name")
///     .show(ui);
/// ```
pub struct TextInput<'a> {
    value: &'a mut String,
    label: Option<&'a str>,
    placeholder: &'a str,
    hint: Option<&'a str>,
    error: Option<&'a str>,
    password: bool,
    disabled: bool,
    multiline: bool,
    max_length: Option<usize>,
    min_width: Option<f32>,
    full_width: bool,
    icon_left: Option<&'a str>,
}

impl<'a> TextInput<'a> {
    /// Create a new text input bound to the given string
    pub fn new(value: &'a mut String) -> Self {
        Self {
            value,
            label: None,
            placeholder: "",
            hint: None,
            error: None,
            password: false,
            disabled: false,
            multiline: false,
            max_length: None,
            min_width: None,
            full_width: false,
            icon_left: None,
        }
    }

    /// Set the label above the input
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Set placeholder text
    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// Set hint text below the input
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// Set error message (displays in red)
    pub fn error(mut self, error: &'a str) -> Self {
        self.error = Some(error);
        self
    }

    /// Make this a password field
    pub fn password(mut self) -> Self {
        self.password = true;
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set maximum character length
    pub fn max_length(mut self, max: usize) -> Self {
        self.max_length = Some(max);
        self
    }

    /// Set minimum width
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = Some(width);
        self
    }

    /// Make input full width
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// Add an icon to the left
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon_left = Some(icon);
        self
    }

    /// Show the input and return the response
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for TextInput<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let has_error = self.error.is_some();

        ui.vertical(|ui| {
            // Label
            if let Some(label) = self.label {
                ui.add_space(tokens.spacing_xs);
                ui.label(
                    RichText::new(label)
                        .size(tokens.font_size_sm)
                        .color(tokens.foreground),
                );
                ui.add_space(tokens.spacing_xs);
            }

            // Input field
            let desired_width = if self.full_width {
                ui.available_width()
            } else {
                self.min_width.unwrap_or(200.0)
            };

            let text_edit = if self.password {
                egui::TextEdit::singleline(self.value).password(true)
            } else if self.multiline {
                egui::TextEdit::multiline(self.value)
            } else {
                egui::TextEdit::singleline(self.value)
            };

            let border_color = if has_error {
                tokens.destructive
            } else {
                tokens.border
            };

            // Custom frame for the input
            let frame = egui::Frame::NONE
                .fill(tokens.input)
                .stroke(egui::Stroke::new(1.0, border_color))
                .corner_radius(tokens.rounding_md())
                .inner_margin(egui::Margin::symmetric(
                    tokens.spacing_md as i8,
                    tokens.spacing_sm as i8,
                ));

            let response = frame
                .show(ui, |ui| {
                    ui.add_enabled(
                        !self.disabled,
                        text_edit
                            .hint_text(self.placeholder)
                            .desired_width(desired_width - tokens.spacing_md * 2.0)
                            .margin(Vec2::ZERO)
                            .frame(false),
                    )
                })
                .inner;

            // Hint or Error
            if let Some(error) = self.error {
                ui.add_space(tokens.spacing_xs);
                ui.label(
                    RichText::new(error)
                        .size(tokens.font_size_xs)
                        .color(tokens.destructive),
                );
            } else if let Some(hint) = self.hint {
                ui.add_space(tokens.spacing_xs);
                ui.label(
                    RichText::new(hint)
                        .size(tokens.font_size_xs)
                        .color(tokens.muted_foreground),
                );
            }

            // Enforce max length
            if let Some(max) = self.max_length {
                if self.value.len() > max {
                    self.value.truncate(max);
                }
            }

            response
        })
        .inner
    }
}

/// Number input component
///
/// # Example
///
/// ```rust,ignore
/// let mut count = 0i64;
/// NumberInput::new(&mut count)
///     .label("Count")
///     .min(0)
///     .max(100)
///     .show(ui);
/// ```
pub struct NumberInput<'a, T: egui::emath::Numeric> {
    value: &'a mut T,
    label: Option<&'a str>,
    hint: Option<&'a str>,
    error: Option<&'a str>,
    min: Option<T>,
    max: Option<T>,
    step: Option<T>,
    disabled: bool,
    min_width: Option<f32>,
    full_width: bool,
    prefix: Option<&'a str>,
    suffix: Option<&'a str>,
}

impl<'a, T: egui::emath::Numeric> NumberInput<'a, T> {
    /// Create a new number input
    pub fn new(value: &'a mut T) -> Self {
        Self {
            value,
            label: None,
            hint: None,
            error: None,
            min: None,
            max: None,
            step: None,
            disabled: false,
            min_width: None,
            full_width: false,
            prefix: None,
            suffix: None,
        }
    }

    /// Set the label
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Set hint text
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// Set error message
    pub fn error(mut self, error: &'a str) -> Self {
        self.error = Some(error);
        self
    }

    /// Set minimum value
    pub fn min(mut self, min: T) -> Self {
        self.min = Some(min);
        self
    }

    /// Set maximum value
    pub fn max(mut self, max: T) -> Self {
        self.max = Some(max);
        self
    }

    /// Set step increment
    pub fn step(mut self, step: T) -> Self {
        self.step = Some(step);
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set minimum width
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = Some(width);
        self
    }

    /// Make full width
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// Add prefix (e.g., "$")
    pub fn prefix(mut self, prefix: &'a str) -> Self {
        self.prefix = Some(prefix);
        self
    }

    /// Add suffix (e.g., "px")
    pub fn suffix(mut self, suffix: &'a str) -> Self {
        self.suffix = Some(suffix);
        self
    }

    /// Show the input
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a, T: egui::emath::Numeric> Widget for NumberInput<'a, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let has_error = self.error.is_some();

        ui.vertical(|ui| {
            // Label
            if let Some(label) = self.label {
                ui.add_space(tokens.spacing_xs);
                ui.label(
                    RichText::new(label)
                        .size(tokens.font_size_sm)
                        .color(tokens.foreground),
                );
                ui.add_space(tokens.spacing_xs);
            }

            // Build drag value
            let mut drag = egui::DragValue::new(self.value);

            if let Some(min) = self.min {
                drag = drag.range(min..=self.max.unwrap_or(min));
            } else if let Some(max) = self.max {
                drag = drag.range(T::from_f64(f64::NEG_INFINITY)..=max);
            }

            if let Some(prefix) = self.prefix {
                drag = drag.prefix(prefix);
            }

            if let Some(suffix) = self.suffix {
                drag = drag.suffix(suffix);
            }

            let border_color = if has_error {
                tokens.destructive
            } else {
                tokens.border
            };

            let frame = egui::Frame::NONE
                .fill(tokens.input)
                .stroke(egui::Stroke::new(1.0, border_color))
                .corner_radius(tokens.rounding_md())
                .inner_margin(egui::Margin::symmetric(
                    tokens.spacing_md as i8,
                    tokens.spacing_sm as i8,
                ));

            let response = frame
                .show(ui, |ui| ui.add_enabled(!self.disabled, drag))
                .inner;

            // Hint or Error
            if let Some(error) = self.error {
                ui.add_space(tokens.spacing_xs);
                ui.label(
                    RichText::new(error)
                        .size(tokens.font_size_xs)
                        .color(tokens.destructive),
                );
            } else if let Some(hint) = self.hint {
                ui.add_space(tokens.spacing_xs);
                ui.label(
                    RichText::new(hint)
                        .size(tokens.font_size_xs)
                        .color(tokens.muted_foreground),
                );
            }

            response
        })
        .inner
    }
}

/// Multi-line text area
pub struct TextArea<'a> {
    value: &'a mut String,
    label: Option<&'a str>,
    placeholder: &'a str,
    hint: Option<&'a str>,
    error: Option<&'a str>,
    rows: usize,
    disabled: bool,
    full_width: bool,
    code: bool,
}

impl<'a> TextArea<'a> {
    /// Create a new text area
    pub fn new(value: &'a mut String) -> Self {
        Self {
            value,
            label: None,
            placeholder: "",
            hint: None,
            error: None,
            rows: 4,
            disabled: false,
            full_width: false,
            code: false,
        }
    }

    /// Set label
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Set placeholder
    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// Set hint text
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// Set error message
    pub fn error(mut self, error: &'a str) -> Self {
        self.error = Some(error);
        self
    }

    /// Set number of visible rows
    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Make full width
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// Use monospace font (for code)
    pub fn code(mut self) -> Self {
        self.code = true;
        self
    }

    /// Show the text area
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for TextArea<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let has_error = self.error.is_some();

        ui.vertical(|ui| {
            // Label
            if let Some(label) = self.label {
                ui.add_space(tokens.spacing_xs);
                ui.label(
                    RichText::new(label)
                        .size(tokens.font_size_sm)
                        .color(tokens.foreground),
                );
                ui.add_space(tokens.spacing_xs);
            }

            let desired_width = if self.full_width {
                ui.available_width()
            } else {
                300.0
            };

            let mut text_edit = egui::TextEdit::multiline(self.value)
                .hint_text(self.placeholder)
                .desired_width(desired_width)
                .desired_rows(self.rows);

            if self.code {
                text_edit = text_edit.code_editor();
            }

            let border_color = if has_error {
                tokens.destructive
            } else {
                tokens.border
            };

            let frame = egui::Frame::NONE
                .fill(tokens.input)
                .stroke(egui::Stroke::new(1.0, border_color))
                .corner_radius(tokens.rounding_md())
                .inner_margin(egui::Margin::same(tokens.spacing_md as i8));

            let response = frame
                .show(ui, |ui| ui.add_enabled(!self.disabled, text_edit))
                .inner;

            // Hint or Error
            if let Some(error) = self.error {
                ui.add_space(tokens.spacing_xs);
                ui.label(
                    RichText::new(error)
                        .size(tokens.font_size_xs)
                        .color(tokens.destructive),
                );
            } else if let Some(hint) = self.hint {
                ui.add_space(tokens.spacing_xs);
                ui.label(
                    RichText::new(hint)
                        .size(tokens.font_size_xs)
                        .color(tokens.muted_foreground),
                );
            }

            response
        })
        .inner
    }
}
