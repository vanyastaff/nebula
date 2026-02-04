//! Table component for displaying tabular data.

use crate::theme::current_theme;
use egui::{Ui, Vec2};

/// Table column definition
pub struct TableColumn<'a> {
    /// Column header
    pub header: &'a str,
    /// Column width (None for auto)
    pub width: Option<f32>,
    /// Sortable
    pub sortable: bool,
    /// Alignment
    pub align: TableAlign,
}

/// Table alignment
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TableAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl<'a> TableColumn<'a> {
    /// Create a new column
    pub fn new(header: &'a str) -> Self {
        Self {
            header,
            width: None,
            sortable: false,
            align: TableAlign::Left,
        }
    }

    /// Set fixed width
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Make sortable
    pub fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }

    /// Set alignment
    pub fn align(mut self, align: TableAlign) -> Self {
        self.align = align;
        self
    }

    /// Right align
    pub fn right(mut self) -> Self {
        self.align = TableAlign::Right;
        self
    }

    /// Center align
    pub fn center(mut self) -> Self {
        self.align = TableAlign::Center;
        self
    }
}

/// Sort state for sortable tables
#[derive(Clone, Debug, Default)]
pub struct TableSort {
    /// Column index
    pub column: usize,
    /// Ascending order
    pub ascending: bool,
}

/// A table component
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::{Table, TableColumn};
///
/// let columns = vec![
///     TableColumn::new("Name"),
///     TableColumn::new("Age").width(60.0).right(),
///     TableColumn::new("Email"),
/// ];
///
/// Table::new(columns)
///     .striped()
///     .show(ui, |table| {
///         table.row(|row| {
///             row.cell(|ui| { ui.label("John"); });
///             row.cell(|ui| { ui.label("30"); });
///             row.cell(|ui| { ui.label("john@example.com"); });
///         });
///     });
/// ```
pub struct Table<'a> {
    columns: Vec<TableColumn<'a>>,
    striped: bool,
    bordered: bool,
    hoverable: bool,
    sort: Option<&'a mut TableSort>,
}

impl<'a> Table<'a> {
    /// Create a new table
    pub fn new(columns: Vec<TableColumn<'a>>) -> Self {
        Self {
            columns,
            striped: false,
            bordered: false,
            hoverable: true,
            sort: None,
        }
    }

    /// Enable striped rows
    pub fn striped(mut self) -> Self {
        self.striped = true;
        self
    }

    /// Enable borders
    pub fn bordered(mut self) -> Self {
        self.bordered = true;
        self
    }

    /// Disable hover effect
    pub fn no_hover(mut self) -> Self {
        self.hoverable = false;
        self
    }

    /// Enable sorting
    pub fn sortable(mut self, sort: &'a mut TableSort) -> Self {
        self.sort = Some(sort);
        self
    }

