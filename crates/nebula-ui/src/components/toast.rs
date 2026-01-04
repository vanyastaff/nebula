//! Toast notification system (similar to Sonner).

use crate::theme::current_theme;
use egui::{Color32, Context, Id, Pos2, RichText, Vec2};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Toast variant/type
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastVariant {
    /// Default/info toast
    #[default]
    Default,
    /// Success toast
    Success,
    /// Error toast
    Error,
    /// Warning toast
    Warning,
    /// Info toast
    Info,
    /// Loading toast
    Loading,
}

impl ToastVariant {
    fn icon(&self) -> &'static str {
        match self {
            ToastVariant::Default => "💬",
            ToastVariant::Success => "✓",
            ToastVariant::Error => "✕",
            ToastVariant::Warning => "⚠",
            ToastVariant::Info => "ℹ",
            ToastVariant::Loading => "⏳",
        }
    }

    fn color(&self, tokens: &crate::theme::ThemeTokens) -> Color32 {
        match self {
            ToastVariant::Default => tokens.foreground,
            ToastVariant::Success => Color32::from_rgb(34, 197, 94),
            ToastVariant::Error => tokens.destructive,
            ToastVariant::Warning => Color32::from_rgb(234, 179, 8),
            ToastVariant::Info => Color32::from_rgb(59, 130, 246),
            ToastVariant::Loading => tokens.primary,
        }
    }
}

/// Toast position on screen
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastPosition {
    /// Top left
    TopLeft,
    /// Top center
    TopCenter,
    /// Top right
    TopRight,
    /// Bottom left
    BottomLeft,
    /// Bottom center
    #[default]
    BottomCenter,
    /// Bottom right
    BottomRight,
}

/// A single toast notification
#[derive(Clone, Debug)]
pub struct Toast {
    /// Unique ID
    pub id: u64,
    /// Title/message
    pub message: String,
    /// Optional description
    pub description: Option<String>,
    /// Toast variant
    pub variant: ToastVariant,
    /// Duration before auto-dismiss (None = persistent)
    pub duration: Option<Duration>,
    /// When the toast was created
    pub created_at: Instant,
    /// Whether toast can be dismissed by user
    pub dismissible: bool,
    /// Optional action button
    pub action: Option<ToastAction>,
    /// Whether toast is being dismissed (for animation)
    pub dismissing: bool,
}

/// Toast action button
#[derive(Clone, Debug)]
pub struct ToastAction {
    /// Button label
    pub label: String,
    /// Action ID (returned when clicked)
    pub id: String,
}

impl Toast {
    /// Create a new toast
    pub fn new(message: impl Into<String>) -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        Self {
            id: COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            message: message.into(),
            description: None,
            variant: ToastVariant::Default,
            duration: Some(Duration::from_secs(4)),
            created_at: Instant::now(),
            dismissible: true,
            action: None,
            dismissing: false,
        }
    }

    /// Set description
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set variant
    pub fn variant(mut self, variant: ToastVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Success variant
    pub fn success(mut self) -> Self {
        self.variant = ToastVariant::Success;
        self
    }

    /// Error variant
    pub fn error(mut self) -> Self {
        self.variant = ToastVariant::Error;
        self
    }

    /// Warning variant
    pub fn warning(mut self) -> Self {
        self.variant = ToastVariant::Warning;
        self
    }

    /// Info variant
    pub fn info(mut self) -> Self {
        self.variant = ToastVariant::Info;
        self
    }

    /// Loading variant
    pub fn loading(mut self) -> Self {
        self.variant = ToastVariant::Loading;
        self.duration = None; // Loading toasts don't auto-dismiss
        self
    }

    /// Set duration
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Make persistent (no auto-dismiss)
    pub fn persistent(mut self) -> Self {
        self.duration = None;
        self
    }

    /// Disable dismissing
    pub fn no_dismiss(mut self) -> Self {
        self.dismissible = false;
        self
    }

    /// Add action button
    pub fn action(mut self, label: impl Into<String>, id: impl Into<String>) -> Self {
        self.action = Some(ToastAction {
            label: label.into(),
            id: id.into(),
        });
        self
    }

    /// Check if toast has expired
    pub fn is_expired(&self) -> bool {
        if let Some(duration) = self.duration {
            self.created_at.elapsed() > duration
        } else {
            false
        }
    }
}

