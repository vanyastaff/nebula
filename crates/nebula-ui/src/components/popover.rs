//! Popover component for floating content.

use crate::theme::current_theme;
use egui::{Response, Ui};

/// Popover placement
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PopoverPlacement {
    /// Above the trigger
    Top,
    /// Below the trigger (default)
    #[default]
    Bottom,
    /// To the left of the trigger
    Left,
    /// To the right of the trigger
    Right,
}

/// A popover component for floating content triggered by click
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::Popover;
///
/// let response = ui.button("Click me");
/// Popover::new("popover_id", &response)
///     .width(200.0)
///     .show(ui, |ui| {
///         ui.label("Popover content");
///     });
/// ```
pub struct Popover<'a> {
    id: &'a str,
    response: &'a Response,
    width: Option<f32>,
    placement: PopoverPlacement,
}

impl<'a> Popover<'a> {
    /// Create a new popover
    pub fn new(id: &'a str, response: &'a Response) -> Self {
        Self {
            id,
            response,
            width: None,
            placement: PopoverPlacement::Bottom,
        }
    }

    /// Set the popover width
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Set the placement
    pub fn placement(mut self, placement: PopoverPlacement) -> Self {
        self.placement = placement;
        self
    }

    /// Show the popover
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) {
        let popup_id = ui.make_persistent_id(self.id);

        if self.response.clicked() {
            ui.memory_mut(|mem| mem.toggle_popup(popup_id));
        }

        let theme = current_theme();
        let tokens = &theme.tokens;

        egui::popup_below_widget(
            ui,
            popup_id,
            self.response,
            egui::PopupCloseBehavior::CloseOnClickOutside,
            |ui| {
                if let Some(width) = self.width {
                    ui.set_min_width(width);
                }

                ui.spacing_mut().item_spacing.y = tokens.spacing_sm;
                add_contents(ui);
            },
        );
    }

    /// Check if the popover is open
    pub fn is_open(&self, ui: &Ui) -> bool {
        let popup_id = ui.make_persistent_id(self.id);
        ui.memory(|mem| mem.is_popup_open(popup_id))
    }

    /// Close the popover
    pub fn close(&self, ui: &mut Ui) {
        let popup_id = ui.make_persistent_id(self.id);
        ui.memory_mut(|mem| mem.close_popup(popup_id));
    }
}
