//! Sheet component for slide-out panels.

use crate::theme::current_theme;
use egui::{Context, RichText, Ui, Vec2};

/// Sheet side (where it appears from)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SheetSide {
    /// Appears from the right (default)
    #[default]
    Right,
    /// Appears from the left
    Left,
    /// Appears from the bottom
    Bottom,
    /// Appears from the top
    Top,
}

/// A slide-out sheet panel
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::Sheet;
///
/// Sheet::new("sheet_id", &mut open)
///     .title("Settings")
///     .side(SheetSide::Right)
///     .width(400.0)
///     .show(ctx, |ui| {
///         ui.label("Sheet content");
///     });
/// ```
pub struct Sheet<'a> {
    id: &'a str,
    open: &'a mut bool,
    title: Option<&'a str>,
    side: SheetSide,
    width: f32,
    height: f32,
    closable: bool,
}

impl<'a> Sheet<'a> {
    /// Create a new sheet
    pub fn new(id: &'a str, open: &'a mut bool) -> Self {
        Self {
            id,
            open,
            title: None,
            side: SheetSide::Right,
            width: 400.0,
            height: 300.0,
            closable: true,
        }
    }

    /// Set the title
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// Set the side
    pub fn side(mut self, side: SheetSide) -> Self {
        self.side = side;
        self
    }

    /// Set the width (for left/right sheets)
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Set the height (for top/bottom sheets)
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Set whether the sheet has a close button
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// Show the sheet
    pub fn show<R>(self, ctx: &Context, add_contents: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        if !*self.open {
            return None;
        }

        let theme = current_theme();
        let tokens = &theme.tokens;

        let screen_rect = ctx.screen_rect();

        // Calculate panel rect based on side
        let panel_rect = match self.side {
            SheetSide::Right => egui::Rect::from_min_size(
                egui::Pos2::new(screen_rect.right() - self.width, screen_rect.top()),
                Vec2::new(self.width, screen_rect.height()),
            ),
            SheetSide::Left => egui::Rect::from_min_size(
                screen_rect.min,
                Vec2::new(self.width, screen_rect.height()),
            ),
            SheetSide::Bottom => egui::Rect::from_min_size(
                egui::Pos2::new(screen_rect.left(), screen_rect.bottom() - self.height),
                Vec2::new(screen_rect.width(), self.height),
            ),
            SheetSide::Top => egui::Rect::from_min_size(
                screen_rect.min,
                Vec2::new(screen_rect.width(), self.height),
            ),
        };

        // Overlay backdrop
        let backdrop_response = egui::Area::new(egui::Id::new(format!("{}_backdrop", self.id)))
            .fixed_pos(screen_rect.min)
            .show(ctx, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(screen_rect.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(100));
                response
            });

        // Close on backdrop click
        if backdrop_response.inner.clicked() && self.closable {
            *self.open = false;
            return None;
        }

        // Sheet panel
        let result = egui::Area::new(egui::Id::new(self.id))
            .fixed_pos(panel_rect.min)
            .show(ctx, |ui| {
                let frame = egui::Frame::new()
                    .fill(tokens.background)
                    .stroke(egui::Stroke::new(1.0, tokens.border))
                    .shadow(egui::Shadow {
                        offset: [-4, 0],
                        blur: 16,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(60),
                    });

                frame
                    .show(ui, |ui| {
                        ui.set_min_size(panel_rect.size());
                        ui.set_max_size(panel_rect.size());

                        ui.vertical(|ui| {
                            // Header
                            if self.title.is_some() || self.closable {
                                ui.horizontal(|ui| {
                                    if let Some(title) = self.title {
                                        ui.label(
                                            RichText::new(title)
                                                .size(tokens.font_size_lg)
                                                .strong()
                                                .color(tokens.foreground),
                                        );
                                    }

                                    if self.closable {
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let close_btn = ui.add(
                                                    egui::Button::new(
                                                        RichText::new("✕")
                                                            .size(tokens.font_size_md),
                                                    )
                                                    .fill(egui::Color32::TRANSPARENT)
                                                    .stroke(egui::Stroke::NONE),
                                                );

                                                if close_btn.clicked() {
                                                    *self.open = false;
                                                }

                                                if close_btn.hovered() {
                                                    ui.ctx().set_cursor_icon(
                                                        egui::CursorIcon::PointingHand,
                                                    );
                                                }
                                            },
                                        );
                                    }
                                });
                                ui.add_space(tokens.spacing_md);
                                ui.separator();
                                ui.add_space(tokens.spacing_md);
                            }

                            // Content
                            add_contents(ui)
                        })
                        .inner
                    })
                    .inner
            });

        Some(result.inner)
    }
}
