//! Label component for form fields and text display.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Widget};

/// A styled label component
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::Label;
///
/// // Using Widget trait
/// ui.add(Label::new("Username").required());
///
/// // Or with show method
/// Label::new("Username").required().show(ui);
/// ```
pub struct Label<'a> {
    text: &'a str,
    required: bool,
    disabled: bool,
    description: Option<&'a str>,
}

impl<'a> Label<'a> {
    /// Create a new label with the given text
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            required: false,
            disabled: false,
            description: None,
        }
    }

    /// Mark as required (shows asterisk)
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Set disabled state (muted appearance)
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Add description text below the label
    pub fn description(mut self, text: &'a str) -> Self {
        self.description = Some(text);
        self
    }

    /// Show the label and return the response
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }

    /// Show the label with an associated widget
    pub fn show_with<R>(self, ui: &mut Ui, add_widget: impl FnOnce(&mut Ui) -> R) -> R {
        ui.vertical(|ui| {
            ui.add(self);
            ui.add_space(4.0);
            add_widget(ui)
        })
        .inner
    }
}

impl<'a> Widget for Label<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let text_color = if self.disabled {
            tokens.muted_foreground
        } else {
            tokens.foreground
        };

        let response = ui.horizontal(|ui| {
            ui.label(
                RichText::new(self.text)
                    .size(tokens.font_size_sm)
                    .color(text_color),
            );

            if self.required {
                ui.label(
                    RichText::new("*")
                        .size(tokens.font_size_sm)
                        .color(tokens.destructive),
                );
            }
        });

        if let Some(desc) = self.description {
            ui.label(
                RichText::new(desc)
                    .size(tokens.font_size_xs)
                    .color(tokens.muted_foreground),
            );
        }

        response.response
    }
}
