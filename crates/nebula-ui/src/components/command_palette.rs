//! Command palette component (similar to VS Code's Ctrl+P).

use crate::theme::current_theme;
use egui::{Context, Id, Key, Response, RichText, Ui, Vec2};

/// A command item in the palette
#[derive(Clone, Debug)]
pub struct CommandItem {
    /// Unique identifier
    pub id: String,
    /// Display label
    pub label: String,
    /// Optional description
    pub description: Option<String>,
    /// Optional keyboard shortcut
    pub shortcut: Option<String>,
    /// Optional icon
    pub icon: Option<String>,
    /// Optional category/group
    pub category: Option<String>,
    /// Whether item is disabled
    pub disabled: bool,
}

impl CommandItem {
    /// Create a new command item
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            shortcut: None,
            icon: None,
            category: None,
            disabled: false,
        }
    }

    /// Add description
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Add shortcut hint
    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Add icon
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set category
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Set disabled
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

/// Command palette response
#[derive(Clone, Debug)]
pub enum CommandPaletteResponse {
    /// Palette is still open
    Open,
    /// User selected a command
    Selected(String),
    /// User dismissed the palette
    Dismissed,
}

/// Command palette component
///
/// # Example
///
/// ```rust,ignore
/// let mut open = false;
/// let mut query = String::new();
/// let mut selected_index = 0usize;
/// let commands = vec![
///     CommandItem::new("save", "Save File").shortcut("Ctrl+S"),
///     CommandItem::new("open", "Open File").shortcut("Ctrl+O"),
/// ];
///
/// let response = CommandPalette::new(&mut open, &mut query, &commands, &mut selected_index)
///     .show(ctx);
///
/// if let CommandPaletteResponse::Selected(id) = response {
///     match id.as_str() {
///         "save" => save_file(),
///         _ => {}
///     }
/// }
/// ```
pub struct CommandPalette<'a> {
    open: &'a mut bool,
    query: &'a mut String,
    commands: &'a [CommandItem],
    selected_index: &'a mut usize,
    placeholder: &'a str,
    max_results: usize,
    width: f32,
    fuzzy_match: bool,
}

impl<'a> CommandPalette<'a> {
    /// Create a new command palette
    pub fn new(
        open: &'a mut bool,
        query: &'a mut String,
        commands: &'a [CommandItem],
        selected_index: &'a mut usize,
    ) -> Self {
        Self {
            open,
            query,
            commands,
            selected_index,
            placeholder: "Type a command...",
            max_results: 10,
            width: 500.0,
            fuzzy_match: true,
        }
    }

