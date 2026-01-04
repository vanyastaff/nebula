//! Carousel/slider component for cycling through content.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Vec2};

/// Carousel orientation
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CarouselOrientation {
    /// Horizontal carousel (default)
    #[default]
    Horizontal,
    /// Vertical carousel
    Vertical,
}

/// Carousel component for cycling through items
///
/// # Example
///
/// ```rust,ignore
/// let mut index = 0;
/// Carousel::new(&mut index, 5)
///     .show(ui, |ui, idx| {
///         ui.label(format!("Slide {}", idx + 1));
///     });
/// ```
pub struct Carousel<'a> {
    current: &'a mut usize,
    total: usize,
    orientation: CarouselOrientation,
    show_arrows: bool,
    show_dots: bool,
    auto_play: bool,
    loop_items: bool,
    item_width: Option<f32>,
    item_height: Option<f32>,
    gap: Option<f32>,
}

impl<'a> Carousel<'a> {
    /// Create a new carousel
    pub fn new(current: &'a mut usize, total: usize) -> Self {
        Self {
            current,
            total,
            orientation: CarouselOrientation::Horizontal,
            show_arrows: true,
            show_dots: true,
            auto_play: false,
            loop_items: true,
            item_width: None,
            item_height: None,
            gap: None,
        }
    }

    /// Set orientation
    pub fn orientation(mut self, orientation: CarouselOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Vertical orientation
    pub fn vertical(mut self) -> Self {
        self.orientation = CarouselOrientation::Vertical;
        self
    }

    /// Hide navigation arrows
    pub fn hide_arrows(mut self) -> Self {
        self.show_arrows = false;
        self
    }

    /// Hide dot indicators
    pub fn hide_dots(mut self) -> Self {
        self.show_dots = false;
        self
    }

    /// Enable auto-play
    pub fn auto_play(mut self) -> Self {
        self.auto_play = true;
        self
    }

    /// Disable looping
    pub fn no_loop(mut self) -> Self {
        self.loop_items = false;
        self
    }

    /// Set item width
    pub fn item_width(mut self, width: f32) -> Self {
        self.item_width = Some(width);
        self
    }

    /// Set item height
    pub fn item_height(mut self, height: f32) -> Self {
        self.item_height = Some(height);
        self
    }

    /// Set gap between items
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = Some(gap);
        self
    }

    /// Navigate to previous item
    fn prev(&mut self) {
        if self.total == 0 {
            return;
        }
        if *self.current > 0 {
            *self.current -= 1;
        } else if self.loop_items {
            *self.current = self.total - 1;
        }
    }

    /// Navigate to next item
    fn next(&mut self) {
        if self.total == 0 {
            return;
        }
        if *self.current < self.total - 1 {
            *self.current += 1;
        } else if self.loop_items {
            *self.current = 0;
        }
    }

    /// Show the carousel with content
    pub fn show<R>(
        mut self,
        ui: &mut Ui,
        add_content: impl FnOnce(&mut Ui, usize) -> R,
    ) -> CarouselResponse<R> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let gap = self.gap.unwrap_or(tokens.spacing_md);
        let mut changed = false;
        let mut inner = None;

        let is_horizontal = self.orientation == CarouselOrientation::Horizontal;

