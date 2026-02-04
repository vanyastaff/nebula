//! Badge component for labels and status indicators.

use crate::theme::current_theme;
use egui::{Color32, Response, RichText, Ui, Vec2, Widget};

/// Badge variant
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BadgeVariant {
    /// Default gray badge
    #[default]
    Default,
    /// Primary color badge
    Primary,
    /// Secondary badge
    Secondary,
    /// Success (green)
    Success,
    /// Warning (yellow)
    Warning,
    /// Destructive (red)
    Destructive,
    /// Info (blue)
    Info,
    /// Outline style
    Outline,
}

/// A badge/tag component
///
/// # Example
///
/// ```rust,ignore
/// Badge::new("New")
///     .success()
///     .show(ui);
/// ```
pub struct Badge<'a> {
    text: &'a str,
    variant: BadgeVariant,
    icon: Option<&'a str>,
    removable: bool,
    size: BadgeSize,
}

/// Badge size
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BadgeSize {
    /// Small badge
    Sm,
    /// Default size
    #[default]
    Md,
    /// Large badge
    Lg,
}

impl<'a> Badge<'a> {
    /// Create a new badge
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            variant: BadgeVariant::Default,
            icon: None,
            removable: false,
            size: BadgeSize::Md,
        }
    }

    /// Set variant
    pub fn variant(mut self, variant: BadgeVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Primary variant
    pub fn primary(mut self) -> Self {
        self.variant = BadgeVariant::Primary;
        self
    }

    /// Secondary variant
    pub fn secondary(mut self) -> Self {
        self.variant = BadgeVariant::Secondary;
        self
    }

    /// Success variant
    pub fn success(mut self) -> Self {
        self.variant = BadgeVariant::Success;
        self
    }

    /// Warning variant
    pub fn warning(mut self) -> Self {
        self.variant = BadgeVariant::Warning;
        self
    }

    /// Destructive variant
    pub fn destructive(mut self) -> Self {
        self.variant = BadgeVariant::Destructive;
        self
    }

    /// Info variant
    pub fn info(mut self) -> Self {
        self.variant = BadgeVariant::Info;
        self
    }

    /// Outline variant
    pub fn outline(mut self) -> Self {
        self.variant = BadgeVariant::Outline;
        self
    }

    /// Add icon
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Make removable (shows X button)
    pub fn removable(mut self) -> Self {
        self.removable = true;
        self
    }

    /// Set size
    pub fn size(mut self, size: BadgeSize) -> Self {
        self.size = size;
        self
    }

    /// Small size
    pub fn small(mut self) -> Self {
        self.size = BadgeSize::Sm;
        self
    }

    /// Large size
    pub fn large(mut self) -> Self {
        self.size = BadgeSize::Lg;
        self
    }

    /// Show the badge, returns (response, removed)
    pub fn show(self, ui: &mut Ui) -> BadgeResponse {
        self.show_inner(ui)
    }

    fn show_inner(self, ui: &mut Ui) -> BadgeResponse {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let (bg, fg, border) = self.colors(&theme);
        let (font_size, padding_h, padding_v) = self.sizing(&theme);

        let mut removed = false;

        let frame = egui::Frame::NONE
            .fill(bg)
            .stroke(egui::Stroke::new(1.0, border))
            .corner_radius(tokens.radius_full)
            .inner_margin(egui::Margin::symmetric(padding_h as i8, padding_v as i8));

        let response = frame
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = tokens.spacing_xs;

                    if let Some(icon) = self.icon {
                        ui.label(RichText::new(icon).size(font_size).color(fg));
                    }

                    ui.label(RichText::new(self.text).size(font_size).color(fg));

                    if self.removable {
                        let x_response = ui.add(
                            egui::Button::new(RichText::new("×").size(font_size).color(fg))
                                .frame(false)
                                .min_size(Vec2::splat(font_size + 2.0)),
                        );
                        if x_response.clicked() {
                            removed = true;
                        }
                    }
                });
            })
            .response;

        BadgeResponse { response, removed }
    }
}

impl<'a> Widget for Badge<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show_inner(ui).response
    }
}

impl<'a> Badge<'a> {
    fn colors(&self, theme: &crate::theme::Theme) -> (Color32, Color32, Color32) {
        let tokens = &theme.tokens;

        // For light theme, use darker text colors for better contrast
        let darken = |color: Color32| -> Color32 {
            if theme.is_dark {
                color
            } else {
                // Darken the color by 30% for light theme
                Color32::from_rgb(
                    (color.r() as f32 * 0.7) as u8,
                    (color.g() as f32 * 0.7) as u8,
                    (color.b() as f32 * 0.7) as u8,
                )
            }
        };

        match self.variant {
            BadgeVariant::Default => (tokens.muted, tokens.muted_foreground, tokens.border),
            BadgeVariant::Primary => (
                with_alpha(tokens.primary, 30),
                darken(tokens.primary),
                tokens.primary,
            ),
            BadgeVariant::Secondary => {
                (tokens.secondary, tokens.secondary_foreground, tokens.border)
            }
            BadgeVariant::Success => (
                with_alpha(tokens.success, 30),
                darken(tokens.success),
                tokens.success,
            ),
            BadgeVariant::Warning => (
                with_alpha(tokens.warning, 30),
                darken(tokens.warning),
                tokens.warning,
            ),
            BadgeVariant::Destructive => (
                with_alpha(tokens.destructive, 30),
                darken(tokens.destructive),
                tokens.destructive,
            ),
            BadgeVariant::Info => (
                with_alpha(tokens.info, 30),
                darken(tokens.info),
                tokens.info,
            ),
            BadgeVariant::Outline => (Color32::TRANSPARENT, tokens.foreground, tokens.border),
        }
    }

    fn sizing(&self, theme: &crate::theme::Theme) -> (f32, f32, f32) {
        let tokens = &theme.tokens;

        match self.size {
            BadgeSize::Sm => (tokens.font_size_xs, tokens.spacing_sm, tokens.spacing_xs),
            BadgeSize::Md => (tokens.font_size_sm, tokens.spacing_md, tokens.spacing_xs),
            BadgeSize::Lg => (tokens.font_size_md, tokens.spacing_md, tokens.spacing_sm),
        }
    }
}

/// Response from showing a badge
pub struct BadgeResponse {
    /// The badge response
    pub response: Response,
    /// Whether the remove button was clicked
    pub removed: bool,
}

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// A group of badges
pub struct BadgeGroup<'a> {
    badges: Vec<&'a str>,
    variant: BadgeVariant,
    removable: bool,
}

impl<'a> BadgeGroup<'a> {
    /// Create a new badge group
    pub fn new(badges: Vec<&'a str>) -> Self {
        Self {
            badges,
            variant: BadgeVariant::Default,
            removable: false,
        }
    }

    /// Set variant for all badges
    pub fn variant(mut self, variant: BadgeVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Make all badges removable
    pub fn removable(mut self) -> Self {
        self.removable = true;
        self
    }

    /// Show all badges, returns indices of removed badges
    pub fn show(self, ui: &mut Ui) -> Vec<usize> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let mut removed = Vec::new();

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(tokens.spacing_xs);

            for (i, text) in self.badges.iter().enumerate() {
                let mut badge = Badge::new(text).variant(self.variant);
                if self.removable {
                    badge = badge.removable();
                }

                let response = badge.show(ui);
                if response.removed {
                    removed.push(i);
                }
            }
        });

        removed
    }
}
