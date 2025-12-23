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
//! // widget.show(ui, &theme);
//! ```

pub mod theme;
pub mod traits;
pub mod widgets;

pub use theme::ParameterTheme;
pub use traits::{ParameterWidget, UiExt, WidgetResponse};

// Re-export commonly used types
pub use widgets::*;