    /// Set placeholder text
    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// Set maximum results to show
    pub fn max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }

    /// Set width
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Disable fuzzy matching
    pub fn exact_match(mut self) -> Self {
        self.fuzzy_match = false;
        self
    }

    /// Filter commands and return indices
    fn filter_command_indices(&self) -> Vec<usize> {
        if self.query.is_empty() {
            return (0..self.commands.len().min(self.max_results)).collect();
        }

        let query_lower = self.query.to_lowercase();

        let mut results: Vec<(usize, i32)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(idx, cmd)| {
                let label_lower = cmd.label.to_lowercase();

                let score = if self.fuzzy_match {
                    fuzzy_score(&query_lower, &label_lower)
                } else if label_lower.contains(&query_lower) {
                    Some(100)
                } else {
                    None
                };

                score.map(|s| (idx, s))
            })
            .collect();

        // Sort by score (higher is better)
        results.sort_by(|a, b| b.1.cmp(&a.1));

        results
            .into_iter()
            .take(self.max_results)
            .map(|(idx, _)| idx)
            .collect()
    }

    /// Show the command palette
    pub fn show(self, ctx: &Context) -> CommandPaletteResponse {
        if !*self.open {
            return CommandPaletteResponse::Dismissed;
        }

        let theme = current_theme();
        let tokens = &theme.tokens;

        // Get filtered indices (Vec<usize> doesn't borrow self)
        let filtered_indices = self.filter_command_indices();
        let filtered_len = filtered_indices.len();

        // Clamp selected index
        if *self.selected_index >= filtered_len {
            *self.selected_index = filtered_len.saturating_sub(1);
        }

        let mut response = CommandPaletteResponse::Open;
        let mut should_close = false;
        let mut selected_id: Option<String> = None;

        // Handle keyboard navigation
        let escape_pressed = ctx.input(|i| i.key_pressed(Key::Escape));
        let up_pressed = ctx.input(|i| i.key_pressed(Key::ArrowUp));
        let down_pressed = ctx.input(|i| i.key_pressed(Key::ArrowDown));
        let enter_pressed = ctx.input(|i| i.key_pressed(Key::Enter));

        if escape_pressed {
            should_close = true;
            response = CommandPaletteResponse::Dismissed;
        }

        if up_pressed {
            *self.selected_index = self.selected_index.saturating_sub(1);
        }

        if down_pressed && *self.selected_index < filtered_len.saturating_sub(1) {
            *self.selected_index += 1;
        }

        if enter_pressed {
            if let Some(&cmd_idx) = filtered_indices.get(*self.selected_index) {
                if let Some(cmd) = self.commands.get(cmd_idx) {
                    if !cmd.disabled {
                        should_close = true;
                        selected_id = Some(cmd.id.clone());
                    }
                }
            }
        }

        // Backdrop
        egui::Area::new(Id::new("command_palette_backdrop"))
            .fixed_pos(egui::Pos2::ZERO)
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                let screen = ui.ctx().input(|i| i.screen_rect());

                // Semi-transparent backdrop
                ui.painter()
                    .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(100));

                // Close on backdrop click
                let backdrop_response = ui.allocate_rect(screen, egui::Sense::click());
                if backdrop_response.clicked() {
                    should_close = true;
                    response = CommandPaletteResponse::Dismissed;
                }
            });

        // Command palette window
        egui::Window::new("Command Palette")
            .id(Id::new("command_palette"))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 100.0))
            .fixed_size(Vec2::new(self.width, 0.0))
            .frame(
                egui::Frame::NONE
                    .fill(tokens.card)
                    .stroke(egui::Stroke::new(1.0, tokens.border))
                    .corner_radius(tokens.rounding_lg())
                    .shadow(egui::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: tokens.shadow_color,
                    }),
            )
            .show(ctx, |ui| {
                ui.set_min_width(self.width);

                // Search input
                let input_frame = egui::Frame::NONE
                    .fill(tokens.input)
                    .inner_margin(tokens.spacing_md as i8);

                input_frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("🔍")
                                .size(tokens.font_size_md)
                                .color(tokens.muted_foreground),
                        );

                        let text_edit = egui::TextEdit::singleline(self.query)
                            .hint_text(self.placeholder)
                            .frame(false)
                            .desired_width(ui.available_width());

                        let edit_response = ui.add(text_edit);
                        edit_response.request_focus();
                    });
                });

                // Separator
                ui.add(egui::Separator::default().spacing(0.0));

                // Results list
                if filtered_indices.is_empty() {
                    ui.add_space(tokens.spacing_lg);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("No results found")
                                .size(tokens.font_size_sm)
                                .color(tokens.muted_foreground),
                        );
                    });
                    ui.add_space(tokens.spacing_lg);
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            let mut current_category: Option<&str> = None;

                            for (i, &cmd_idx) in filtered_indices.iter().enumerate() {
                                let cmd = &self.commands[cmd_idx];

                                // Category header
                                if let Some(cat) = &cmd.category {
                                    if current_category != Some(cat.as_str()) {
                                        current_category = Some(cat.as_str());

                                        ui.add_space(tokens.spacing_xs);
                                        ui.label(
                                            RichText::new(cat)
                                                .size(tokens.font_size_xs)
                                                .color(tokens.muted_foreground),
                                        );
                                        ui.add_space(tokens.spacing_xs);
                                    }
                                }

                                let is_selected = i == *self.selected_index;
                                let item_response = show_command_item(ui, cmd, is_selected);

                                if item_response.clicked() && !cmd.disabled {
                                    should_close = true;
                                    selected_id = Some(cmd.id.clone());
                                }

                                if item_response.hovered() {
                                    *self.selected_index = i;
                                }
                            }
                        });
                }

                // Footer with hints
                ui.add_space(tokens.spacing_xs);
                ui.add(egui::Separator::default().spacing(0.0));

                let footer_frame = egui::Frame::NONE
                    .fill(tokens.muted.gamma_multiply(0.3))
                    .inner_margin(egui::Margin::symmetric(
                        tokens.spacing_md as i8,
                        tokens.spacing_xs as i8,
                    ));

                footer_frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("↑↓ Navigate")
                                .size(tokens.font_size_xs)
                                .color(tokens.muted_foreground),
                        );
                        ui.add_space(tokens.spacing_md);
                        ui.label(
                            RichText::new("↵ Select")
                                .size(tokens.font_size_xs)
                                .color(tokens.muted_foreground),
                        );
                        ui.add_space(tokens.spacing_md);
                        ui.label(
                            RichText::new("Esc Dismiss")
                                .size(tokens.font_size_xs)
                                .color(tokens.muted_foreground),
                        );
                    });
                });
            });

        // Handle close/select after all UI
        if should_close {
            *self.open = false;
        }

        if let Some(id) = selected_id {
            return CommandPaletteResponse::Selected(id);
        }

        response
    }
}

