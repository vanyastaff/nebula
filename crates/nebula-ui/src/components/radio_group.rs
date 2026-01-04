//! Radio group component.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Vec2, Widget};

/// Radio button option
#[derive(Clone, Debug)]
pub struct RadioOption<T> {
    /// Value
    pub value: T,
    /// Display label
    pub label: String,
    /// Optional description
    pub description: Option<String>,
    /// Whether disabled
    pub disabled: bool,
}

impl<T> RadioOption<T> {
    /// Create a new option
    pub fn new(value: T, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
            description: None,
            disabled: false,
        }
    }

    /// Add description
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set disabled
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

/// Radio group orientation
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RadioOrientation {
    /// Vertical layout (default)
    #[default]
    Vertical,
    /// Horizontal layout
    Horizontal,
}

/// Radio group component
///
/// # Example
///
/// ```rust,ignore
/// #[derive(PartialEq, Clone)]
/// enum Size { Small, Medium, Large }
///
/// let mut size = Size::Medium;
/// let options = vec![
///     RadioOption::new(Size::Small, "Small"),
///     RadioOption::new(Size::Medium, "Medium"),
///     RadioOption::new(Size::Large, "Large"),
/// ];
///
/// RadioGroup::new(&mut size, options).show(ui);
/// ```
pub struct RadioGroup<'a, T: PartialEq + Clone> {
    selected: &'a mut T,
    options: Vec<RadioOption<T>>,
    label: Option<&'a str>,
    orientation: RadioOrientation,
    disabled: bool,
    card_style: bool,
}

impl<'a, T: PartialEq + Clone> RadioGroup<'a, T> {
    /// Create a new radio group
    pub fn new(selected: &'a mut T, options: Vec<RadioOption<T>>) -> Self {
        Self {
            selected,
            options,
            label: None,
            orientation: RadioOrientation::Vertical,
            disabled: false,
            card_style: false,
        }
    }

    /// Set label
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Set horizontal orientation
    pub fn horizontal(mut self) -> Self {
        self.orientation = RadioOrientation::Horizontal;
        self
    }

    /// Set orientation
    pub fn orientation(mut self, orientation: RadioOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Disable all options
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Use card style (options as clickable cards)
    pub fn card_style(mut self) -> Self {
        self.card_style = true;
        self
    }

    /// Show the radio group
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a, T: PartialEq + Clone> Widget for RadioGroup<'a, T> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        ui.vertical(|ui| {
            // Label
            if let Some(label) = self.label {
                ui.label(
                    RichText::new(label)
                        .size(tokens.font_size_sm)
                        .color(tokens.foreground),
                );
                ui.add_space(tokens.spacing_xs);
            }

            // Options
            match self.orientation {
                RadioOrientation::Vertical => {
                    for option in &self.options {
                        let is_disabled = self.disabled || option.disabled;
                        let is_selected = *self.selected == option.value;

                        if self.card_style {
                            if show_card_option(
                                ui,
                                &option.label,
                                option.description.as_deref(),
                                is_selected,
                                is_disabled,
                            ) {
                                *self.selected = option.value.clone();
                            }
                        } else {
                            if show_radio_option(
                                ui,
                                &option.label,
                                option.description.as_deref(),
                                is_selected,
                                is_disabled,
                            ) {
                                *self.selected = option.value.clone();
                            }
                        }
                    }
                }
                RadioOrientation::Horizontal => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = tokens.spacing_md;
                        for option in &self.options {
                            let is_disabled = self.disabled || option.disabled;
                            let is_selected = *self.selected == option.value;

                            if self.card_style {
                                if show_card_option(
                                    ui,
                                    &option.label,
                                    option.description.as_deref(),
                                    is_selected,
                                    is_disabled,
                                ) {
                                    *self.selected = option.value.clone();
                                }
                            } else {
                                if show_radio_option(
                                    ui,
                                    &option.label,
                                    option.description.as_deref(),
                                    is_selected,
                                    is_disabled,
                                ) {
                                    *self.selected = option.value.clone();
                                }
                            }
                        }
                    });
                }
            }
        })
        .response
    }
}

/// Show a radio option, returns true if clicked
fn show_radio_option(
    ui: &mut Ui,
    label: &str,
    description: Option<&str>,
    is_selected: bool,
    disabled: bool,
) -> bool {
    let theme = current_theme();
    let tokens = &theme.tokens;

    let mut clicked = false;

    ui.horizontal(|ui| {
        // Custom radio button
        let size = 18.0;
        let (rect, response) = ui.allocate_exact_size(
            Vec2::splat(size),
            if disabled {
                egui::Sense::hover()
            } else {
                egui::Sense::click()
            },
        );

        // Outer circle
        let outer_color = if is_selected {
            tokens.primary
        } else {
            tokens.border
        };

        ui.painter().circle_stroke(
            rect.center(),
            size / 2.0 - 1.0,
            egui::Stroke::new(2.0, outer_color),
        );

        // Inner circle (when selected)
        if is_selected {
            ui.painter()
                .circle_filled(rect.center(), size / 2.0 - 5.0, tokens.primary);
        }

        if response.clicked() && !disabled {
            clicked = true;
        }

        // Label
        ui.add_space(tokens.spacing_xs);

        ui.vertical(|ui| {
            let text_color = if disabled {
                tokens.muted_foreground
            } else {
                tokens.foreground
            };

            ui.label(
                RichText::new(label)
                    .size(tokens.font_size_sm)
                    .color(text_color),
            );

            if let Some(desc) = description {
                ui.label(
                    RichText::new(desc)
                        .size(tokens.font_size_xs)
                        .color(tokens.muted_foreground),
                );
            }
        });
    });

    clicked
}