    /// Show the table
    pub fn show<R>(mut self, ui: &mut Ui, add_rows: impl FnOnce(&mut TableBuilder<'_>) -> R) -> R {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let available_width = ui.available_width();
        let col_count = self.columns.len();

        // Calculate column widths
        let fixed_width: f32 = self.columns.iter().filter_map(|c| c.width).sum();
        let auto_cols = self.columns.iter().filter(|c| c.width.is_none()).count();
        let auto_width = if auto_cols > 0 {
            (available_width - fixed_width) / auto_cols as f32
        } else {
            0.0
        };

        let col_widths: Vec<f32> = self
            .columns
            .iter()
            .map(|c| c.width.unwrap_or(auto_width))
            .collect();

        ui.vertical(|ui| {
            // Header
            let header_height = 40.0;

            ui.horizontal(|ui| {
                for (i, col) in self.columns.iter().enumerate() {
                    let width = col_widths[i];

                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(width, header_height),
                        if col.sortable {
                            egui::Sense::click()
                        } else {
                            egui::Sense::hover()
                        },
                    );

                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter();

                        // Background
                        painter.rect_filled(rect, 0.0, tokens.muted);

                        // Border
                        if self.bordered {
                            painter.rect_stroke(
                                rect,
                                0.0,
                                egui::Stroke::new(1.0, tokens.border),
                                egui::StrokeKind::Inside,
                            );
                        }

                        // Header text
                        let text_pos = match col.align {
                            TableAlign::Left => egui::Pos2::new(rect.left() + 8.0, rect.center().y),
                            TableAlign::Center => rect.center(),
                            TableAlign::Right => {
                                egui::Pos2::new(rect.right() - 8.0, rect.center().y)
                            }
                        };

                        let align = match col.align {
                            TableAlign::Left => egui::Align2::LEFT_CENTER,
                            TableAlign::Center => egui::Align2::CENTER_CENTER,
                            TableAlign::Right => egui::Align2::RIGHT_CENTER,
                        };

                        let mut header_text = col.header.to_string();

                        // Sort indicator
                        if let Some(sort) = &self.sort {
                            if sort.column == i {
                                header_text.push_str(if sort.ascending { " ▲" } else { " ▼" });
                            }
                        }

                        painter.text(
                            text_pos,
                            align,
                            &header_text,
                            egui::FontId::proportional(tokens.font_size_sm),
                            tokens.foreground,
                        );
                    }

                    // Handle sort click
                    if response.clicked() && col.sortable {
                        if let Some(sort) = &mut self.sort {
                            if sort.column == i {
                                sort.ascending = !sort.ascending;
                            } else {
                                sort.column = i;
                                sort.ascending = true;
                            }
                        }
                    }

                    if response.hovered() && col.sortable {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                }
            });

            // Body
            let mut builder = TableBuilder {
                col_widths: &col_widths,
                col_aligns: self.columns.iter().map(|c| c.align).collect(),
                row_index: 0,
                striped: self.striped,
                bordered: self.bordered,
                hoverable: self.hoverable,
            };

            add_rows(&mut builder)
        })
        .inner
    }
}

/// Table builder for adding rows
pub struct TableBuilder<'a> {
    col_widths: &'a [f32],
    col_aligns: Vec<TableAlign>,
    row_index: usize,
    striped: bool,
    bordered: bool,
    hoverable: bool,
}

impl<'a> TableBuilder<'a> {
    /// Add a row
    pub fn row(&mut self, ui: &mut Ui, add_cells: impl FnOnce(&mut RowBuilder<'_>)) {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let row_height = 40.0;
        let is_striped = self.striped && self.row_index % 2 == 1;

        ui.horizontal(|ui| {
            let mut builder = RowBuilder {
                col_widths: self.col_widths,
                col_aligns: &self.col_aligns,
                col_index: 0,
                row_height,
                is_striped,
                bordered: self.bordered,
            };

            add_cells(&mut builder);
        });

        self.row_index += 1;
    }
}

/// Row builder for adding cells
pub struct RowBuilder<'a> {
    col_widths: &'a [f32],
    col_aligns: &'a [TableAlign],
    col_index: usize,
    row_height: f32,
    is_striped: bool,
    bordered: bool,
}

impl<'a> RowBuilder<'a> {
    /// Add a cell
    pub fn cell(&mut self, ui: &mut Ui, add_content: impl FnOnce(&mut Ui)) {
        let theme = current_theme();
        let tokens = &theme.tokens;

        if self.col_index >= self.col_widths.len() {
            return;
        }

        let width = self.col_widths[self.col_index];
        let align = self
            .col_aligns
            .get(self.col_index)
            .copied()
            .unwrap_or(TableAlign::Left);

        let (rect, _response) =
            ui.allocate_exact_size(Vec2::new(width, self.row_height), egui::Sense::hover());

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();

            // Striped background
            if self.is_striped {
                painter.rect_filled(rect, 0.0, tokens.muted.linear_multiply(0.5));
            }

            // Border
            if self.bordered {
                painter.rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(1.0, tokens.border),
                    egui::StrokeKind::Inside,
                );
            }
        }

        // Content
        let content_rect = rect.shrink(8.0);
        let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(content_rect).layout(
            match align {
                TableAlign::Left => egui::Layout::left_to_right(egui::Align::Center),
                TableAlign::Center => {
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight)
                }
                TableAlign::Right => egui::Layout::right_to_left(egui::Align::Center),
            },
        ));

        add_content(&mut child_ui);

        self.col_index += 1;
    }
}
