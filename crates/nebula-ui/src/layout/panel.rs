//! Panel component for content areas.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui};

/// Panel position
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelPosition {
    /// Left side panel
    Left,
    /// Right side panel
    Right,
    /// Top panel
    Top,
    /// Bottom panel
    Bottom,
    /// Center (no special positioning)
    #[default]
    Center,
}

/// A panel component with optional header
pub struct Panel<'a> {
    title: Option<&'a str>,
    position: PanelPosition,
    resizable: bool,
    collapsible: bool,
    collapsed: Option<&'a mut bool>,
    default_width: f32,
    default_height: f32,
    min_size: f32,
    max_size: Option<f32>,
    show_separator: bool,
    actions: Option<Box<dyn FnOnce(&mut Ui) + 'a>>,
}

impl<'a> Panel<'a> {
    /// Create a new panel
    pub fn new() -> Self {
        Self {
            title: None,
            position: PanelPosition::Center,
            resizable: false,
            collapsible: false,
            collapsed: None,
            default_width: 250.0,
            default_height: 200.0,
            min_size: 100.0,
            max_size: None,
            show_separator: true,
            actions: None,
        }
    }

    /// Set title
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// Set position
    pub fn position(mut self, position: PanelPosition) -> Self {
        self.position = position;
        self
    }

    /// Left panel
    pub fn left(mut self) -> Self {
        self.position = PanelPosition::Left;
        self
    }

    /// Right panel
    pub fn right(mut self) -> Self {
        self.position = PanelPosition::Right;
        self
    }

    /// Top panel
    pub fn top(mut self) -> Self {
        self.position = PanelPosition::Top;
        self
    }

    /// Bottom panel
    pub fn bottom(mut self) -> Self {
        self.position = PanelPosition::Bottom;
        self
    }

    /// Make resizable
    pub fn resizable(mut self) -> Self {
        self.resizable = true;
        self
    }

    /// Make collapsible
    pub fn collapsible(mut self, collapsed: &'a mut bool) -> Self {
        self.collapsible = true;
        self.collapsed = Some(collapsed);
        self
    }

    /// Set default width (for left/right panels)
    pub fn default_width(mut self, width: f32) -> Self {
        self.default_width = width;
        self
    }

    /// Set default height (for top/bottom panels)
    pub fn default_height(mut self, height: f32) -> Self {
        self.default_height = height;
        self
    }

    /// Set minimum size
    pub fn min_size(mut self, size: f32) -> Self {
        self.min_size = size;
        self
    }

    /// Set maximum size
    pub fn max_size(mut self, size: f32) -> Self {
        self.max_size = Some(size);
        self
    }

    /// Hide separator line
    pub fn no_separator(mut self) -> Self {
        self.show_separator = false;
        self
    }

    /// Add header actions
    pub fn actions(mut self, actions: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.actions = Some(Box::new(actions));
        self
    }

    /// Show the panel
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> PanelResponse<R> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let is_collapsed = self.collapsed.as_ref().map_or(false, |c| **c);

        let frame = egui::Frame::NONE
            .fill(tokens.card)
            .inner_margin(egui::Margin::same(tokens.spacing_md as i8));

        let inner = frame.show(ui, |ui| {
            // Header
            if self.title.is_some() || self.collapsible || self.actions.is_some() {
                ui.horizontal(|ui| {
                    // Collapse button
                    if self.collapsible {
                        if let Some(collapsed) = self.collapsed {
                            let icon = if *collapsed { "▶" } else { "▼" };
                            if ui.small_button(icon).clicked() {
                                *collapsed = !*collapsed;
                            }
                        }
                    }

                    // Title
                    if let Some(title) = self.title {
                        ui.label(
                            RichText::new(title)
                                .size(tokens.font_size_md)
                                .color(tokens.foreground)
                                .strong(),
                        );
                    }

                    // Actions
                    if !is_collapsed {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if let Some(actions) = self.actions {
                                actions(ui);
                            }
                        });
                    }
                });

                if !is_collapsed {
                    ui.add_space(tokens.spacing_sm);
                    if self.show_separator {
                        ui.separator();
                        ui.add_space(tokens.spacing_sm);
                    }
                }
            }

            // Content
            if !is_collapsed {
                add_contents(ui)
            } else {
                // Return default when collapsed
                // This is a bit awkward but necessary for the return type
                unsafe { std::mem::zeroed() }
            }
        });

        PanelResponse {
            inner: if is_collapsed {
                None
            } else {
                Some(inner.inner)
            },
            response: inner.response,
            collapsed: is_collapsed,
        }
    }
}

impl Default for Panel<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Response from showing a panel
pub struct PanelResponse<R> {
    /// The inner content's return value (None if collapsed)
    pub inner: Option<R>,
    /// The panel's response
    pub response: Response,
    /// Whether the panel is collapsed
    pub collapsed: bool,
}

/// Side panel helper that uses egui's built-in panels
pub fn side_panel_left<R>(
    ctx: &egui::Context,
    id: impl Into<egui::Id>,
    default_width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let theme = current_theme();
    let tokens = &theme.tokens;

    let mut result = None;

    egui::SidePanel::left(id)
        .default_width(default_width)
        .frame(
            egui::Frame::NONE
                .fill(tokens.card)
                .stroke(egui::Stroke::new(1.0, tokens.border))
                .inner_margin(egui::Margin::same(tokens.spacing_md as i8)),
        )
        .show(ctx, |ui| {
            result = Some(add_contents(ui));
        });

    result
}

/// Side panel helper that uses egui's built-in panels
pub fn side_panel_right<R>(
    ctx: &egui::Context,
    id: impl Into<egui::Id>,
    default_width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let theme = current_theme();
    let tokens = &theme.tokens;

    let mut result = None;

    egui::SidePanel::right(id)
        .default_width(default_width)
        .frame(
            egui::Frame::NONE
                .fill(tokens.card)
                .stroke(egui::Stroke::new(1.0, tokens.border))
                .inner_margin(egui::Margin::same(tokens.spacing_md as i8)),
        )
        .show(ctx, |ui| {
            result = Some(add_contents(ui));
        });

    result
}

/// Top panel helper
pub fn top_panel<R>(
    ctx: &egui::Context,
    id: impl Into<egui::Id>,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let theme = current_theme();
    let tokens = &theme.tokens;

    let mut result = None;

    egui::TopBottomPanel::top(id)
        .frame(
            egui::Frame::NONE
                .fill(tokens.card)
                .stroke(egui::Stroke::new(1.0, tokens.border))
                .inner_margin(egui::Margin::symmetric(
                    tokens.spacing_md as i8,
                    tokens.spacing_sm as i8,
                )),
        )
        .show(ctx, |ui| {
            result = Some(add_contents(ui));
        });

    result
}

/// Bottom panel helper
pub fn bottom_panel<R>(
    ctx: &egui::Context,
    id: impl Into<egui::Id>,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let theme = current_theme();
    let tokens = &theme.tokens;

    let mut result = None;

    egui::TopBottomPanel::bottom(id)
        .frame(
            egui::Frame::NONE
                .fill(tokens.card)
                .stroke(egui::Stroke::new(1.0, tokens.border))
                .inner_margin(egui::Margin::symmetric(
                    tokens.spacing_md as i8,
                    tokens.spacing_sm as i8,
                )),
        )
        .show(ctx, |ui| {
            result = Some(add_contents(ui));
        });

    result
}
