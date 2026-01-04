//! Toolbar component.

use crate::components::{Button, ButtonSize, ButtonVariant, IconButton};
use crate::icons::Icon;
use crate::theme::current_theme;
use egui::{Response, RichText, Ui};

/// A toolbar item
pub enum ToolbarItem<'a> {
    /// A button with text
    Button {
        label: &'a str,
        icon: Option<Icon>,
        variant: ButtonVariant,
        disabled: bool,
    },
    /// An icon-only button
    IconButton {
        icon: Icon,
        tooltip: Option<&'a str>,
        variant: ButtonVariant,
        disabled: bool,
        selected: bool,
    },
    /// A separator
    Separator,
    /// A spacer (flexible space)
    Spacer,
    /// A text label
    Label(&'a str),
    /// Custom widget
    Custom(Box<dyn FnOnce(&mut Ui) -> Response + 'a>),
}

impl<'a> ToolbarItem<'a> {
    /// Create a button item
    pub fn button(label: &'a str) -> Self {
        Self::Button {
            label,
            icon: None,
            variant: ButtonVariant::Ghost,
            disabled: false,
        }
    }

    /// Create a button with icon
    pub fn button_with_icon(label: &'a str, icon: Icon) -> Self {
        Self::Button {
            label,
            icon: Some(icon),
            variant: ButtonVariant::Ghost,
            disabled: false,
        }
    }

    /// Create an icon button
    pub fn icon(icon: Icon) -> Self {
        Self::IconButton {
            icon,
            tooltip: None,
            variant: ButtonVariant::Ghost,
            disabled: false,
            selected: false,
        }
    }

    /// Create an icon button with tooltip
    pub fn icon_with_tooltip(icon: Icon, tooltip: &'a str) -> Self {
        Self::IconButton {
            icon,
            tooltip: Some(tooltip),
            variant: ButtonVariant::Ghost,
            disabled: false,
            selected: false,
        }
    }

    /// Create a separator
    pub fn separator() -> Self {
        Self::Separator
    }

    /// Create a spacer
    pub fn spacer() -> Self {
        Self::Spacer
    }

    /// Create a label
    pub fn label(text: &'a str) -> Self {
        Self::Label(text)
    }

    /// Create a custom widget
    pub fn custom(f: impl FnOnce(&mut Ui) -> Response + 'a) -> Self {
        Self::Custom(Box::new(f))
    }
}

/// A horizontal toolbar
pub struct Toolbar<'a> {
    items: Vec<ToolbarItem<'a>>,
    height: f32,
    bordered: bool,
}

impl<'a> Toolbar<'a> {
    /// Create a new toolbar
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            height: 40.0,
            bordered: true,
        }
    }

    /// Add items
    pub fn items(mut self, items: Vec<ToolbarItem<'a>>) -> Self {
        self.items = items;
        self
    }

    /// Add a single item
    pub fn item(mut self, item: ToolbarItem<'a>) -> Self {
        self.items.push(item);
        self
    }

    /// Set height
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Remove border
    pub fn no_border(mut self) -> Self {
        self.bordered = false;
        self
    }

    /// Show the toolbar and return clicked item indices
    pub fn show(self, ui: &mut Ui) -> Vec<usize> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let mut clicked = Vec::new();

        let frame = if self.bordered {
            egui::Frame::NONE
                .fill(tokens.card)
                .stroke(egui::Stroke::new(1.0, tokens.border))
                .inner_margin(egui::Margin::symmetric(
                    tokens.spacing_sm as i8,
                    tokens.spacing_xs as i8,
                ))
        } else {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(
                tokens.spacing_sm as i8,
                tokens.spacing_xs as i8,
            ))
        };

        frame.show(ui, |ui| {
            ui.set_min_height(self.height);

            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = tokens.spacing_xs;

                for (idx, item) in self.items.into_iter().enumerate() {
                    match item {
                        ToolbarItem::Button {
                            label,
                            icon,
                            variant,
                            disabled,
                        } => {
                            let mut btn = Button::new(label)
                                .variant(variant)
                                .size(ButtonSize::Sm)
                                .disabled(disabled);

                            if let Some(icon) = icon {
                                btn = btn.icon(icon.as_str());
                            }

                            if btn.show(ui).clicked() {
                                clicked.push(idx);
                            }
                        }
                        ToolbarItem::IconButton {
                            icon,
                            tooltip,
                            variant,
                            disabled,
                            selected,
                        } => {
                            let mut btn = IconButton::new(icon.as_str())
                                .variant(variant)
                                .disabled(disabled)
                                .selected(selected);

                            if let Some(tip) = tooltip {
                                btn = btn.tooltip(tip);
                            }

                            if btn.show(ui).clicked() {
                                clicked.push(idx);
                            }
                        }
                        ToolbarItem::Separator => {
                            ui.separator();
                        }
                        ToolbarItem::Spacer => {
                            ui.add_space(ui.available_width());
                        }
                        ToolbarItem::Label(text) => {
                            ui.label(
                                RichText::new(text)
                                    .size(tokens.font_size_sm)
                                    .color(tokens.muted_foreground),
                            );
                        }
                        ToolbarItem::Custom(f) => {
                            if f(ui).clicked() {
                                clicked.push(idx);
                            }
                        }
                    }
                }
            });
        });

        clicked
    }
}

