//! Skeleton loading placeholder component.

use crate::theme::current_theme;
use egui::{Response, Ui, Vec2, Widget};

/// Skeleton shape
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkeletonShape {
    /// Rectangular skeleton
    #[default]
    Rectangle,
    /// Circular skeleton
    Circle,
    /// Text line skeleton
    Text,
}

/// A skeleton loading placeholder
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::Skeleton;
///
/// // Rectangle skeleton
/// ui.add(Skeleton::new().width(200.0).height(20.0));
///
/// // Circle skeleton (avatar placeholder)
/// ui.add(Skeleton::circle(40.0));
///
/// // Text lines
/// ui.add(Skeleton::text());
/// ```
pub struct Skeleton {
    shape: SkeletonShape,
    width: Option<f32>,
    height: f32,
    animated: bool,
}

impl Skeleton {
    /// Create a new skeleton
    pub fn new() -> Self {
        Self {
            shape: SkeletonShape::Rectangle,
            width: None,
            height: 20.0,
            animated: true,
        }
    }

    /// Create a circular skeleton
    pub fn circle(size: f32) -> Self {
        Self {
            shape: SkeletonShape::Circle,
            width: Some(size),
            height: size,
            animated: true,
        }
    }

    /// Create a text line skeleton
    pub fn text() -> Self {
        Self {
            shape: SkeletonShape::Text,
            width: None,
            height: 16.0,
            animated: true,
        }
    }

    /// Set the width
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Set the height
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Disable animation
    pub fn static_skeleton(mut self) -> Self {
        self.animated = false;
        self
    }

    /// Show the skeleton
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl Default for Skeleton {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Skeleton {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let width = self.width.unwrap_or(ui.available_width());
        let size = match self.shape {
            SkeletonShape::Circle => Vec2::splat(self.height),
            _ => Vec2::new(width, self.height),
        };

        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();

            // Base color
            let base_color = tokens.muted;

            // Animation
            let alpha = if self.animated {
                let time = ui.ctx().input(|i| i.time);
                let pulse = (time * 2.0).sin() * 0.3 + 0.7;
                (pulse * 255.0) as u8
            } else {
                255
            };

            let color = egui::Color32::from_rgba_unmultiplied(
                base_color.r(),
                base_color.g(),
                base_color.b(),
                alpha,
            );

            match self.shape {
                SkeletonShape::Circle => {
                    painter.circle_filled(rect.center(), self.height / 2.0, color);
                }
                SkeletonShape::Rectangle => {
                    painter.rect_filled(rect, tokens.rounding_sm(), color);
                }
                SkeletonShape::Text => {
                    painter.rect_filled(rect, tokens.rounding_sm(), color);
                }
            }

            if self.animated {
                ui.ctx().request_repaint();
            }
        }

        response
    }
}

/// A skeleton group for common patterns
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::SkeletonGroup;
///
/// // Card skeleton
/// SkeletonGroup::card().show(ui);
///
/// // List skeleton
/// SkeletonGroup::list(5).show(ui);
/// ```
pub struct SkeletonGroup {
    variant: SkeletonGroupVariant,
    count: usize,
}

enum SkeletonGroupVariant {
    Card,
    List,
    Avatar,
    Paragraph,
}

impl SkeletonGroup {
    /// Create a card skeleton
    pub fn card() -> Self {
        Self {
            variant: SkeletonGroupVariant::Card,
            count: 1,
        }
    }

    /// Create a list skeleton with n items
    pub fn list(count: usize) -> Self {
        Self {
            variant: SkeletonGroupVariant::List,
            count,
        }
    }

    /// Create an avatar with text skeleton
    pub fn avatar() -> Self {
        Self {
            variant: SkeletonGroupVariant::Avatar,
            count: 1,
        }
    }

    /// Create a paragraph skeleton with n lines
    pub fn paragraph(lines: usize) -> Self {
        Self {
            variant: SkeletonGroupVariant::Paragraph,
            count: lines,
        }
    }

    /// Show the skeleton group
    pub fn show(self, ui: &mut Ui) {
        let theme = current_theme();
        let tokens = &theme.tokens;

        match self.variant {
            SkeletonGroupVariant::Card => {
                ui.vertical(|ui| {
                    ui.add(Skeleton::new().height(150.0));
                    ui.add_space(tokens.spacing_sm);
                    ui.add(Skeleton::text().width(ui.available_width() * 0.7));
                    ui.add_space(tokens.spacing_xs);
                    ui.add(Skeleton::text().width(ui.available_width() * 0.5));
                });
            }
            SkeletonGroupVariant::List => {
                ui.vertical(|ui| {
                    for i in 0..self.count {
                        ui.horizontal(|ui| {
                            ui.add(Skeleton::circle(32.0));
                            ui.add_space(tokens.spacing_sm);
                            ui.vertical(|ui| {
                                ui.add(Skeleton::text().width(150.0));
                                ui.add_space(tokens.spacing_xs);
                                ui.add(Skeleton::text().width(100.0).height(12.0));
                            });
                        });
                        if i < self.count - 1 {
                            ui.add_space(tokens.spacing_sm);
                        }
                    }
                });
            }
            SkeletonGroupVariant::Avatar => {
                ui.horizontal(|ui| {
                    ui.add(Skeleton::circle(40.0));
                    ui.add_space(tokens.spacing_sm);
                    ui.vertical(|ui| {
                        ui.add(Skeleton::text().width(120.0));
                        ui.add_space(tokens.spacing_xs);
                        ui.add(Skeleton::text().width(80.0).height(12.0));
                    });
                });
            }
            SkeletonGroupVariant::Paragraph => {
                ui.vertical(|ui| {
                    for i in 0..self.count {
                        let width_factor = if i == self.count - 1 { 0.6 } else { 1.0 };
                        ui.add(Skeleton::text().width(ui.available_width() * width_factor));
                        if i < self.count - 1 {
                            ui.add_space(tokens.spacing_xs);
                        }
                    }
                });
            }
        }
    }
}