/// Toast manager/container
#[derive(Clone)]
pub struct Toaster {
    toasts: Arc<Mutex<VecDeque<Toast>>>,
    position: ToastPosition,
    max_visible: usize,
    gap: f32,
    width: f32,
}

impl Default for Toaster {
    fn default() -> Self {
        Self::new()
    }
}

impl Toaster {
    /// Create a new toaster
    pub fn new() -> Self {
        Self {
            toasts: Arc::new(Mutex::new(VecDeque::new())),
            position: ToastPosition::BottomCenter,
            max_visible: 5,
            gap: 8.0,
            width: 360.0,
        }
    }

    /// Set position
    pub fn position(mut self, position: ToastPosition) -> Self {
        self.position = position;
        self
    }

    /// Set max visible toasts
    pub fn max_visible(mut self, max: usize) -> Self {
        self.max_visible = max;
        self
    }

    /// Set gap between toasts
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Set toast width
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Add a toast
    pub fn toast(&self, toast: Toast) -> u64 {
        let id = toast.id;
        if let Ok(mut toasts) = self.toasts.lock() {
            toasts.push_back(toast);
        }
        id
    }

    /// Show a simple message
    pub fn message(&self, msg: impl Into<String>) -> u64 {
        self.toast(Toast::new(msg))
    }

    /// Show success toast
    pub fn success(&self, msg: impl Into<String>) -> u64 {
        self.toast(Toast::new(msg).success())
    }

    /// Show error toast
    pub fn error(&self, msg: impl Into<String>) -> u64 {
        self.toast(Toast::new(msg).error())
    }

    /// Show warning toast
    pub fn warning(&self, msg: impl Into<String>) -> u64 {
        self.toast(Toast::new(msg).warning())
    }

    /// Show info toast
    pub fn info(&self, msg: impl Into<String>) -> u64 {
        self.toast(Toast::new(msg).info())
    }

    /// Show loading toast (returns ID for later update)
    pub fn loading(&self, msg: impl Into<String>) -> u64 {
        self.toast(Toast::new(msg).loading())
    }

    /// Dismiss a toast by ID
    pub fn dismiss(&self, id: u64) {
        if let Ok(mut toasts) = self.toasts.lock() {
            if let Some(toast) = toasts.iter_mut().find(|t| t.id == id) {
                toast.dismissing = true;
            }
        }
    }

    /// Dismiss all toasts
    pub fn dismiss_all(&self) {
        if let Ok(mut toasts) = self.toasts.lock() {
            for toast in toasts.iter_mut() {
                toast.dismissing = true;
            }
        }
    }

    /// Update a toast (e.g., change loading to success)
    pub fn update(&self, id: u64, f: impl FnOnce(&mut Toast)) {
        if let Ok(mut toasts) = self.toasts.lock() {
            if let Some(toast) = toasts.iter_mut().find(|t| t.id == id) {
                f(toast);
            }
        }
    }

    /// Show the toaster UI
    pub fn show(&self, ctx: &Context) -> Option<ToasterResponse> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let mut response = None;

        // Remove expired and dismissed toasts
        if let Ok(mut toasts) = self.toasts.lock() {
            toasts.retain(|t| !t.is_expired() && !t.dismissing);
        }

        // Get toasts to display
        let toasts_to_show: Vec<Toast> = if let Ok(toasts) = self.toasts.lock() {
            toasts.iter().take(self.max_visible).cloned().collect()
        } else {
            return None;
        };

        if toasts_to_show.is_empty() {
            return None;
        }

        // Calculate position
        let screen_rect = ctx.input(|i| i.screen_rect());
        let margin = 16.0;

