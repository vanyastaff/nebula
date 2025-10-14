//! UI components for nebula-parameter using egui
//!
//! This crate provides egui-based widgets for rendering and editing
//! parameters from the nebula-parameter crate.
//!
//! # Features
//!
//! - **Theme system**: Consistent styling with dark/light themes
//! - **Validation display**: Visual feedback for validation errors
//! - **All parameter types**: Complete widget implementations
//! - **Responsive**: Works well with different screen sizes
//!
//! # Example
//!
//! ```no_run
//! use nebula_parameter::{TextParameter, ParameterMetadata};
//! use nebula_parameter_ui::{ParameterWidget, TextWidget};
//!
//! let param = TextParameter {
//!     metadata: ParameterMetadata::builder()
//!         .key("username")
//!         .name("Username")
//!         .required(true)
//!         .build()
//!         .unwrap(),
//!     value: None,
//!     default: None,
//!     options: None,
//!     display: None,
//!     validation: None,
//! };
//!
//! let mut widget = TextWidget::new(param);
//! // In your egui app:
//! // widget.render(ui);
//! ```

pub mod helpers;
pub mod theme;
pub mod layout;
// pub mod flex_layout;  // Temporarily disabled
pub mod grid_layout;
pub mod widgets;

use egui::{Response, Ui};
pub use theme::ParameterTheme;
pub use helpers::{ValidationState, ParameterContext};
pub use layout::{LayoutConfig, AdaptiveContainer, adaptive_container};
// pub use flex_layout::{FlexConfig, FlexContainer, flex_container, FlexDirection, JustifyContent, AlignItems};  // Temporarily disabled
pub use grid_layout::{GridConfig, GridContainer, grid_container};

/// Trait for rendering a parameter as an egui widget
pub trait ParameterWidget {
    /// Render the parameter widget and return the response
    ///
    /// # Arguments
    /// * `ui` - The egui UI context
    ///
    /// # Returns
    /// The UI response from rendering the widget
    fn render(&mut self, ui: &mut Ui) -> Response;

    /// Render with a custom theme
    ///
    /// # Arguments
    /// * `ui` - The egui UI context
    /// * `theme` - The theme to use for rendering
    ///
    /// # Returns
    /// The UI response from rendering the widget
    fn render_with_theme(&mut self, ui: &mut Ui, _theme: &ParameterTheme) -> Response {
        // Default implementation falls back to render()
        // Individual widgets can override this to use the theme
        self.render(ui)
    }

    /// Check if the parameter value has changed since last render
    fn has_changed(&self) -> bool;

    /// Reset the changed flag
    fn reset_changed(&mut self);
    
    /// Get validation state (if any)
    fn validation_state(&self) -> ValidationState {
        ValidationState::Valid
    }
}

/// Extension trait for rendering any parameter type
pub trait ParameterUiExt {
    /// Render this parameter as a UI widget
    fn ui(&mut self, ui: &mut Ui) -> Response;
}

// Re-export commonly used types
pub use widgets::*;
