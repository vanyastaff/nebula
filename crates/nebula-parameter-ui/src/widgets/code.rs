//! Code editor widget for CodeParameter.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{FontFamily, FontId, Frame, Rounding, Stroke, TextEdit, Ui};
use nebula_parameter::core::{HasValue, Parameter};
use nebula_parameter::types::{CodeLanguage, CodeParameter};

/// Widget for code input with syntax highlighting info.
pub struct CodeWidget {
    parameter: CodeParameter,
    /// Internal buffer for code editing
    buffer: String,
}

impl ParameterWidget for CodeWidget {
    type Parameter = CodeParameter;

    fn new(parameter: Self::Parameter) -> Self {
        let buffer = parameter.get().map(|t| t.to_string()).unwrap_or_default();
        Self { parameter, buffer }
    }

    fn parameter(&self) -> &Self::Parameter {
        &self.parameter
    }

    fn parameter_mut(&mut self) -> &mut Self::Parameter {
        &mut self.parameter
    }

    fn show(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> WidgetResponse {
        let mut response = WidgetResponse::default();

        let metadata = self.parameter.metadata();
        let name = metadata.name.clone();
        let required = metadata.required;
        let hint = metadata.hint.clone();
        let placeholder = metadata
            .placeholder
            .clone()
            .or_else(|| Some(metadata.description.clone()))
            .filter(|s| !s.is_empty())
            .unwrap_or_default();

        let language = self.parameter.get_language();
        let lang_name = language_display_name(&language);

        // Header with language badge
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&name).color(theme.label_color));
            if required {
                ui.label(egui::RichText::new("*").color(theme.error));
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Language badge - simple text
                ui.label(
                    egui::RichText::new(lang_name)
                        .small()
                        .color(theme.hint_color)
                        .family(FontFamily::Monospace),
                );
            });
        });

        ui.add_space(2.0);

        let min_lines = self
            .parameter
            .options
            .as_ref()
            .and_then(|o| o.min_lines)
            .unwrap_or(6) as usize;

        let is_readonly = self.parameter.is_readonly();
        let show_line_numbers = self
            .parameter
            .options
            .as_ref()
            .is_some_and(|o| o.line_numbers);

        // Code editor - with line numbers it needs a frame, otherwise flat
        if show_line_numbers {
            Frame::none()
                .fill(theme.input_bg)
                .stroke(Stroke::new(1.0, theme.input_border))
                .rounding(Rounding::same(theme.border_radius as u8))
                .inner_margin(egui::Margin::same(0))
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        // Line numbers column
                        let line_count = self.buffer.lines().count().max(min_lines);

                        Frame::none()
                            .fill(theme.surface)
                            .inner_margin(egui::Margin::symmetric(6, 6))
                            .show(ui, |ui| {
                                let line_numbers: String =
                                    (1..=line_count).map(|n| format!("{:3}\n", n)).collect();

                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&line_numbers)
                                            .font(FontId::new(12.0, FontFamily::Monospace))
                                            .color(theme.hint_color),
                                    )
                                    .selectable(false),
                                );
                            });

                        // Separator
                        let separator_rect = ui.available_rect_before_wrap();
                        ui.painter().vline(
                            separator_rect.left(),
                            separator_rect.y_range(),
                            Stroke::new(1.0, theme.input_border),
                        );

                        // Code area
                        Frame::none()
                            .inner_margin(egui::Margin::same(6))
                            .show(ui, |ui| {
                                let text_edit = TextEdit::multiline(&mut self.buffer)
                                    .font(FontId::new(12.0, FontFamily::Monospace))
                                    .hint_text(&placeholder)
                                    .desired_rows(min_lines)
                                    .desired_width(f32::INFINITY)
                                    .lock_focus(true)
                                    .code_editor();

                                let edit_response = if is_readonly {
                                    ui.add(text_edit.interactive(false))
                                } else {
                                    ui.add(text_edit)
                                };

                                if edit_response.changed() {
                                    if let Err(e) = self
                                        .parameter
                                        .set(nebula_value::Text::from(self.buffer.as_str()))
                                    {
                                        response.error = Some(e.to_string());
                                    } else {
                                        response.changed = true;
                                    }
                                }

                                if edit_response.lost_focus() {
                                    response.lost_focus = true;
                                }
                            });
                    });
                });
        } else {
            // Simple code editor without line numbers - flat
            let text_edit = TextEdit::multiline(&mut self.buffer)
                .font(FontId::new(12.0, FontFamily::Monospace))
                .hint_text(&placeholder)
                .desired_rows(min_lines)
                .desired_width(f32::INFINITY)
                .lock_focus(true)
                .code_editor();

            let edit_response = if is_readonly {
                ui.add(text_edit.interactive(false))
            } else {
                ui.add(text_edit)
            };

            if edit_response.changed() {
                if let Err(e) = self
                    .parameter
                    .set(nebula_value::Text::from(self.buffer.as_str()))
                {
                    response.error = Some(e.to_string());
                } else {
                    response.changed = true;
                }
            }

            if edit_response.lost_focus() {
                response.lost_focus = true;
            }
        }

        // Status bar - simple text
        ui.horizontal(|ui| {
            let line_count = self.buffer.lines().count();
            let char_count = self.buffer.len();
            ui.label(
                egui::RichText::new(format!("{} lines, {} chars", line_count, char_count))
                    .small()
                    .color(theme.hint_color),
            );

            if is_readonly {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("Read Only")
                            .small()
                            .color(theme.warning),
                    );
                });
            }
        });

        // Hint (help text below field)
        if let Some(hint_text) = hint {
            if !hint_text.is_empty() {
                ui.label(
                    egui::RichText::new(&hint_text)
                        .small()
                        .color(theme.hint_color),
                );
            }
        }

        // Error
        if let Some(ref error) = response.error {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(error).small().color(theme.error));
        }

        response
    }
}

impl CodeWidget {
    /// Get the current code value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.buffer
    }

    /// Set the code value directly.
    pub fn set_value(&mut self, value: &str) {
        self.buffer = value.to_string();
        let _ = self.parameter.set(nebula_value::Text::from(value));
    }

    /// Get the programming language.
    #[must_use]
    pub fn language(&self) -> CodeLanguage {
        self.parameter.get_language()
    }

    /// Get the line count.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.buffer.lines().count()
    }

    /// Check if the editor is read-only.
    #[must_use]
    pub fn is_readonly(&self) -> bool {
        self.parameter.is_readonly()
    }
}

/// Get display name for a code language.
fn language_display_name(lang: &CodeLanguage) -> &'static str {
    match lang {
        CodeLanguage::JavaScript => "JavaScript",
        CodeLanguage::TypeScript => "TypeScript",
        CodeLanguage::Python => "Python",
        CodeLanguage::Rust => "Rust",
        CodeLanguage::Go => "Go",
        CodeLanguage::Java => "Java",
        CodeLanguage::C => "C",
        CodeLanguage::Cpp => "C++",
        CodeLanguage::CSharp => "C#",
        CodeLanguage::Php => "PHP",
        CodeLanguage::Ruby => "Ruby",
        CodeLanguage::Shell => "Shell",
        CodeLanguage::Sql => "SQL",
        CodeLanguage::Json => "JSON",
        CodeLanguage::Yaml => "YAML",
        CodeLanguage::Xml => "XML",
        CodeLanguage::Html => "HTML",
        CodeLanguage::Css => "CSS",
        CodeLanguage::Markdown => "Markdown",
        CodeLanguage::PlainText => "Plain Text",
    }
}
