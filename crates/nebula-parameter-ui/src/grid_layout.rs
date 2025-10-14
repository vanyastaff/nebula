//! Grid layout system for even widget placement

use egui::{Response, Ui, Grid, Vec2};

/// Grid layout configuration for even widget placement
#[derive(Debug, Clone)]
pub struct GridConfig {
    /// Number of columns
    pub columns: usize,
    /// Spacing between items
    pub spacing: f32,
    /// Whether to stretch items to fill available height
    pub stretch_height: bool,
    /// Maximum width for each item
    pub item_max_width: Option<f32>,
    /// Minimum width for each item
    pub item_min_width: Option<f32>,
}

impl Default for GridConfig {
    fn default() -> Self {
        Self {
            columns: 2,
            spacing: 16.0,
            stretch_height: true,
            item_max_width: Some(400.0),
            item_min_width: Some(280.0),
        }
    }
}

impl GridConfig {
    /// Create a 2-column grid with stretch height
    pub fn two_columns() -> Self {
        Self {
            columns: 2,
            stretch_height: true,
            ..Default::default()
        }
    }
    
    /// Create a 3-column grid with stretch height
    pub fn three_columns() -> Self {
        Self {
            columns: 3,
            stretch_height: true,
            ..Default::default()
        }
    }
    
    /// Set the gap between items
    pub fn gap(mut self, gap: f32) -> Self {
        self.spacing = gap;
        self
    }
    
    /// Set maximum width for items
    pub fn item_max_width(mut self, width: f32) -> Self {
        self.item_max_width = Some(width);
        self
    }
    
    /// Set minimum width for items
    pub fn item_min_width(mut self, width: f32) -> Self {
        self.item_min_width = Some(width);
        self
    }
    
    /// Enable or disable height stretching
    pub fn stretch_height(mut self, stretch: bool) -> Self {
        self.stretch_height = stretch;
        self
    }
}

/// Grid container that arranges items in even rows and columns
pub struct GridContainer<'a, F> {
    config: GridConfig,
    render_fn: F,
    _phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a, F> GridContainer<'a, F>
where
    F: FnMut(&mut Ui) -> Response,
{
    pub fn new(render_fn: F) -> Self {
        Self {
            config: GridConfig::default(),
            render_fn,
            _phantom: std::marker::PhantomData,
        }
    }
    
    pub fn with_config(mut self, config: GridConfig) -> Self {
        self.config = config;
        self
    }
    
    pub fn two_columns(mut self) -> Self {
        self.config = GridConfig::two_columns();
        self
    }
    
    pub fn three_columns(mut self) -> Self {
        self.config = GridConfig::three_columns();
        self
    }
    
    pub fn gap(mut self, gap: f32) -> Self {
        self.config.spacing = gap;
        self
    }
    
    pub fn item_max_width(mut self, width: f32) -> Self {
        self.config.item_max_width = Some(width);
        self
    }
    
    pub fn item_min_width(mut self, width: f32) -> Self {
        self.config.item_min_width = Some(width);
        self
    }
    
    pub fn stretch_height(mut self, stretch: bool) -> Self {
        self.config.stretch_height = stretch;
        self
    }
    
    pub fn show(mut self, ui: &mut Ui) -> Response {
        let available_width = ui.available_width();
        let item_width = self.calculate_item_width(available_width);
        
        // Calculate height for stretch mode
        let item_height = if self.config.stretch_height {
            self.calculate_stretch_height(ui)
        } else {
            0.0
        };
        
        // Create a grid with even spacing
        let grid_id = format!("parameter_grid_{}", std::ptr::addr_of!(self) as usize);
        let response = Grid::new(grid_id)
            .num_columns(self.config.columns)
            .spacing([self.config.spacing, self.config.spacing])
            .show(ui, |ui| {
                ui.allocate_ui(
                    Vec2::new(item_width, item_height),
                    |ui| {
                        (self.render_fn)(ui)
                    }
                ).response
            });
        
        response.response
    }
    
    fn calculate_item_width(&self, available_width: f32) -> f32 {
        let spacing = self.config.spacing * (self.config.columns - 1) as f32;
        let content_width = available_width - spacing;
        let base_width = content_width / self.config.columns as f32;
        
        let min_width = self.config.item_min_width.unwrap_or(200.0);
        let max_width = self.config.item_max_width.unwrap_or(f32::INFINITY);
        
        base_width.clamp(min_width, max_width)
    }
    
    fn calculate_stretch_height(&self, ui: &mut Ui) -> f32 {
        // Estimate height for even stretching
        let style = ui.style();
        let spacing = style.spacing.item_spacing.y;
        let frame_margin = (style.spacing.window_margin.top + style.spacing.window_margin.bottom) as f32;
        
        // Base height that works well for most widgets
        let base_height = 140.0;
        base_height + spacing + frame_margin
    }
}

/// Helper function to create a grid container
pub fn grid_container<F>(render_fn: F) -> GridContainer<'static, F>
where
    F: FnMut(&mut Ui) -> Response,
{
    GridContainer::new(render_fn)
}
