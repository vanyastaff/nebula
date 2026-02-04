//! Icon system for nebula-ui.
//!
//! Uses Unicode symbols and custom SVG-like icons for common UI elements.

use egui::{Color32, RichText, Ui, Vec2};

/// Icon identifier
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    // Navigation
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    ChevronDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,

    // Actions
    Plus,
    Minus,
    Close,
    Check,
    Search,
    Settings,
    Edit,
    Delete,
    Copy,
    Paste,
    Undo,
    Redo,
    Save,
    Download,
    Upload,
    Refresh,

    // Flow
    Play,
    Pause,
    Stop,

    // Objects
    File,
    Folder,
    FolderOpen,
    Home,
    User,
    Users,
    Lock,
    Unlock,
    Link,
    Unlink,

    // Status
    Info,
    Warning,
    Error,
    Success,
    Question,

    // Misc
    Menu,
    MoreHorizontal,
    MoreVertical,
    Grid,
    List,
    Eye,
    EyeOff,
    Filter,
    Sort,

    // Node-specific
    Execution,
    Variable,
    Input,
    Output,
}

impl Icon {
    /// Get the Unicode character for this icon
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            // Navigation
            Icon::ChevronLeft => "‹",
            Icon::ChevronRight => "›",
            Icon::ChevronUp => "ˆ",
            Icon::ChevronDown => "ˇ",
            Icon::ArrowLeft => "←",
            Icon::ArrowRight => "→",
            Icon::ArrowUp => "↑",
            Icon::ArrowDown => "↓",

            // Actions
            Icon::Plus => "+",
            Icon::Minus => "−",
            Icon::Close => "✕",
            Icon::Check => "✓",
            Icon::Search => "⌕",
            Icon::Settings => "⚙",
            Icon::Edit => "✎",
            Icon::Delete => "🗑",
            Icon::Copy => "⎘",
            Icon::Paste => "📋",
            Icon::Undo => "↶",
            Icon::Redo => "↷",
            Icon::Save => "💾",
            Icon::Download => "⬇",
            Icon::Upload => "⬆",
            Icon::Refresh => "⟳",

            // Flow
            Icon::Play => "▶",
            Icon::Pause => "⏸",
            Icon::Stop => "⏹",

            // Objects
            Icon::File => "📄",
            Icon::Folder => "📁",
            Icon::FolderOpen => "📂",
            Icon::Home => "🏠",
            Icon::User => "👤",
            Icon::Users => "👥",
            Icon::Lock => "🔒",
            Icon::Unlock => "🔓",
            Icon::Link => "🔗",
            Icon::Unlink => "⛓",

            // Status
            Icon::Info => "ℹ",
            Icon::Warning => "⚠",
            Icon::Error => "✖",
            Icon::Success => "✔",
            Icon::Question => "?",

            // Misc
            Icon::Menu => "☰",
            Icon::MoreHorizontal => "⋯",
            Icon::MoreVertical => "⋮",
            Icon::Grid => "⊞",
            Icon::List => "☰",
            Icon::Eye => "👁",
            Icon::EyeOff => "🙈",
            Icon::Filter => "⧩",
            Icon::Sort => "⇅",

            // Node-specific
            Icon::Execution => "▷",
            Icon::Variable => "𝑥",
            Icon::Input => "→",
            Icon::Output => "←",
        }
    }

    /// Display the icon
    pub fn show(self, ui: &mut Ui) -> egui::Response {
        ui.label(self.as_str())
    }

    /// Display the icon with size
    pub fn show_sized(self, ui: &mut Ui, size: f32) -> egui::Response {
        ui.label(RichText::new(self.as_str()).size(size))
    }

    /// Display the icon with color
    pub fn show_colored(self, ui: &mut Ui, color: Color32) -> egui::Response {
        ui.label(RichText::new(self.as_str()).color(color))
    }

    /// Display the icon with size and color
    pub fn show_with(self, ui: &mut Ui, size: f32, color: Color32) -> egui::Response {
        ui.label(RichText::new(self.as_str()).size(size).color(color))
    }

    /// Get as RichText for custom styling
    pub fn rich_text(self) -> RichText {
        RichText::new(self.as_str())
    }
}

impl std::fmt::Display for Icon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Custom painted icon (for more complex icons)
pub struct PaintedIcon {
    size: f32,
    color: Color32,
}

impl PaintedIcon {
    /// Create a new painted icon
    pub fn new(size: f32, color: Color32) -> Self {
        Self { size, color }
    }

    /// Draw an execution arrow (for flow connections)
    pub fn execution_arrow(self, ui: &mut Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), egui::Sense::hover());

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let center = rect.center();
            let half = self.size / 2.0;

            // Draw arrow pointing right
            let points = vec![
                egui::Pos2::new(center.x - half * 0.5, center.y - half * 0.6),
                egui::Pos2::new(center.x + half * 0.5, center.y),
                egui::Pos2::new(center.x - half * 0.5, center.y + half * 0.6),
            ];

            painter.add(egui::Shape::convex_polygon(
                points,
                self.color,
                egui::Stroke::NONE,
            ));
        }

        response
    }

    /// Draw a pin dot
    pub fn pin_dot(self, ui: &mut Ui, connected: bool) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), egui::Sense::hover());

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let center = rect.center();
            let radius = self.size / 2.0 - 1.0;

            if connected {
                painter.circle_filled(center, radius, self.color);
            } else {
                painter.circle_stroke(center, radius, egui::Stroke::new(1.5, self.color));
            }
        }

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_icon_as_str() {
        assert_eq!(Icon::Play.as_str(), "▶");
        assert_eq!(Icon::Plus.as_str(), "+");
    }

    #[test]
    fn test_icon_display() {
        assert_eq!(format!("{}", Icon::Check), "✓");
    }
}
