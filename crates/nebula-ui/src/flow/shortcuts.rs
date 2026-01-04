//! Keyboard shortcuts for flow editor.

use egui::{Context, Key, Modifiers};

/// Action triggered by keyboard shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    /// Undo last action.
    Undo,
    /// Redo last undone action.
    Redo,
    /// Delete selected items.
    Delete,
    /// Select all.
    SelectAll,
    /// Deselect all.
    DeselectAll,
    /// Copy selected items.
    Copy,
    /// Cut selected items.
    Cut,
    /// Paste copied items.
    Paste,
    /// Duplicate selected items.
    Duplicate,
    /// Zoom in.
    ZoomIn,
    /// Zoom out.
    ZoomOut,
    /// Reset zoom to 100%.
    ZoomReset,
    /// Fit view to all nodes.
    FitView,
    /// Toggle fullscreen.
    ToggleFullscreen,
    /// Find/search.
    Find,
    /// Save.
    Save,
}

/// Configuration for keyboard shortcuts.
#[derive(Debug, Clone)]
pub struct ShortcutsConfig {
    /// Enable undo/redo shortcuts.
    pub enable_undo_redo: bool,
    /// Enable delete shortcut.
    pub enable_delete: bool,
    /// Enable selection shortcuts.
    pub enable_selection: bool,
    /// Enable clipboard shortcuts.
    pub enable_clipboard: bool,
    /// Enable zoom shortcuts.
    pub enable_zoom: bool,
    /// Enable view shortcuts.
    pub enable_view: bool,
    /// Enable save shortcut.
    pub enable_save: bool,
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self {
            enable_undo_redo: true,
            enable_delete: true,
            enable_selection: true,
            enable_clipboard: true,
            enable_zoom: true,
            enable_view: true,
            enable_save: true,
        }
    }
}

/// Keyboard shortcuts handler.
pub struct KeyboardShortcuts {
    config: ShortcutsConfig,
}

impl KeyboardShortcuts {
    /// Creates a new keyboard shortcuts handler.
    pub fn new() -> Self {
        Self {
            config: ShortcutsConfig::default(),
        }
    }

    /// Sets the configuration.
    pub fn config(mut self, config: ShortcutsConfig) -> Self {
        self.config = config;
        self
    }

    /// Processes keyboard input and returns triggered actions.
    pub fn process(&self, ctx: &Context) -> Vec<ShortcutAction> {
        let mut actions = Vec::new();

        ctx.input(|i| {
            let modifiers = i.modifiers;

            // Undo/Redo
            if self.config.enable_undo_redo {
                if i.key_pressed(Key::Z) {
                    if is_command(modifiers) && modifiers.shift {
                        actions.push(ShortcutAction::Redo);
                    } else if is_command(modifiers) {
                        actions.push(ShortcutAction::Undo);
                    }
                }

                if i.key_pressed(Key::Y) && is_command(modifiers) {
                    actions.push(ShortcutAction::Redo);
                }
            }

            // Delete
            if self.config.enable_delete {
                if i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace) {
                    actions.push(ShortcutAction::Delete);
                }
            }

            // Selection
            if self.config.enable_selection {
                if i.key_pressed(Key::A) && is_command(modifiers) {
                    actions.push(ShortcutAction::SelectAll);
                }

                if i.key_pressed(Key::Escape) {
                    actions.push(ShortcutAction::DeselectAll);
                }
            }

            // Clipboard
            if self.config.enable_clipboard {
                if i.key_pressed(Key::C) && is_command(modifiers) {
                    actions.push(ShortcutAction::Copy);
                }

                if i.key_pressed(Key::X) && is_command(modifiers) {
                    actions.push(ShortcutAction::Cut);
                }

                if i.key_pressed(Key::V) && is_command(modifiers) {
                    actions.push(ShortcutAction::Paste);
                }

                if i.key_pressed(Key::D) && is_command(modifiers) {
                    actions.push(ShortcutAction::Duplicate);
                }
            }

            // Zoom
            if self.config.enable_zoom {
                // Zoom in: Ctrl/Cmd + Plus or Ctrl/Cmd + =
                if is_command(modifiers) && i.key_pressed(Key::Plus) {
                    actions.push(ShortcutAction::ZoomIn);
                }

                // Zoom out: Ctrl/Cmd + Minus
                if is_command(modifiers) && i.key_pressed(Key::Minus) {
                    actions.push(ShortcutAction::ZoomOut);
                }

                // Reset zoom: Ctrl/Cmd + 0
                if is_command(modifiers) && i.key_pressed(Key::Num0) {
                    actions.push(ShortcutAction::ZoomReset);
                }
            }

            // View
            if self.config.enable_view {
                // Fit view: Ctrl/Cmd + Shift + 1
                if is_command(modifiers) && modifiers.shift && i.key_pressed(Key::Num1) {
                    actions.push(ShortcutAction::FitView);
                }

                // Fullscreen: F11
                if i.key_pressed(Key::F11) {
                    actions.push(ShortcutAction::ToggleFullscreen);
                }
            }

            // Find
            if i.key_pressed(Key::F) && is_command(modifiers) {
                actions.push(ShortcutAction::Find);
            }

            // Save
            if self.config.enable_save {
                if i.key_pressed(Key::S) && is_command(modifiers) {
                    actions.push(ShortcutAction::Save);
                }
            }
        });

        actions
    }

    /// Returns a description of all available shortcuts.
    pub fn get_shortcuts_help(&self) -> Vec<(&'static str, &'static str)> {
        let mut help = Vec::new();

        if self.config.enable_undo_redo {
            help.push(("Ctrl/Cmd + Z", "Undo"));
            help.push(("Ctrl/Cmd + Shift + Z", "Redo"));
            help.push(("Ctrl/Cmd + Y", "Redo"));
        }

        if self.config.enable_delete {
            help.push(("Delete/Backspace", "Delete selected"));
        }

        if self.config.enable_selection {
            help.push(("Ctrl/Cmd + A", "Select all"));
            help.push(("Escape", "Deselect all"));
        }

        if self.config.enable_clipboard {
            help.push(("Ctrl/Cmd + C", "Copy"));
            help.push(("Ctrl/Cmd + X", "Cut"));
            help.push(("Ctrl/Cmd + V", "Paste"));
            help.push(("Ctrl/Cmd + D", "Duplicate"));
        }

        if self.config.enable_zoom {
            help.push(("Ctrl/Cmd + Plus", "Zoom in"));
            help.push(("Ctrl/Cmd + Minus", "Zoom out"));
            help.push(("Ctrl/Cmd + 0", "Reset zoom"));
        }

        if self.config.enable_view {
            help.push(("Ctrl/Cmd + Shift + 1", "Fit view"));
            help.push(("F11", "Toggle fullscreen"));
        }

        help.push(("Ctrl/Cmd + F", "Find"));

        if self.config.enable_save {
            help.push(("Ctrl/Cmd + S", "Save"));
        }

        help
    }
}

impl Default for KeyboardShortcuts {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if Ctrl (Windows/Linux) or Cmd (macOS) is pressed.
fn is_command(modifiers: Modifiers) -> bool {
    modifiers.command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shortcuts_config_default() {
        let config = ShortcutsConfig::default();
        assert!(config.enable_undo_redo);
        assert!(config.enable_delete);
        assert!(config.enable_selection);
    }

    #[test]
    fn test_shortcuts_help() {
        let shortcuts = KeyboardShortcuts::new();
        let help = shortcuts.get_shortcuts_help();
        assert!(!help.is_empty());
        assert!(help.iter().any(|(_, desc)| *desc == "Undo"));
    }
}
