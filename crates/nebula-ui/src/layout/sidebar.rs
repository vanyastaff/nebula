//! Sidebar navigation component.

use crate::icons::Icon;
use crate::theme::current_theme;
use egui::{Color32, RichText, Ui};

/// A sidebar item
#[derive(Clone)]
pub struct SidebarItem {
    /// Unique identifier
    pub id: String,
    /// Display label
    pub label: String,
    /// Icon
    pub icon: Option<Icon>,
    /// Badge count (e.g., notifications)
    pub badge: Option<u32>,
    /// Whether this item is disabled
    pub disabled: bool,
    /// Nested items
    pub children: Vec<SidebarItem>,
}

impl SidebarItem {
    /// Create a new sidebar item
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            badge: None,
            disabled: false,
            children: Vec::new(),
        }
    }

    /// Set icon
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Set badge count
    pub fn badge(mut self, count: u32) -> Self {
        self.badge = Some(count);
        self
    }

    /// Set disabled
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    /// Add child items
    pub fn children(mut self, children: Vec<SidebarItem>) -> Self {
        self.children = children;
        self
    }
}

/// A section in the sidebar
pub struct SidebarSection {
    /// Section title (optional)
    pub title: Option<String>,
    /// Items in this section
    pub items: Vec<SidebarItem>,
}

impl SidebarSection {
    /// Create a new section
    pub fn new(items: Vec<SidebarItem>) -> Self {
        Self { title: None, items }
    }

    /// Create a section with title
    pub fn titled(title: impl Into<String>, items: Vec<SidebarItem>) -> Self {
        Self {
            title: Some(title.into()),
            items,
        }
    }
}

/// Sidebar component
pub struct Sidebar<'a> {
    selected: &'a mut Option<String>,
    sections: Vec<SidebarSection>,
    collapsed: bool,
    width: f32,
    collapsed_width: f32,
    header: Option<Box<dyn FnOnce(&mut Ui) + 'a>>,
    footer: Option<Box<dyn FnOnce(&mut Ui) + 'a>>,
}

impl<'a> Sidebar<'a> {
    /// Create a new sidebar
    pub fn new(selected: &'a mut Option<String>) -> Self {
        Self {
            selected,
            sections: Vec::new(),
            collapsed: false,
            width: 240.0,
            collapsed_width: 56.0,
            header: None,
            footer: None,
        }
    }

    /// Add a section
    pub fn section(mut self, section: SidebarSection) -> Self {
        self.sections.push(section);
        self
    }

    /// Add sections
    pub fn sections(mut self, sections: Vec<SidebarSection>) -> Self {
        self.sections = sections;
        self
    }

    /// Set collapsed state
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    /// Set expanded width
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Set collapsed width
    pub fn collapsed_width(mut self, width: f32) -> Self {
        self.collapsed_width = width;
        self
    }

    /// Add header content
    pub fn header(mut self, header: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.header = Some(Box::new(header));
        self
    }

    /// Add footer content
    pub fn footer(mut self, footer: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.footer = Some(Box::new(footer));
        self
    }

