//! Scroll area component with custom styling.

use crate::theme::current_theme;
use egui::Ui;

/// Scroll direction
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollDirection {
    /// Vertical scrolling only
    #[default]
    Vertical,
    /// Horizontal scrolling only
    Horizontal,
    /// Both directions
    Both,
}

/// A styled scroll area component
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::ScrollArea;
///
/// ScrollArea::new()
///     .max_height(300.0)
///     .show(ui, |ui| {
///         for i in 0..100 {
///             ui.label(format!("Item {}", i));
///         }
///     });
/// ```
pub struct ScrollArea {
    direction: ScrollDirection,
    max_height: Option<f32>,
    max_width: Option<f32>,
    min_height: Option<f32>,
    min_width: Option<f32>,
    auto_shrink: bool,
    stick_to_bottom: bool,
    stick_to_right: bool,
}

impl ScrollArea {
    /// Create a new scroll area
    pub fn new() -> Self {
        Self {
            direction: ScrollDirection::Vertical,
            max_height: None,
            max_width: None,
            min_height: None,
            min_width: None,
            auto_shrink: true,
            stick_to_bottom: false,
            stick_to_right: false,
        }
    }

    /// Set scroll direction
    pub fn direction(mut self, direction: ScrollDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Vertical scrolling only
    pub fn vertical(mut self) -> Self {
        self.direction = ScrollDirection::Vertical;
        self
    }

    /// Horizontal scrolling only
    pub fn horizontal(mut self) -> Self {
        self.direction = ScrollDirection::Horizontal;
        self
    }

    /// Both directions
    pub fn both(mut self) -> Self {
        self.direction = ScrollDirection::Both;
        self
    }

    /// Set maximum height
    pub fn max_height(mut self, height: f32) -> Self {
        self.max_height = Some(height);
        self
    }

    /// Set maximum width
    pub fn max_width(mut self, width: f32) -> Self {
        self.max_width = Some(width);
        self
    }

    /// Set minimum height
    pub fn min_height(mut self, height: f32) -> Self {
        self.min_height = Some(height);
        self
    }

    /// Set minimum width
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = Some(width);
        self
    }

    /// Disable auto-shrink
    pub fn auto_shrink(mut self, shrink: bool) -> Self {
        self.auto_shrink = shrink;
        self
    }

    /// Stick to bottom (for chat-like UIs)
    pub fn stick_to_bottom(mut self) -> Self {
        self.stick_to_bottom = true;
        self
    }

    /// Stick to right
    pub fn stick_to_right(mut self) -> Self {
        self.stick_to_right = true;
        self
    }

    /// Show the scroll area with content
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let (horizontal, vertical) = match self.direction {
            ScrollDirection::Vertical => (false, true),
            ScrollDirection::Horizontal => (true, false),
            ScrollDirection::Both => (true, true),
        };

        let mut scroll =
            egui::ScrollArea::new([horizontal, vertical]).auto_shrink(self.auto_shrink);

        if let Some(max_h) = self.max_height {
            scroll = scroll.max_height(max_h);
        }
        if let Some(max_w) = self.max_width {
            scroll = scroll.max_width(max_w);
        }
        if let Some(min_h) = self.min_height {
            scroll = scroll.min_scrolled_height(min_h);
        }
        if let Some(min_w) = self.min_width {
            scroll = scroll.min_scrolled_width(min_w);
        }

        if self.stick_to_bottom {
            scroll = scroll.stick_to_bottom(true);
        }
        if self.stick_to_right {
            scroll = scroll.stick_to_right(true);
        }

        scroll.show(ui, add_contents).inner
    }
}

impl Default for ScrollArea {
    fn default() -> Self {
        Self::new()
    }
}