/// Show a command item
fn show_command_item(ui: &mut Ui, cmd: &CommandItem, selected: bool) -> Response {
    let theme = current_theme();
    let tokens = &theme.tokens;

    let bg_color = if selected {
        tokens.accent
    } else {
        egui::Color32::TRANSPARENT
    };

    let text_color = if cmd.disabled {
        tokens.muted_foreground
    } else {
        tokens.foreground
    };

    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();

        // Background
        if selected || response.hovered() {
            painter.rect_filled(rect, 0.0, bg_color);
        }

        // Icon
        let mut content_x = rect.min.x + tokens.spacing_md;

        if let Some(icon) = &cmd.icon {
            painter.text(
                egui::Pos2::new(content_x + 8.0, rect.center().y),
                egui::Align2::CENTER_CENTER,
                icon,
                egui::FontId::proportional(tokens.font_size_md),
                text_color,
            );
            content_x += 24.0;
        }

        // Label
        painter.text(
            egui::Pos2::new(content_x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &cmd.label,
            egui::FontId::proportional(tokens.font_size_sm),
            text_color,
        );

        // Description
        if let Some(desc) = &cmd.description {
            let label_width = cmd.label.len() as f32 * 7.0;
            painter.text(
                egui::Pos2::new(content_x + label_width + tokens.spacing_md, rect.center().y),
                egui::Align2::LEFT_CENTER,
                desc,
                egui::FontId::proportional(tokens.font_size_xs),
                tokens.muted_foreground,
            );
        }

        // Shortcut
        if let Some(shortcut) = &cmd.shortcut {
            let shortcut_frame = egui::Rect::from_min_size(
                egui::Pos2::new(rect.max.x - 80.0, rect.center().y - 10.0),
                Vec2::new(70.0, 20.0),
            );

            painter.rect_filled(shortcut_frame, tokens.radius_sm, tokens.muted);

            painter.text(
                shortcut_frame.center(),
                egui::Align2::CENTER_CENTER,
                shortcut,
                egui::FontId::proportional(tokens.font_size_xs),
                tokens.muted_foreground,
            );
        }
    }

    response
}

/// Simple fuzzy matching score
fn fuzzy_score(query: &str, target: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let mut score = 0i32;
    let mut query_chars = query.chars().peekable();
    let mut last_match_idx: Option<usize> = None;
    let mut consecutive_bonus = 0i32;

    for (idx, target_char) in target.chars().enumerate() {
        if let Some(&query_char) = query_chars.peek() {
            if query_char == target_char {
                query_chars.next();
                score += 10;

                // Bonus for consecutive matches
                if let Some(last) = last_match_idx {
                    if idx == last + 1 {
                        consecutive_bonus += 5;
                        score += consecutive_bonus;
                    } else {
                        consecutive_bonus = 0;
                    }
                }

                // Bonus for matching at start or after separator
                if idx == 0 {
                    score += 15;
                } else if target
                    .chars()
                    .nth(idx - 1)
                    .is_some_and(|c| c == ' ' || c == '_' || c == '-')
                {
                    score += 10;
                }

                last_match_idx = Some(idx);
            }
        }
    }

    // All query characters must be matched
    if query_chars.peek().is_some() {
        None
    } else {
        Some(score)
    }
}

/// Quick action bar (simplified command palette)
pub struct QuickActions<'a> {
    actions: &'a [(&'a str, &'a str)], // (id, label)
}

impl<'a> QuickActions<'a> {
    /// Create quick actions bar
    pub fn new(actions: &'a [(&'a str, &'a str)]) -> Self {
        Self { actions }
    }

    /// Show the quick actions
    pub fn show(self, ui: &mut Ui) -> Option<&'a str> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let mut clicked = None;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = tokens.spacing_xs;

            for (id, label) in self.actions {
                let button = egui::Button::new(
                    RichText::new(*label)
                        .size(tokens.font_size_sm)
                        .color(tokens.foreground),
                )
                .fill(tokens.secondary)
                .corner_radius(tokens.rounding_md());

                if ui.add(button).clicked() {
                    clicked = Some(*id);
                }
            }
        });

        clicked
    }
}
