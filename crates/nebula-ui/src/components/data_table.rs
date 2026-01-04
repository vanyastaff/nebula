//! Data table component with sorting, filtering, and pagination.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Vec2};
use std::cmp::Ordering;

/// Column definition for data table
#[derive(Clone, Debug)]
pub struct DataColumn<T> {
    /// Column ID
    pub id: String,
    /// Header text
    pub header: String,
    /// Width (None = auto)
    pub width: Option<f32>,
    /// Whether column is sortable
    pub sortable: bool,
    /// Whether column is filterable
    pub filterable: bool,
    /// Cell renderer
    pub render: fn(&T) -> String,
    /// Sort comparator
    pub compare: Option<fn(&T, &T) -> Ordering>,
    /// Alignment
    pub align: ColumnAlign,
}

/// Column alignment
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl<T> DataColumn<T> {
    /// Create a new column
    pub fn new(id: impl Into<String>, header: impl Into<String>, render: fn(&T) -> String) -> Self {
        Self {
            id: id.into(),
            header: header.into(),
            width: None,
            sortable: false,
            filterable: false,
            render,
            compare: None,
            align: ColumnAlign::Left,
        }
    }

    /// Set width
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Make sortable
    pub fn sortable(mut self, compare: fn(&T, &T) -> Ordering) -> Self {
        self.sortable = true;
        self.compare = Some(compare);
        self
    }

    /// Make filterable
    pub fn filterable(mut self) -> Self {
        self.filterable = true;
        self
    }

    /// Set alignment
    pub fn align(mut self, align: ColumnAlign) -> Self {
        self.align = align;
        self
    }

    /// Right align
    pub fn right(mut self) -> Self {
        self.align = ColumnAlign::Right;
        self
    }

    /// Center align
    pub fn center(mut self) -> Self {
        self.align = ColumnAlign::Center;
        self
    }
}

/// Sort direction
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortDirection {
    #[default]
    None,
    Ascending,
    Descending,
}

impl SortDirection {
    fn toggle(&self) -> Self {
        match self {
            SortDirection::None => SortDirection::Ascending,
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::None,
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            SortDirection::None => "↕",
            SortDirection::Ascending => "↑",
            SortDirection::Descending => "↓",
        }
    }
}

/// Table state
#[derive(Clone, Debug, Default)]
pub struct DataTableState {
    /// Current sort column
    pub sort_column: Option<String>,
    /// Sort direction
    pub sort_direction: SortDirection,
    /// Filter values per column
    pub filters: std::collections::HashMap<String, String>,
    /// Current page (0-indexed)
    pub page: usize,
    /// Selected row indices
    pub selected: Vec<usize>,
}

impl DataTableState {
    /// Create new state
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all selections
    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// Select a row
    pub fn select(&mut self, index: usize) {
        if !self.selected.contains(&index) {
            self.selected.push(index);
        }
    }

    /// Deselect a row
    pub fn deselect(&mut self, index: usize) {
        self.selected.retain(|&i| i != index);
    }

    /// Toggle row selection
    pub fn toggle(&mut self, index: usize) {
        if self.selected.contains(&index) {
            self.deselect(index);
        } else {
            self.select(index);
        }
    }

    /// Check if row is selected
    pub fn is_selected(&self, index: usize) -> bool {
        self.selected.contains(&index)
    }
}

/// Data table component
///
/// # Example
///
/// ```rust,ignore
/// struct User { name: String, email: String, age: u32 }
///
/// let columns = vec![
///     DataColumn::new("name", "Name", |u: &User| u.name.clone()),
///     DataColumn::new("email", "Email", |u: &User| u.email.clone()),
///     DataColumn::new("age", "Age", |u: &User| u.age.to_string()).right(),
/// ];
///
/// let mut state = DataTableState::new();
/// DataTable::new(&users, &columns, &mut state)
///     .page_size(10)
///     .show(ui);
/// ```
pub struct DataTable<'a, T> {
    data: &'a [T],
    columns: &'a [DataColumn<T>],
    state: &'a mut DataTableState,
    page_size: usize,
    selectable: bool,
    striped: bool,
    bordered: bool,
    hoverable: bool,
    compact: bool,
    show_pagination: bool,
    empty_message: &'a str,
}

impl<'a, T> DataTable<'a, T> {
    /// Create a new data table
    pub fn new(data: &'a [T], columns: &'a [DataColumn<T>], state: &'a mut DataTableState) -> Self {
        Self {
            data,
            columns,
            state,
            page_size: 10,
            selectable: false,
            striped: true,
            bordered: true,
            hoverable: true,
            compact: false,
            show_pagination: true,
            empty_message: "No data available",
        }
    }

