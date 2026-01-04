//! Select/dropdown component.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Vec2, Widget};

/// An option in a select
#[derive(Clone, Debug)]
pub struct SelectOption<T> {
    /// The value
    pub value: T,
    /// Display label
    pub label: String,
    /// Optional description
    pub description: Option<String>,
    /// Optional icon
    pub icon: Option<String>,
    /// Whether this option is disabled
    pub disabled: bool,
}

impl<T> SelectOption<T> {
    /// Create a new option
    pub fn new(value: T, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
            description: None,
            icon: None,
            disabled: false,
        }
    }

    /// Add description
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Add icon
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set disabled
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

/// A select/dropdown component
///
/// # Example
///
/// ```rust,ignore
/// let mut selected = 0usize;
/// let options = vec![
///     SelectOption::new(0, "Option 1"),
///     SelectOption::new(1, "Option 2"),
///     SelectOption::new(2, "Option 3"),
/// ];
///
/// Select::new(&mut selected, options)
///     .label("Choose an option")
///     .show(ui);
/// ```
pub struct Select<'a, T: PartialEq + Clone> {
    selected: &'a mut T,
    options: Vec<SelectOption<T>>,
    label: Option<&'a str>,
    placeholder: &'a str,
    hint: Option<&'a str>,
    error: Option<&'a str>,
    disabled: bool,
    searchable: bool,
    full_width: bool,
    min_width: f32,
}

impl<'a, T: PartialEq + Clone> Select<'a, T> {
    /// Create a new select
    pub fn new(selected: &'a mut T, options: Vec<SelectOption<T>>) -> Self {
        Self {
            selected,
            options,
            label: None,
            placeholder: "Select...",
            hint: None,
            error: None,
            disabled: false,
            searchable: false,
            full_width: false,
            min_width: 180.0,
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

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Enable search filtering
    pub fn searchable(mut self) -> Self {
        self.searchable = true;
        self
    }

    /// Make full width
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// Set minimum width
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }

    /// Show the select
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a, T: PartialEq + Clone> Widget for Select<'a, T> {
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

            // Find currently selected option
            let current_label = self
                .options
                .iter()
                .find(|o| o.value == *self.selected)
                .map(|o| o.label.as_str())
                .unwrap_or(self.placeholder);

            let width = if self.full_width {
                ui.available_width()
            } else {
                self.min_width
            };

            let _border_color = if has_error {
                tokens.destructive
            } else {
                tokens.border
            };

            // ComboBox
            let response = egui::ComboBox::from_id_salt(ui.id().with("select"))
                .selected_text(current_label)
                .width(width)
                .show_ui(ui, |ui| {
                    for option in &self.options {
                        let is_selected = option.value == *self.selected;

                        let response = ui.add_enabled(
                            !option.disabled,
                            egui::SelectableLabel::new(is_selected, &option.label),
                        );

                        if response.clicked() {
                            *self.selected = option.value.clone();
                        }

                        // Show description if present
                        if let Some(desc) = &option.description {
                            ui.label(
                                RichText::new(desc)
                                    .size(tokens.font_size_xs)
                                    .color(tokens.muted_foreground),
                            );
                        }
                    }
                });

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

            response.response
        })
        .inner
    }
}

/// A simple string select helper
pub fn string_select<'a>(ui: &mut Ui, selected: &mut String, options: &[&str]) -> Response {
    let opts: Vec<SelectOption<String>> = options
        .iter()
        .map(|s| SelectOption::new(s.to_string(), *s))
        .collect();

    Select::new(selected, opts).show(ui)
}

/// Multi-select component
pub struct MultiSelect<'a, T: PartialEq + Clone> {
    selected: &'a mut Vec<T>,
    options: Vec<SelectOption<T>>,
    label: Option<&'a str>,
    placeholder: &'a str,
    max_selections: Option<usize>,
    disabled: bool,
    full_width: bool,
}

impl<'a, T: PartialEq + Clone + std::fmt::Debug> MultiSelect<'a, T> {
    /// Create a new multi-select
    pub fn new(selected: &'a mut Vec<T>, options: Vec<SelectOption<T>>) -> Self {
        Self {
            selected,
            options,
            label: None,
            placeholder: "Select items...",
            max_selections: None,
            disabled: false,
            full_width: false,
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

    /// Set maximum selections
    pub fn max(mut self, max: usize) -> Self {
        self.max_selections = Some(max);
        self
    }

    /// Set disabled
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Make full width
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// Show the multi-select
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a, T: PartialEq + Clone + std::fmt::Debug> Widget for MultiSelect<'a, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

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

            // Selected items display
            let selected_labels: Vec<&str> = self
                .options
                .iter()
                .filter(|o| self.selected.contains(&o.value))
                .map(|o| o.label.as_str())
                .collect();

            let display_text = if selected_labels.is_empty() {
                self.placeholder.to_string()
            } else {
                format!("{} selected", selected_labels.len())
            };

            // Dropdown
            egui::ComboBox::from_id_salt(ui.id().with("multi_select"))
                .selected_text(&display_text)
                .show_ui(ui, |ui| {
                    for option in &self.options {
                        let is_selected = self.selected.contains(&option.value);

                        let can_select = is_selected
                            || self
                                .max_selections
                                .map_or(true, |max| self.selected.len() < max);

                        let mut checked = is_selected;
                        let response = ui.add_enabled(
                            !option.disabled && can_select,
                            egui::Checkbox::new(&mut checked, &option.label),
                        );

                        if response.changed() {
                            if checked {
                                if !self.selected.contains(&option.value) {
                                    self.selected.push(option.value.clone());
                                }
                            } else {
                                self.selected.retain(|v| v != &option.value);
                            }
                        }
                    }
                });

            // Show selected as badges
            if !selected_labels.is_empty() {
                ui.add_space(tokens.spacing_xs);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(tokens.spacing_xs);

                    for option in self.options.iter() {
                        if self.selected.contains(&option.value) {
                            let badge = crate::components::Badge::new(&option.label)
                                .small()
                                .removable();

                            if badge.show(ui).removed {
                                self.selected.retain(|v| v != &option.value);
                            }
                        }
                    }
                });
            }
        })
        .response
    }
}
