//! Avatar component for user/entity representation.

use crate::theme::current_theme;
use egui::{Color32, Response, Ui, Vec2, Widget};

/// Avatar size
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AvatarSize {
    /// Extra small (20px)
    Xs,
    /// Small (24px)
    Sm,
    /// Medium (32px) - default
    #[default]
    Md,
    /// Large (40px)
    Lg,
    /// Extra large (48px)
    Xl,
}

impl AvatarSize {
    fn pixels(&self) -> f32 {
        match self {
            Self::Xs => 20.0,
            Self::Sm => 24.0,
            Self::Md => 32.0,
            Self::Lg => 40.0,
            Self::Xl => 48.0,
        }
    }

    fn font_size(&self) -> f32 {
        match self {
            Self::Xs => 10.0,
            Self::Sm => 11.0,
            Self::Md => 13.0,
            Self::Lg => 16.0,
            Self::Xl => 18.0,
        }
    }
}

/// Avatar shape
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AvatarShape {
    /// Circular avatar
    #[default]
    Circle,
    /// Rounded square
    Rounded,
    /// Square with slight rounding
    Square,
}

/// An avatar component for displaying user/entity images or initials
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::Avatar;
///
/// // With initials
/// ui.add(Avatar::new().initials("JD").size(AvatarSize::Lg));
///
/// // With fallback icon
/// ui.add(Avatar::new().fallback_icon("👤"));
/// ```
pub struct Avatar<'a> {
    initials: Option<&'a str>,
    fallback_icon: Option<&'a str>,
    size: AvatarSize,
    shape: AvatarShape,
    color: Option<Color32>,
    border: bool,
    status: Option<AvatarStatus>,
}

/// Online status indicator
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvatarStatus {
    /// Online (green)
    Online,
    /// Away/Idle (yellow)
    Away,
    /// Busy/Do not disturb (red)
    Busy,
    /// Offline (gray)
    Offline,
}

impl<'a> Avatar<'a> {
    /// Create a new avatar
    pub fn new() -> Self {
        Self {
            initials: None,
            fallback_icon: None,
            size: AvatarSize::Md,
            shape: AvatarShape::Circle,
            color: None,
            border: false,
            status: None,
        }
    }

    /// Set initials to display
    pub fn initials(mut self, initials: &'a str) -> Self {
        self.initials = Some(initials);
        self
    }

    /// Set a fallback icon
    pub fn fallback_icon(mut self, icon: &'a str) -> Self {
        self.fallback_icon = Some(icon);
        self
    }

    /// Set the size
    pub fn size(mut self, size: AvatarSize) -> Self {
        self.size = size;
        self
    }

    /// Set the shape
    pub fn shape(mut self, shape: AvatarShape) -> Self {
        self.shape = shape;
        self
    }

    /// Set a custom background color
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    /// Add a border
    pub fn border(mut self) -> Self {
        self.border = true;
        self
    }

    /// Add a status indicator
    pub fn status(mut self, status: AvatarStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// Show the avatar
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl Default for Avatar<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Widget for Avatar<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let size = self.size.pixels();
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();

            // Background color
            let bg_color = self.color.unwrap_or(tokens.muted);

            // Shape rounding
            let rounding = match self.shape {
                AvatarShape::Circle => size / 2.0,
                AvatarShape::Rounded => tokens.radius_md,
                AvatarShape::Square => tokens.radius_sm,
            };

            // Draw background
            painter.rect_filled(rect, rounding, bg_color);

            // Draw border if enabled
            if self.border {
                painter.rect_stroke(
                    rect,
                    rounding,
                    egui::Stroke::new(2.0, tokens.border),
                    egui::StrokeKind::Inside,
                );
            }

            // Draw content (initials or icon)
            let content = self.initials.or(self.fallback_icon).unwrap_or("?");

            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                content,
                egui::FontId::proportional(self.size.font_size()),
                tokens.foreground,
            );

            // Draw status indicator
            if let Some(status) = self.status {
                let status_color = match status {
                    AvatarStatus::Online => tokens.success,
                    AvatarStatus::Away => tokens.warning,
                    AvatarStatus::Busy => tokens.destructive,
                    AvatarStatus::Offline => tokens.muted_foreground,
                };

                let status_size = size * 0.25;
                let status_pos = egui::Pos2::new(
                    rect.right() - status_size / 2.0,
                    rect.bottom() - status_size / 2.0,
                );

                // White border around status
                painter.circle_filled(status_pos, status_size / 2.0 + 1.5, tokens.background);
                painter.circle_filled(status_pos, status_size / 2.0, status_color);
            }
        }

        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        response
    }
}