    /// Set page size
    pub fn page_size(mut self, size: usize) -> Self {
        self.page_size = size;
        self
    }

    /// Enable row selection
    pub fn selectable(mut self) -> Self {
        self.selectable = true;
        self
    }

    /// Disable striped rows
    pub fn no_stripes(mut self) -> Self {
        self.striped = false;
        self
    }

    /// Disable borders
    pub fn no_borders(mut self) -> Self {
        self.bordered = false;
        self
    }

    /// Disable hover effect
    pub fn no_hover(mut self) -> Self {
        self.hoverable = false;
        self
    }

    /// Compact mode (less padding)
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Hide pagination
    pub fn no_pagination(mut self) -> Self {
        self.show_pagination = false;
        self
    }

    /// Set empty message
    pub fn empty_message(mut self, msg: &'a str) -> Self {
        self.empty_message = msg;
        self
    }

    /// Get filtered and sorted data indices
    fn get_processed_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.data.len()).collect();

        // Apply filters
        for col in self.columns {
            if let Some(filter) = self.state.filters.get(&col.id) {
                if !filter.is_empty() {
                    let filter_lower = filter.to_lowercase();
                    indices.retain(|&i| {
                        let value = (col.render)(&self.data[i]).to_lowercase();
                        value.contains(&filter_lower)
                    });
                }
            }
        }

        // Apply sorting
        if let Some(sort_col) = &self.state.sort_column {
            if let Some(col) = self.columns.iter().find(|c| &c.id == sort_col) {
                if let Some(compare) = col.compare {
                    indices.sort_by(|&a, &b| {
                        let ordering = compare(&self.data[a], &self.data[b]);
                        match self.state.sort_direction {
                            SortDirection::Ascending => ordering,
                            SortDirection::Descending => ordering.reverse(),
                            SortDirection::None => Ordering::Equal,
                        }
                    });
                }
            }
        }

        indices
    }

    /// Show the table
    pub fn show(mut self, ui: &mut Ui) -> DataTableResponse {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let row_height = if self.compact { 32.0 } else { 44.0 };
        let padding = if self.compact {
            tokens.spacing_sm
        } else {
            tokens.spacing_md
        };

        let indices = self.get_processed_indices();
        let total_pages = (indices.len() + self.page_size - 1) / self.page_size;

        // Clamp page
        if self.state.page >= total_pages && total_pages > 0 {
            self.state.page = total_pages - 1;
        }

        let page_start = self.state.page * self.page_size;
        let page_end = (page_start + self.page_size).min(indices.len());
        let page_indices = &indices[page_start..page_end];

        let mut response = DataTableResponse {
            clicked_row: None,
            selected_rows: self.state.selected.clone(),
        };

        let frame = egui::Frame::NONE
            .fill(tokens.card)
            .stroke(if self.bordered {
                egui::Stroke::new(1.0, tokens.border)
            } else {
                egui::Stroke::NONE
            })
            .corner_radius(tokens.rounding_lg());

        frame.show(ui, |ui| {
            // Header
            ui.horizontal(|ui| {
                if self.selectable {
                    // Select all checkbox
                    let all_selected = !page_indices.is_empty()
                        && page_indices.iter().all(|&i| self.state.is_selected(i));
                    let mut checked = all_selected;

                    ui.allocate_ui(Vec2::new(32.0, row_height), |ui| {
                        ui.centered_and_justified(|ui| {
                            if ui.checkbox(&mut checked, "").changed() {
                                if checked {
                                    for &i in page_indices {
                                        self.state.select(i);
                                    }
                                } else {
                                    for &i in page_indices {
                                        self.state.deselect(i);
                                    }
                                }
                            }
                        });
                    });
                }

                for col in self.columns.iter() {
                    let width = col.width.unwrap_or(100.0);

                    let header_response = ui.allocate_ui(Vec2::new(width, row_height), |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(padding);

                            let is_sorted = self.state.sort_column.as_ref() == Some(&col.id);

                            let header_text = RichText::new(&col.header)
                                .size(tokens.font_size_sm)
                                .color(tokens.foreground)
                                .strong();

                            if col.sortable {
                                let btn_text = if is_sorted {
                                    format!("{} {}", col.header, self.state.sort_direction.icon())
                                } else {
                                    col.header.clone()
                                };

                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(btn_text)
                                                .size(tokens.font_size_sm)
                                                .strong(),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    if is_sorted {
                                        self.state.sort_direction =
                                            self.state.sort_direction.toggle();
                                        if self.state.sort_direction == SortDirection::None {
                                            self.state.sort_column = None;
                                        }
                                    } else {
                                        self.state.sort_column = Some(col.id.clone());
                                        self.state.sort_direction = SortDirection::Ascending;
                                    }
                                }
                            } else {
                                ui.label(header_text);
                            }
                        });
                    });
                }
            });

            // Header separator
            ui.add(egui::Separator::default().spacing(0.0));

            // Body
            if page_indices.is_empty() {
                ui.add_space(tokens.spacing_lg);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(self.empty_message)
                            .size(tokens.font_size_sm)
                            .color(tokens.muted_foreground),
                    );
                });
                ui.add_space(tokens.spacing_lg);
            } else {
                for (row_idx, &data_idx) in page_indices.iter().enumerate() {
                    let is_selected = self.state.is_selected(data_idx);
                    let is_striped = self.striped && row_idx % 2 == 1;

                    let bg_color = if is_selected {
                        tokens.accent
                    } else if is_striped {
                        tokens.muted.gamma_multiply(0.3)
                    } else {
                        egui::Color32::TRANSPARENT
                    };

                    let row_rect = ui.horizontal(|ui| {
                        let (rect, row_response) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), row_height),
                            egui::Sense::click(),
                        );

                        // Row background
                        if bg_color != egui::Color32::TRANSPARENT || row_response.hovered() {
                            let hover_color = if row_response.hovered() && self.hoverable {
                                tokens.accent.gamma_multiply(0.5)
                            } else {
                                bg_color
                            };
                            ui.painter().rect_filled(rect, 0.0, hover_color);
                        }

                        // Draw row content over the background
                        ui.allocate_ui_at_rect(rect, |ui| {
                            ui.horizontal(|ui| {
                                if self.selectable {
                                    let mut checked = is_selected;
                                    ui.allocate_ui(Vec2::new(32.0, row_height), |ui| {
                                        ui.centered_and_justified(|ui| {
                                            if ui.checkbox(&mut checked, "").changed() {
                                                self.state.toggle(data_idx);
                                            }
                                        });
                                    });
                                }

                                for col in self.columns.iter() {
                                    let width = col.width.unwrap_or(100.0);
                                    let value = (col.render)(&self.data[data_idx]);

                                    ui.allocate_ui(Vec2::new(width, row_height), |ui| {
                                        ui.horizontal(|ui| {
                                            ui.add_space(padding);

                                            let align = match col.align {
                                                ColumnAlign::Left => egui::Align::LEFT,
                                                ColumnAlign::Center => egui::Align::Center,
                                                ColumnAlign::Right => egui::Align::RIGHT,
                                            };

                                            ui.with_layout(
                                                egui::Layout::left_to_right(align),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(value)
                                                            .size(tokens.font_size_sm)
                                                            .color(tokens.foreground),
                                                    );
                                                },
                                            );
                                        });
                                    });
                                }
                            });
                        });

                        if row_response.clicked() {
                            response.clicked_row = Some(data_idx);
                            if self.selectable {
                                self.state.toggle(data_idx);
                            }
                        }
                    });
                }
            }

            // Pagination
            if self.show_pagination && total_pages > 1 {
                ui.add(egui::Separator::default().spacing(0.0));

                ui.horizontal(|ui| {
                    ui.add_space(tokens.spacing_md);

                    ui.label(
                        RichText::new(format!(
                            "Showing {} - {} of {}",
                            page_start + 1,
                            page_end,
                            indices.len()
                        ))
                        .size(tokens.font_size_xs)
                        .color(tokens.muted_foreground),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(tokens.spacing_md);

                        // Next button
                        if ui
                            .add_enabled(self.state.page < total_pages - 1, egui::Button::new("▶"))
                            .clicked()
                        {
                            self.state.page += 1;
                        }

                        // Page indicator
                        ui.label(
                            RichText::new(format!("{} / {}", self.state.page + 1, total_pages))
                                .size(tokens.font_size_sm),
                        );

                        // Previous button
                        if ui
                            .add_enabled(self.state.page > 0, egui::Button::new("◀"))
                            .clicked()
                        {
                            self.state.page -= 1;
                        }
                    });
                });

                ui.add_space(tokens.spacing_xs);
            }
        });

        response.selected_rows = self.state.selected.clone();
        response
    }
}

/// Response from data table
#[derive(Clone, Debug)]
pub struct DataTableResponse {
    /// Row that was clicked (data index)
    pub clicked_row: Option<usize>,
    /// Currently selected row indices
    pub selected_rows: Vec<usize>,
}