        if is_horizontal {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    // Previous button
                    if self.show_arrows {
                        let can_prev = self.loop_items || *self.current > 0;
                        if ui
                            .add_enabled(
                                can_prev,
                                egui::Button::new("◀").min_size(Vec2::splat(32.0)),
                            )
                            .clicked()
                        {
                            self.prev();
                            changed = true;
                        }
                    }

                    // Content area
                    let content_width = self.item_width.unwrap_or(ui.available_width() - 80.0);
                    let content_height = self.item_height.unwrap_or(200.0);

                    let frame = egui::Frame::NONE
                        .fill(tokens.card)
                        .stroke(egui::Stroke::new(1.0, tokens.border))
                        .corner_radius(tokens.rounding_lg())
                        .inner_margin(tokens.spacing_md as i8);

                    frame.show(ui, |ui| {
                        ui.set_min_size(Vec2::new(content_width, content_height));
                        inner = Some(add_content(ui, *self.current));
                    });

                    // Next button
                    if self.show_arrows {
                        let can_next = self.loop_items || *self.current < self.total - 1;
                        if ui
                            .add_enabled(
                                can_next,
                                egui::Button::new("▶").min_size(Vec2::splat(32.0)),
                            )
                            .clicked()
                        {
                            self.next();
                            changed = true;
                        }
                    }
                });

                // Dot indicators
                if self.show_dots && self.total > 0 {
                    ui.add_space(tokens.spacing_sm);
                    ui.horizontal(|ui| {
                        ui.with_layout(
                            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = tokens.spacing_xs;
                                    for i in 0..self.total {
                                        let is_active = i == *self.current;
                                        let size = if is_active { 10.0 } else { 8.0 };
                                        let color = if is_active {
                                            tokens.primary
                                        } else {
                                            tokens.muted
                                        };

                                        let (rect, response) = ui.allocate_exact_size(
                                            Vec2::splat(size),
                                            egui::Sense::click(),
                                        );

                                        ui.painter().circle_filled(
                                            rect.center(),
                                            size / 2.0,
                                            color,
                                        );

                                        if response.clicked() {
                                            *self.current = i;
                                            changed = true;
                                        }
                                    }
                                });
                            },
                        );
                    });
                }
            });
        } else {
            // Vertical carousel
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    // Previous button (up)
                    if self.show_arrows {
                        let can_prev = self.loop_items || *self.current > 0;
                        if ui
                            .add_enabled(
                                can_prev,
                                egui::Button::new("▲").min_size(Vec2::splat(32.0)),
                            )
                            .clicked()
                        {
                            self.prev();
                            changed = true;
                        }
                    }

                    // Content area
                    let content_width = self.item_width.unwrap_or(200.0);
                    let content_height = self.item_height.unwrap_or(ui.available_height() - 80.0);

                    let frame = egui::Frame::NONE
                        .fill(tokens.card)
                        .stroke(egui::Stroke::new(1.0, tokens.border))
                        .corner_radius(tokens.rounding_lg())
                        .inner_margin(tokens.spacing_md as i8);

                    frame.show(ui, |ui| {
                        ui.set_min_size(Vec2::new(content_width, content_height));
                        inner = Some(add_content(ui, *self.current));
                    });

                    // Next button (down)
                    if self.show_arrows {
                        let can_next = self.loop_items || *self.current < self.total - 1;
                        if ui
                            .add_enabled(
                                can_next,
                                egui::Button::new("▼").min_size(Vec2::splat(32.0)),
                            )
                            .clicked()
                        {
                            self.next();
                            changed = true;
                        }
                    }
                });

                // Dot indicators (vertical)
                if self.show_dots && self.total > 0 {
                    ui.add_space(tokens.spacing_sm);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = tokens.spacing_xs;
                        for i in 0..self.total {
                            let is_active = i == *self.current;
                            let size = if is_active { 10.0 } else { 8.0 };
                            let color = if is_active {
                                tokens.primary
                            } else {
                                tokens.muted
                            };

                            let (rect, response) =
                                ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());

                            ui.painter().circle_filled(rect.center(), size / 2.0, color);

                            if response.clicked() {
                                *self.current = i;
                                changed = true;
                            }
                        }
                    });
                }
            });
        }

        CarouselResponse {
            changed,
            current: *self.current,
            inner: inner.unwrap(),
        }
    }
}

/// Response from carousel
pub struct CarouselResponse<R> {
    /// Whether the index changed
    pub changed: bool,
    /// Current index
    pub current: usize,
    /// Inner content response
    pub inner: R,
}

/// Image carousel helper
pub struct ImageCarousel<'a> {
    images: &'a [&'a str],
    current: &'a mut usize,
    fit: ImageFit,
}

/// How to fit images in the carousel
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageFit {
    /// Contain image within bounds
    #[default]
    Contain,
    /// Cover the entire area
    Cover,
    /// Fill exactly
    Fill,
}

impl<'a> ImageCarousel<'a> {
    /// Create a new image carousel
    pub fn new(images: &'a [&'a str], current: &'a mut usize) -> Self {
        Self {
            images,
            current,
            fit: ImageFit::Contain,
        }
    }

    /// Set image fit mode
    pub fn fit(mut self, fit: ImageFit) -> Self {
        self.fit = fit;
        self
    }

    /// Show the image carousel
    pub fn show(self, ui: &mut Ui) -> CarouselResponse<()> {
        let total = self.images.len();

        Carousel::new(self.current, total).show(ui, |ui, idx| {
            if idx < self.images.len() {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new(format!("🖼 {}", self.images[idx]))
                            .size(14.0)
                            .color(ui.style().visuals.text_color()),
                    );
                });
            }
        })
    }
}