/// Show a card-style option, returns true if clicked
fn show_card_option(
    ui: &mut Ui,
    label: &str,
    description: Option<&str>,
    is_selected: bool,
    disabled: bool,
) -> bool {
    let theme = current_theme();
    let tokens = &theme.tokens;

    let border_color = if is_selected {
        tokens.primary
    } else {
        tokens.border
    };

    let bg_color = if is_selected {
        tokens.accent
    } else {
        tokens.card
    };

    let frame = egui::Frame::NONE
        .fill(bg_color)
        .stroke(egui::Stroke::new(
            if is_selected { 2.0 } else { 1.0 },
            border_color,
        ))
        .corner_radius(tokens.rounding_lg())
        .inner_margin(tokens.spacing_md as i8);

    let frame_response = frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            // Radio indicator
            let size = 18.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());

            let outer_color = if is_selected {
                tokens.primary
            } else {
                tokens.border
            };

            ui.painter().circle_stroke(
                rect.center(),
                size / 2.0 - 1.0,
                egui::Stroke::new(2.0, outer_color),
            );

            if is_selected {
                ui.painter()
                    .circle_filled(rect.center(), size / 2.0 - 5.0, tokens.primary);
            }

            ui.add_space(tokens.spacing_sm);

            // Content
            ui.vertical(|ui| {
                let text_color = if disabled {
                    tokens.muted_foreground
                } else {
                    tokens.foreground
                };

                ui.label(
                    RichText::new(label)
                        .size(tokens.font_size_sm)
                        .color(text_color)
                        .strong(),
                );

                if let Some(desc) = description {
                    ui.label(
                        RichText::new(desc)
                            .size(tokens.font_size_xs)
                            .color(tokens.muted_foreground),
                    );
                }
            });
        });
    });

    let interact_response = ui.interact(
        frame_response.response.rect,
        ui.id().with(label),
        if disabled {
            egui::Sense::hover()
        } else {
            egui::Sense::click()
        },
    );

    if interact_response.hovered() && !disabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    interact_response.clicked() && !disabled
}

/// Single radio button (for custom layouts)
pub struct Radio<'a, T: PartialEq + Clone> {
    selected: &'a mut T,
    value: T,
    label: &'a str,
    description: Option<&'a str>,
    disabled: bool,
}

impl<'a, T: PartialEq + Clone> Radio<'a, T> {
    /// Create a new radio button
    pub fn new(selected: &'a mut T, value: T, label: &'a str) -> Self {
        Self {
            selected,
            value,
            label,
            description: None,
            disabled: false,
        }
    }

    /// Add description
    pub fn description(mut self, desc: &'a str) -> Self {
        self.description = Some(desc);
        self
    }

    /// Set disabled
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Show the radio button
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a, T: PartialEq + Clone> Widget for Radio<'a, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let is_selected = *self.selected == self.value;

        let response = ui.horizontal(|ui| {
            // Radio circle
            let size = 18.0;
            let (rect, btn_response) = ui.allocate_exact_size(
                Vec2::splat(size),
                if self.disabled {
                    egui::Sense::hover()
                } else {
                    egui::Sense::click()
                },
            );

            let outer_color = if is_selected {
                tokens.primary
            } else {
                tokens.border
            };

            ui.painter().circle_stroke(
                rect.center(),
                size / 2.0 - 1.0,
                egui::Stroke::new(2.0, outer_color),
            );

            if is_selected {
                ui.painter()
                    .circle_filled(rect.center(), size / 2.0 - 5.0, tokens.primary);
            }

            if btn_response.clicked() && !self.disabled {
                *self.selected = self.value.clone();
            }

            ui.add_space(tokens.spacing_xs);

            // Label
            ui.vertical(|ui| {
                let text_color = if self.disabled {
                    tokens.muted_foreground
                } else {
                    tokens.foreground
                };

                let label_response = ui.add(
                    egui::Label::new(
                        RichText::new(self.label)
                            .size(tokens.font_size_sm)
                            .color(text_color),
                    )
                    .sense(if self.disabled {
                        egui::Sense::hover()
                    } else {
                        egui::Sense::click()
                    }),
                );

                if label_response.clicked() && !self.disabled {
                    *self.selected = self.value.clone();
                }

                if let Some(desc) = self.description {
                    ui.label(
                        RichText::new(desc)
                            .size(tokens.font_size_xs)
                            .color(tokens.muted_foreground),
                    );
                }
            });

            btn_response
        });

        response.inner
    }
}