        let (anchor_pos, grow_up) = match self.position {
            ToastPosition::TopLeft => (Pos2::new(margin, margin), false),
            ToastPosition::TopCenter => (
                Pos2::new(screen_rect.center().x - self.width / 2.0, margin),
                false,
            ),
            ToastPosition::TopRight => (
                Pos2::new(screen_rect.max.x - self.width - margin, margin),
                false,
            ),
            ToastPosition::BottomLeft => (Pos2::new(margin, screen_rect.max.y - margin), true),
            ToastPosition::BottomCenter => (
                Pos2::new(
                    screen_rect.center().x - self.width / 2.0,
                    screen_rect.max.y - margin,
                ),
                true,
            ),
            ToastPosition::BottomRight => (
                Pos2::new(
                    screen_rect.max.x - self.width - margin,
                    screen_rect.max.y - margin,
                ),
                true,
            ),
        };

        // Show each toast
        let mut y_offset = 0.0;

        for toast in &toasts_to_show {
            let toast_height = self.estimate_toast_height(&toast);

            let toast_pos = if grow_up {
                Pos2::new(anchor_pos.x, anchor_pos.y - y_offset - toast_height)
            } else {
                Pos2::new(anchor_pos.x, anchor_pos.y + y_offset)
            };

            let toast_response = self.show_toast(ctx, toast, toast_pos);

            if let Some(action_id) = toast_response {
                response = Some(ToasterResponse {
                    toast_id: toast.id,
                    action_id,
                });
            }

            y_offset += toast_height + self.gap;
        }

        response
    }

    fn estimate_toast_height(&self, toast: &Toast) -> f32 {
        let base_height = 48.0;
        let desc_height = if toast.description.is_some() {
            20.0
        } else {
            0.0
        };
        let action_height = if toast.action.is_some() { 32.0 } else { 0.0 };
        base_height + desc_height + action_height
    }

    fn show_toast(&self, ctx: &Context, toast: &Toast, pos: Pos2) -> Option<String> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let mut action_clicked = None;
        let toast_id = toast.id;

        egui::Area::new(Id::new(format!("toast_{}", toast.id)))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                let frame = egui::Frame::NONE
                    .fill(tokens.card)
                    .stroke(egui::Stroke::new(1.0, tokens.border))
                    .corner_radius(tokens.rounding_lg())
                    .shadow(egui::Shadow {
                        offset: [0, 4],
                        blur: 12,
                        spread: 0,
                        color: tokens.shadow_color,
                    })
                    .inner_margin(tokens.spacing_md as i8);

                frame.show(ui, |ui| {
                    ui.set_min_width(self.width - tokens.spacing_md * 2.0);
                    ui.set_max_width(self.width - tokens.spacing_md * 2.0);

                    ui.horizontal(|ui| {
                        // Icon
                        let icon_color = toast.variant.color(tokens);
                        ui.label(
                            RichText::new(toast.variant.icon())
                                .size(tokens.font_size_lg)
                                .color(icon_color),
                        );

                        ui.add_space(tokens.spacing_sm);

                        // Content
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(&toast.message)
                                    .size(tokens.font_size_sm)
                                    .color(tokens.foreground)
                                    .strong(),
                            );

                            if let Some(desc) = &toast.description {
                                ui.label(
                                    RichText::new(desc)
                                        .size(tokens.font_size_xs)
                                        .color(tokens.muted_foreground),
                                );
                            }

                            // Action button
                            if let Some(action) = &toast.action {
                                ui.add_space(tokens.spacing_xs);
                                let btn = egui::Button::new(
                                    RichText::new(&action.label)
                                        .size(tokens.font_size_xs)
                                        .color(tokens.primary),
                                )
                                .fill(egui::Color32::TRANSPARENT)
                                .frame(false);

                                if ui.add(btn).clicked() {
                                    action_clicked = Some(action.id.clone());
                                }
                            }
                        });

                        // Dismiss button
                        if toast.dismissible {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                                let close_btn = egui::Button::new(
                                    RichText::new("✕")
                                        .size(tokens.font_size_sm)
                                        .color(tokens.muted_foreground),
                                )
                                .fill(egui::Color32::TRANSPARENT)
                                .frame(false);

                                if ui.add(close_btn).clicked() {
                                    self.dismiss(toast_id);
                                }
                            });
                        }
                    });

                    // Progress bar for timed toasts
                    if let Some(duration) = toast.duration {
                        let elapsed = toast.created_at.elapsed();
                        let progress =
                            1.0 - (elapsed.as_secs_f32() / duration.as_secs_f32()).min(1.0);

                        if progress > 0.0 {
                            ui.add_space(tokens.spacing_xs);
                            let bar_rect = ui.available_rect_before_wrap();
                            let bar_rect = egui::Rect::from_min_size(
                                bar_rect.min,
                                Vec2::new(bar_rect.width(), 2.0),
                            );

                            ui.painter().rect_filled(bar_rect, 1.0, tokens.muted);

                            let filled_rect = egui::Rect::from_min_size(
                                bar_rect.min,
                                Vec2::new(bar_rect.width() * progress, 2.0),
                            );

                            ui.painter()
                                .rect_filled(filled_rect, 1.0, toast.variant.color(tokens));

                            ui.allocate_space(Vec2::new(0.0, 2.0));
                        }
                    }
                });
            });

        // Request repaint for animations
        ctx.request_repaint();

        action_clicked
    }
}