    /// Show the sidebar
    pub fn show(mut self, ui: &mut Ui) -> SidebarResponse {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let width = if self.collapsed {
            self.collapsed_width
        } else {
            self.width
        };

        let mut clicked_item = None;

        let header = self.header.take();
        let footer = self.footer.take();
        let collapsed = self.collapsed;
        let sections = &self.sections;

        egui::Frame::NONE
            .fill(tokens.card)
            .stroke(egui::Stroke::new(1.0, tokens.border))
            .show(ui, |ui| {
                ui.set_min_width(width);
                ui.set_max_width(width);

                ui.vertical(|ui| {
                    // Header
                    if let Some(header) = header {
                        ui.add_space(tokens.spacing_md);
                        header(ui);
                        ui.add_space(tokens.spacing_md);
                        ui.separator();
                    }

                    // Scrollable content
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add_space(tokens.spacing_sm);

                            for (section_idx, section) in sections.iter().enumerate() {
                                if section_idx > 0 {
                                    ui.add_space(tokens.spacing_md);
                                }

                                // Section title
                                if let Some(title) = &section.title {
                                    if !collapsed {
                                        ui.add_space(tokens.spacing_sm);
                                        ui.label(
                                            RichText::new(title.to_uppercase())
                                                .size(tokens.font_size_xs)
                                                .color(tokens.muted_foreground),
                                        );
                                        ui.add_space(tokens.spacing_xs);
                                    }
                                }

                                // Items
                                for item in &section.items {
                                    if let Some(id) = self.show_item(ui, item, collapsed) {
                                        clicked_item = Some(id);
                                    }
                                }
                            }

                            ui.add_space(tokens.spacing_sm);
                        });

                    // Footer
                    if let Some(footer) = footer {
                        ui.separator();
                        ui.add_space(tokens.spacing_md);
                        footer(ui);
                        ui.add_space(tokens.spacing_md);
                    }
                });
            });

        // Update selection
        if let Some(id) = &clicked_item {
            *self.selected = Some(id.clone());
        }

        SidebarResponse { clicked_item }
    }

    fn show_item(&self, ui: &mut Ui, item: &SidebarItem, collapsed: bool) -> Option<String> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let is_selected = self.selected.as_ref() == Some(&item.id);
        let has_children = !item.children.is_empty();

        let bg_color = if is_selected {
            tokens.accent
        } else {
            Color32::TRANSPARENT
        };

        let fg_color = if item.disabled {
            tokens.muted_foreground
        } else if is_selected {
            tokens.accent_foreground
        } else {
            tokens.foreground
        };

        let mut clicked = None;

        let frame = egui::Frame::NONE
            .fill(bg_color)
            .corner_radius(tokens.rounding_md())
            .inner_margin(egui::Margin::symmetric(
                tokens.spacing_sm as i8,
                tokens.spacing_sm as i8,
            ));

        let response = frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                // Icon
                if let Some(icon) = item.icon {
                    ui.label(icon.rich_text().size(tokens.font_size_lg).color(fg_color));

                    if !collapsed {
                        ui.add_space(tokens.spacing_sm);
                    }
                }

                // Label (only if not collapsed)
                if !collapsed {
                    ui.label(
                        RichText::new(&item.label)
                            .size(tokens.font_size_md)
                            .color(fg_color),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Badge
                        if let Some(count) = item.badge {
                            let badge_text = if count > 99 {
                                "99+".to_string()
                            } else {
                                count.to_string()
                            };

                            egui::Frame::NONE
                                .fill(tokens.primary)
                                .corner_radius(egui::CornerRadius::same(tokens.radius_full as u8))
                                .inner_margin(egui::Margin::symmetric(
                                    tokens.spacing_xs as i8,
                                    (tokens.spacing_xs / 2.0) as i8,
                                ))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(badge_text)
                                            .size(tokens.font_size_xs)
                                            .color(tokens.primary_foreground),
                                    );
                                });
                        }

                        // Expand arrow for items with children
                        if has_children {
                            ui.label(
                                RichText::new("›")
                                    .size(tokens.font_size_md)
                                    .color(tokens.muted_foreground),
                            );
                        }
                    });
                }
            });
        });

        // Handle click
        let click_response = ui.interact(
            response.response.rect,
            ui.id().with(&item.id),
            if item.disabled {
                egui::Sense::hover()
            } else {
                egui::Sense::click()
            },
        );

        if click_response.clicked() && !item.disabled {
            clicked = Some(item.id.clone());
        }

        if click_response.hovered() && !item.disabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        // Tooltip for collapsed mode
        if collapsed && click_response.hovered() {
            let label = item.label.clone();
            egui::show_tooltip_at_pointer(
                ui.ctx(),
                egui::LayerId::new(egui::Order::Tooltip, ui.id().with("tooltip_layer")),
                ui.id().with("tooltip"),
                |ui: &mut Ui| {
                    ui.label(&label);
                },
            );
        }

        clicked
    }
}

/// Response from showing a sidebar
pub struct SidebarResponse {
    /// The item that was clicked (if any)
    pub clicked_item: Option<String>,
}

impl SidebarResponse {
    /// Check if an item was clicked
    pub fn clicked(&self) -> bool {
        self.clicked_item.is_some()
    }
}
