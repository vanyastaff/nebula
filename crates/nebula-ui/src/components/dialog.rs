//! Dialog and modal components.

use crate::components::{Button, ButtonVariant};
use crate::theme::current_theme;
use egui::{Align2, Color32, Context, Id, RichText, Vec2};

/// Response from a dialog
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogResponse {
    /// Dialog is still open
    Open,
    /// User confirmed/submitted
    Confirmed,
    /// User cancelled/dismissed
    Cancelled,
}

impl DialogResponse {
    /// Check if confirmed
    pub fn confirmed(&self) -> bool {
        matches!(self, Self::Confirmed)
    }

    /// Check if cancelled
    pub fn cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Check if still open
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open)
    }
}

/// A modal dialog
///
/// # Example
///
/// ```rust,ignore
/// Dialog::new("Settings", &mut open)
///     .width(400.0)
///     .show(ctx, |ui| {
///         ui.label("Dialog content");
///     });
/// ```
pub struct Dialog<'a> {
    title: &'a str,
    open: &'a mut bool,
    width: f32,
    max_height: Option<f32>,
    closable: bool,
    show_backdrop: bool,
    id: Option<Id>,
}

impl<'a> Dialog<'a> {
    /// Create a new dialog
    pub fn new(title: &'a str, open: &'a mut bool) -> Self {
        Self {
            title,
            open,
            width: 400.0,
            max_height: None,
            closable: true,
            show_backdrop: true,
            id: None,
        }
    }

    /// Set dialog width
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Set maximum height (enables scrolling)
    pub fn max_height(mut self, height: f32) -> Self {
        self.max_height = Some(height);
        self
    }

    /// Set whether dialog can be closed with X button
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// Set whether to show backdrop
    pub fn backdrop(mut self, show: bool) -> Self {
        self.show_backdrop = show;
        self
    }

    /// Set custom ID
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// Show the dialog
    pub fn show<R>(
        self,
        ctx: &Context,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> Option<R> {
        if !*self.open {
            return None;
        }

        let theme = current_theme();
        let tokens = &theme.tokens;

        // Backdrop
        if self.show_backdrop {
            egui::Area::new(Id::new("dialog_backdrop"))
                .fixed_pos(egui::Pos2::ZERO)
                .order(egui::Order::Background)
                .interactable(true)
                .show(ctx, |ui| {
                    let screen = ui.ctx().screen_rect();

                    // Dark overlay
                    ui.painter()
                        .rect_filled(screen, 0.0, Color32::from_black_alpha(150));

                    // Close on backdrop click (if closable)
                    if self.closable {
                        let response = ui.allocate_rect(screen, egui::Sense::click());
                        if response.clicked() {
                            *self.open = false;
                        }
                    }
                });
        }

        // Dialog window
        let id = self.id.unwrap_or_else(|| Id::new(self.title));
        let mut result = None;

        egui::Window::new(self.title)
            .id(id)
            .open(self.open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(Vec2::new(self.width, 0.0))
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(tokens.card)
                    .stroke(egui::Stroke::new(1.0, tokens.border))
                    .corner_radius(tokens.rounding_lg())
                    .inner_margin(tokens.spacing_lg as i8)
                    .shadow(egui::Shadow {
                        offset: [0, 4],
                        blur: tokens.shadow_lg as u8,
                        spread: 0,
                        color: tokens.shadow_color,
                    }),
            )
            .show(ctx, |ui| {
                if let Some(max_h) = self.max_height {
                    egui::ScrollArea::vertical()
                        .max_height(max_h)
                        .show(ui, |ui| {
                            result = Some(add_contents(ui));
                        });
                } else {
                    result = Some(add_contents(ui));
                }
            });

        result
    }
}

/// Alert dialog with confirm/cancel buttons
///
/// # Example
///
/// ```rust,ignore
/// let response = AlertDialog::new("Delete Item", &mut open)
///     .description("Are you sure you want to delete this item?")
///     .destructive()
///     .show(ctx);
///
/// if response.confirmed() {
///     delete_item();
/// }
/// ```
pub struct AlertDialog<'a> {
    title: &'a str,
    open: &'a mut bool,
    description: Option<&'a str>,
    confirm_text: &'a str,
    cancel_text: &'a str,
    variant: ButtonVariant,
    width: f32,
    icon: Option<&'a str>,
}

impl<'a> AlertDialog<'a> {
    /// Create a new alert dialog
    pub fn new(title: &'a str, open: &'a mut bool) -> Self {
        Self {
            title,
            open,
            description: None,
            confirm_text: "Confirm",
            cancel_text: "Cancel",
            variant: ButtonVariant::Primary,
            width: 350.0,
            icon: None,
        }
    }

    /// Set description text
    pub fn description(mut self, desc: &'a str) -> Self {
        self.description = Some(desc);
        self
    }

    /// Set confirm button text
    pub fn confirm_text(mut self, text: &'a str) -> Self {
        self.confirm_text = text;
        self
    }

    /// Set cancel button text
    pub fn cancel_text(mut self, text: &'a str) -> Self {
        self.cancel_text = text;
        self
    }

