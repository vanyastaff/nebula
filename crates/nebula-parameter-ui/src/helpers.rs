//! Helper functions for rendering parameter UI components

use egui::{Response, RichText, Ui, Frame};
use nebula_parameter::ParameterMetadata;
use crate::theme::ParameterTheme;

/// Validation state for a parameter
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationState {
    /// No validation errors
    Valid,
    /// Field is required but empty
    Required,
    /// Custom validation error
    Error(String),
    /// Warning message
    Warning(String),
}

/// Context for rendering parameter fields
pub struct ParameterContext<'a> {
    /// The parameter metadata
    pub metadata: &'a ParameterMetadata,
    /// Whether the parameter is required
    pub is_required: bool,
    /// Whether the parameter has a value
    pub has_value: bool,
    /// Validation state
    pub validation: ValidationState,
    /// Whether the field is focused
    pub is_focused: bool,
    /// Theme to use
    pub theme: &'a ParameterTheme,
}

impl<'a> ParameterContext<'a> {
    /// Create a new parameter context
    pub fn new(metadata: &'a ParameterMetadata, theme: &'a ParameterTheme) -> Self {
        Self {
            metadata,
            is_required: metadata.required,
            has_value: false,
            validation: ValidationState::Valid,
            is_focused: false,
            theme,
        }
    }
    
    /// Set whether the parameter has a value
    pub fn with_value(mut self, has_value: bool) -> Self {
        self.has_value = has_value;
        self
    }
    
    /// Set validation state
    pub fn with_validation(mut self, validation: ValidationState) -> Self {
        self.validation = validation;
        self
    }
    
    /// Set focus state
    pub fn with_focus(mut self, is_focused: bool) -> Self {
        self.is_focused = is_focused;
        self
    }
}

/// Render a parameter label with optional required indicator
pub fn render_label(ui: &mut Ui, metadata: &ParameterMetadata, theme: &ParameterTheme) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        
        // Label text
        ui.label(
            RichText::new(&metadata.name)
                .color(theme.colors.label)
                .font(theme.fonts.label.clone())
                .strong()
        );
        
        // Required indicator
        if metadata.required {
            ui.label(
                RichText::new("*")
                    .color(theme.colors.required)
                    .strong()
            );
        }
    }).response
}

/// Render parameter description if present
pub fn render_description(ui: &mut Ui, metadata: &ParameterMetadata, theme: &ParameterTheme) {
    if !metadata.description.is_empty() {
        ui.add_space(theme.spacing.description_spacing);
        ui.label(
            RichText::new(&metadata.description)
                .font(theme.fonts.description.clone())
                .color(theme.colors.description)
        );
        ui.add_space(theme.spacing.description_spacing);
    }
}

/// Render parameter hint if present
pub fn render_hint(ui: &mut Ui, metadata: &ParameterMetadata, theme: &ParameterTheme) {
    if let Some(hint) = &metadata.hint {
        ui.add_space(theme.spacing.hint_spacing);
        ui.label(
            RichText::new(hint)
                .font(theme.fonts.hint.clone())
                .italics()
                .color(theme.colors.hint)
        );
    }
}

/// Render validation message based on state
pub fn render_validation(ui: &mut Ui, validation: &ValidationState, theme: &ParameterTheme) {
    match validation {
        ValidationState::Valid => {}
        ValidationState::Required => {
            ui.add_space(theme.spacing.error_spacing);
            ui.label(
                RichText::new("⚠ This field is required")
                    .font(theme.fonts.error.clone())
                    .color(theme.colors.error)
            );
        }
        ValidationState::Error(msg) => {
            ui.add_space(theme.spacing.error_spacing);
            ui.label(
                RichText::new(format!("⚠ {}", msg))
                    .font(theme.fonts.error.clone())
                    .color(theme.colors.error)
            );
        }
        ValidationState::Warning(msg) => {
            ui.add_space(theme.spacing.hint_spacing);
            ui.label(
                RichText::new(format!("⚠ {}", msg))
                    .font(theme.fonts.hint.clone())
                    .color(theme.colors.warning)
            );
        }
    }
}

/// Render a complete parameter field with label, description, hint, and validation
pub fn render_parameter_field<F>(
    ui: &mut Ui,
    context: &ParameterContext<'_>,
    render_input: F,
) -> Response
where
    F: FnOnce(&mut Ui) -> Response,
{
    ui.vertical(|ui| {
        // Add some vertical spacing
        ui.add_space(context.theme.spacing.field_spacing / 2.0);
        
        // Label with required indicator
        render_label(ui, context.metadata, context.theme);
        ui.add_space(context.theme.spacing.label_spacing);
        
        // Description
        render_description(ui, context.metadata, context.theme);
        
        // Input field with proper styling
        let response = render_input(ui);
        
        // Hint
        render_hint(ui, context.metadata, context.theme);
        
        // Validation message
        let validation = if !context.has_value && context.is_required {
            &ValidationState::Required
        } else {
            &context.validation
        };
        render_validation(ui, validation, context.theme);
        
        // Add spacing after the field
        ui.add_space(context.theme.spacing.field_spacing / 2.0);
        
        response
    }).inner
}

/// Create a styled frame for input fields
pub fn create_input_frame(theme: &ParameterTheme, is_focused: bool, has_error: bool) -> Frame {
    let stroke = if has_error {
        theme.visuals.error_stroke(&theme.colors)
    } else if is_focused {
        theme.visuals.focused_stroke(&theme.colors)
    } else {
        theme.visuals.normal_stroke(&theme.colors)
    };
    
    Frame::new()
        .stroke(stroke)
        .corner_radius(theme.visuals.input_rounding)
        .inner_margin(theme.spacing.input_padding)
        .fill(theme.colors.background)
}

/// Backward compatibility - render with default theme
pub fn render_parameter_field_compat<F>(
    ui: &mut Ui,
    metadata: &ParameterMetadata,
    _is_required: bool,  // Deprecated: now read from metadata
    has_value: bool,
    render_input: F,
) -> Response
where
    F: FnOnce(&mut Ui) -> Response,
{
    let theme = ParameterTheme::default();
    let context = ParameterContext::new(metadata, &theme)
        .with_value(has_value);
    
    render_parameter_field(ui, &context, render_input)
}