/// Response from toaster when action is clicked
#[derive(Clone, Debug)]
pub struct ToasterResponse {
    /// Toast ID
    pub toast_id: u64,
    /// Action ID that was clicked
    pub action_id: String,
}

/// Promise-style toast for async operations
pub struct ToastPromise<'a> {
    toaster: &'a Toaster,
    loading_msg: String,
    success_msg: String,
    error_msg: String,
}

impl<'a> ToastPromise<'a> {
    /// Create a new toast promise
    pub fn new(toaster: &'a Toaster, loading: impl Into<String>) -> Self {
        Self {
            toaster,
            loading_msg: loading.into(),
            success_msg: "Success!".to_string(),
            error_msg: "Something went wrong".to_string(),
        }
    }

    /// Set success message
    pub fn success(mut self, msg: impl Into<String>) -> Self {
        self.success_msg = msg.into();
        self
    }

    /// Set error message
    pub fn error(mut self, msg: impl Into<String>) -> Self {
        self.error_msg = msg.into();
        self
    }

    /// Start the loading toast
    pub fn start(&self) -> ToastPromiseHandle {
        let id = self.toaster.loading(&self.loading_msg);
        ToastPromiseHandle {
            toaster: self.toaster.clone(),
            id,
            success_msg: self.success_msg.clone(),
            error_msg: self.error_msg.clone(),
        }
    }
}

/// Handle for a promise toast
pub struct ToastPromiseHandle {
    toaster: Toaster,
    id: u64,
    success_msg: String,
    error_msg: String,
}

impl ToastPromiseHandle {
    /// Mark as success
    pub fn success(self) {
        self.toaster.update(self.id, |toast| {
            toast.message = self.success_msg;
            toast.variant = ToastVariant::Success;
            toast.duration = Some(Duration::from_secs(3));
            toast.created_at = Instant::now();
        });
    }

    /// Mark as error
    pub fn error(self) {
        self.toaster.update(self.id, |toast| {
            toast.message = self.error_msg;
            toast.variant = ToastVariant::Error;
            toast.duration = Some(Duration::from_secs(5));
            toast.created_at = Instant::now();
        });
    }

    /// Mark as error with custom message
    pub fn error_with(self, msg: impl Into<String>) {
        let msg = msg.into();
        self.toaster.update(self.id, |toast| {
            toast.message = msg;
            toast.variant = ToastVariant::Error;
            toast.duration = Some(Duration::from_secs(5));
            toast.created_at = Instant::now();
        });
    }

    /// Dismiss the toast
    pub fn dismiss(self) {
        self.toaster.dismiss(self.id);
    }
}