    /// Make this a destructive action (red confirm button)
    pub fn destructive(mut self) -> Self {
        self.variant = ButtonVariant::Destructive;
        self
    }

    /// Set dialog width
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Set icon
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Show the alert dialog
    pub fn show(self, ctx: &Context) -> DialogResponse {
        if !*self.open {
            return DialogResponse::Cancelled;
        }

        let theme = current_theme();
        let tokens = &theme.tokens;

        let mut response = DialogResponse::Open;
        let mut should_close = false;

        // Extract values to avoid borrow issues
        let title = self.title;
        let width = self.width;
        let icon = self.icon;
        let description = self.description;
        let confirm_text = self.confirm_text;
        let cancel_text = self.cancel_text;
        let variant = self.variant;

        // Use a dummy bool for the dialog since we manage closing ourselves
        let mut dummy_open = true;

        Dialog::new(title, &mut dummy_open)
            .width(width)
            .closable(false)
            .show(ctx, |ui| {
                // Icon and description
                if let Some(icon) = icon {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon).size(24.0));
                        ui.add_space(tokens.spacing_md);
                        if let Some(desc) = description {
                            ui.label(desc);
                        }
                    });
                } else if let Some(desc) = description {
                    ui.label(desc);
                }

                ui.add_space(tokens.spacing_lg);

                // Buttons
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Confirm button
                        if Button::new(confirm_text)
                            .variant(variant)
                            .show(ui)
                            .clicked()
                        {
                            response = DialogResponse::Confirmed;
                            should_close = true;
                        }

                        ui.add_space(tokens.spacing_sm);

                        // Cancel button
                        if Button::new(cancel_text).outline().show(ui).clicked() {
                            response = DialogResponse::Cancelled;
                            should_close = true;
                        }
                    });
                });
            });

        if should_close {
            *self.open = false;
        }

        response
    }
}

/// Confirmation dialog that returns a boolean
pub struct ConfirmDialog<'a> {
    alert: AlertDialog<'a>,
}

impl<'a> ConfirmDialog<'a> {
    /// Create a new confirm dialog
    pub fn new(title: &'a str, open: &'a mut bool) -> Self {
        Self {
            alert: AlertDialog::new(title, open)
                .confirm_text("Yes")
                .cancel_text("No"),
        }
    }

    /// Set the question/description
    pub fn question(mut self, question: &'a str) -> Self {
        self.alert = self.alert.description(question);
        self
    }

    /// Show the dialog
    pub fn show(self, ctx: &Context) -> Option<bool> {
        match self.alert.show(ctx) {
            DialogResponse::Open => None,
            DialogResponse::Confirmed => Some(true),
            DialogResponse::Cancelled => Some(false),
        }
    }
}

/// Input dialog for getting user input
pub struct InputDialog<'a> {
    title: &'a str,
    open: &'a mut bool,
    value: &'a mut String,
    label: Option<&'a str>,
    placeholder: &'a str,
    validate: Option<Box<dyn Fn(&str) -> Option<&'static str> + 'a>>,
    width: f32,
}

impl<'a> InputDialog<'a> {
    /// Create a new input dialog
    pub fn new(title: &'a str, open: &'a mut bool, value: &'a mut String) -> Self {
        Self {
            title,
            open,
            value,
            label: None,
            placeholder: "",
            validate: None,
            width: 350.0,
        }
    }

    /// Set input label
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Set placeholder text
    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// Set validation function
    pub fn validate(mut self, f: impl Fn(&str) -> Option<&'static str> + 'a) -> Self {
        self.validate = Some(Box::new(f));
        self
    }

    /// Set dialog width
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Show the dialog
    pub fn show(self, ctx: &Context) -> DialogResponse {
        if !*self.open {
            return DialogResponse::Cancelled;
        }

        let theme = current_theme();
        let tokens = &theme.tokens;

        let mut response = DialogResponse::Open;
        let mut should_close = false;

        // Validate current value
        let error = self.validate.as_ref().and_then(|f| f(self.value));
        let can_submit = error.is_none() && !self.value.is_empty();

        // Extract values to avoid borrow issues
        let title = self.title;
        let width = self.width;
        let placeholder = self.placeholder;
        let label = self.label;

        let mut dummy_open = true;

        Dialog::new(title, &mut dummy_open)
            .width(width)
            .show(ctx, |ui| {
                // Input field
                let mut input = crate::components::TextInput::new(self.value)
                    .placeholder(placeholder)
                    .full_width();

                if let Some(label) = label {
                    input = input.label(label);
                }

                if let Some(err) = error {
                    input = input.error(err);
                }

                input.show(ui);

                ui.add_space(tokens.spacing_lg);

                // Buttons
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if Button::new("OK")
                            .primary()
                            .disabled(!can_submit)
                            .show(ui)
                            .clicked()
                        {
                            response = DialogResponse::Confirmed;
                            should_close = true;
                        }

                        ui.add_space(tokens.spacing_sm);

                        if Button::new("Cancel").outline().show(ui).clicked() {
                            response = DialogResponse::Cancelled;
                            should_close = true;
                        }
                    });
                });
            });

        if should_close {
            *self.open = false;
        }

        response
    }
}
