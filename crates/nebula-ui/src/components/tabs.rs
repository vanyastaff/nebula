//! Tabs component for tabbed interfaces.

use crate::theme::current_theme;
use egui::{Response, Ui, Vec2, Widget};

/// Tab item
#[derive(Clone)]
pub struct Tab<'a> {
    /// Tab label
    pub label: &'a str,
    /// Optional icon
    pub icon: Option<&'a str>,
    /// Optional badge count
    pub badge: Option<usize>,
    /// Disabled state
    pub disabled: bool,
}

impl<'a> Tab<'a> {
    /// Create a new tab
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            icon: None,
            badge: None,
            disabled: false,
        }
    }

    /// Add an icon
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Add a badge
    pub fn badge(mut self, count: usize) -> Self {
        self.badge = Some(count);
        self
    }

    /// Set disabled
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

/// Tabs variant
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabsVariant {
    /// Default tabs with underline indicator
    #[default]
    Default,
    /// Boxed/pill style tabs
    Boxed,
    /// Outline style tabs
    Outline,
}

/// A tabs component for switching between content
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::{Tabs, Tab};
///
/// Tabs::new(&mut selected)
///     .tab(Tab::new("Overview").icon("📊"))
///     .tab(Tab::new("Settings").icon("⚙"))
///     .tab(Tab::new("Users").badge(5))
///     .show(ui);
/// ```
pub struct Tabs<'a> {
    selected: &'a mut usize,
    tabs: Vec<Tab<'a>>,
    variant: TabsVariant,
    full_width: bool,
}

impl<'a> Tabs<'a> {
    /// Create new tabs
    pub fn new(selected: &'a mut usize) -> Self {
        Self {
            selected,
            tabs: Vec::new(),
            variant: TabsVariant::Default,
            full_width: false,
        }
    }

    /// Add a tab
    pub fn tab(mut self, tab: Tab<'a>) -> Self {
        self.tabs.push(tab);
        self
    }

    /// Add multiple tabs from labels
    pub fn tabs(mut self, labels: &[&'a str]) -> Self {
        for label in labels {
            self.tabs.push(Tab::new(label));
        }
        self
    }

    /// Set the variant
    pub fn variant(mut self, variant: TabsVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Make tabs full width
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// Show the tabs
    pub fn show(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let response = ui.horizontal(|ui| {
            if self.full_width {
                ui.spacing_mut().item_spacing.x = 0.0;
            }

            let tab_count = self.tabs.len();

            for (index, tab) in self.tabs.into_iter().enumerate() {
                let is_selected = *self.selected == index;
                let is_disabled = tab.disabled;

                let tab_width = if self.full_width {
                    Some(ui.available_width() / tab_count as f32)
                } else {
                    None
                };

                // Calculate colors based on variant and state
                let (bg, fg, border) = match self.variant {
                    TabsVariant::Default => {
                        if is_selected {
                            (
                                egui::Color32::TRANSPARENT,
                                tokens.foreground,
                                tokens.primary,
                            )
                        } else {
                            (
                                egui::Color32::TRANSPARENT,
                                tokens.muted_foreground,
                                egui::Color32::TRANSPARENT,
                            )
                        }
                    }
                    TabsVariant::Boxed => {
                        if is_selected {
                            (tokens.primary, tokens.primary_foreground, tokens.primary)
                        } else {
                            (tokens.muted, tokens.muted_foreground, tokens.muted)
                        }
                    }
                    TabsVariant::Outline => {
                        if is_selected {
                            (tokens.background, tokens.foreground, tokens.border)
                        } else {
                            (
                                egui::Color32::TRANSPARENT,
                                tokens.muted_foreground,
                                egui::Color32::TRANSPARENT,
                            )
                        }
                    }
                };

                let height = 36.0;
                let min_width = tab_width.unwrap_or(60.0);

                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(min_width, height),
                    if is_disabled {
                        egui::Sense::hover()
                    } else {
                        egui::Sense::click()
                    },
                );

                if response.clicked() && !is_disabled {
                    *self.selected = index;
                }

                if ui.is_rect_visible(rect) {
                    let painter = ui.painter();

                    // Background
                    match self.variant {
                        TabsVariant::Default => {
                            // Underline only
                            if is_selected {
                                let underline_rect = egui::Rect::from_min_size(
                                    egui::Pos2::new(rect.left(), rect.bottom() - 2.0),
                                    Vec2::new(rect.width(), 2.0),
                                );
                                painter.rect_filled(underline_rect, 0.0, tokens.primary);
                            }
                        }
                        TabsVariant::Boxed => {
                            painter.rect_filled(rect, tokens.rounding_sm(), bg);
                        }
                        TabsVariant::Outline => {
                            if is_selected {
                                painter.rect_stroke(
                                    rect,
                                    tokens.rounding_sm(),
                                    egui::Stroke::new(1.0, border),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        }
                    }

                    // Content
                    let mut content_x = rect.center().x;
                    let content_y = rect.center().y;

                    // Calculate total content width
                    let icon_width = if tab.icon.is_some() {
                        tokens.font_size_sm + 4.0
                    } else {
                        0.0
                    };
                    let badge_width = if tab.badge.is_some() { 20.0 } else { 0.0 };

                    // Icon
                    if let Some(icon) = tab.icon {
                        painter.text(
                            egui::Pos2::new(content_x - 20.0, content_y),
                            egui::Align2::CENTER_CENTER,
                            icon,
                            egui::FontId::proportional(tokens.font_size_sm),
                            fg,
                        );
                    }

                    // Label
                    painter.text(
                        egui::Pos2::new(content_x, content_y),
                        egui::Align2::CENTER_CENTER,
                        tab.label,
                        egui::FontId::proportional(tokens.font_size_sm),
                        fg,
                    );

                    // Badge
                    if let Some(count) = tab.badge {
                        let badge_x = content_x + 30.0;
                        let badge_rect = egui::Rect::from_center_size(
                            egui::Pos2::new(badge_x, content_y),
                            Vec2::new(18.0, 16.0),
                        );
                        painter.rect_filled(badge_rect, 8.0, tokens.primary);
                        painter.text(
                            badge_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            count.to_string(),
                            egui::FontId::proportional(tokens.font_size_xs),
                            tokens.primary_foreground,
                        );
                    }
                }

                if response.hovered() && !is_disabled {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }
        });

        response.response
    }
}

impl<'a> Widget for Tabs<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}