impl Default for Toolbar<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// A toolbar group (buttons grouped together)
pub struct ToolbarGroup<'a> {
    items: Vec<(&'a str, Icon, bool)>, // (tooltip, icon, selected)
    selected_index: Option<usize>,
}

impl<'a> ToolbarGroup<'a> {
    /// Create a new toolbar group
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            selected_index: None,
        }
    }

    /// Add an item
    pub fn item(mut self, tooltip: &'a str, icon: Icon) -> Self {
        self.items.push((tooltip, icon, false));
        self
    }

    /// Set selected index
    pub fn selected(mut self, index: usize) -> Self {
        self.selected_index = Some(index);
        if index < self.items.len() {
            self.items[index].2 = true;
        }
        self
    }

    /// Show the group and return clicked index
    pub fn show(self, ui: &mut Ui) -> Option<usize> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let mut clicked = None;

        egui::Frame::NONE
            .stroke(egui::Stroke::new(1.0, tokens.border))
            .corner_radius(tokens.rounding_md())
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;

                    for (idx, (tooltip, icon, selected)) in self.items.into_iter().enumerate() {
                        let btn = IconButton::new(icon.as_str())
                            .selected(selected)
                            .tooltip(tooltip);

                        if btn.show(ui).clicked() {
                            clicked = Some(idx);
                        }
                    }
                });
            });

        clicked
    }
}

impl Default for ToolbarGroup<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Breadcrumb navigation
pub struct Breadcrumb<'a> {
    items: Vec<&'a str>,
    separator: &'a str,
}

impl<'a> Breadcrumb<'a> {
    /// Create a new breadcrumb
    pub fn new(items: Vec<&'a str>) -> Self {
        Self {
            items,
            separator: "›",
        }
    }

    /// Set custom separator
    pub fn separator(mut self, sep: &'a str) -> Self {
        self.separator = sep;
        self
    }

    /// Show the breadcrumb and return clicked index
    pub fn show(self, ui: &mut Ui) -> Option<usize> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let mut clicked = None;

        ui.horizontal(|ui| {
            for (idx, item) in self.items.iter().enumerate() {
                let is_last = idx == self.items.len() - 1;

                let response = if is_last {
                    ui.label(
                        RichText::new(*item)
                            .size(tokens.font_size_sm)
                            .color(tokens.foreground),
                    )
                } else {
                    let response = ui.add(
                        egui::Label::new(
                            RichText::new(*item)
                                .size(tokens.font_size_sm)
                                .color(tokens.muted_foreground),
                        )
                        .sense(egui::Sense::click()),
                    );

                    if response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }

                    response
                };

                if response.clicked() && !is_last {
                    clicked = Some(idx);
                }

                if !is_last {
                    ui.label(
                        RichText::new(self.separator)
                            .size(tokens.font_size_sm)
                            .color(tokens.muted_foreground),
                    );
                }
            }
        });

        clicked
    }
}
