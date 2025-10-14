//! Adaptive layout helpers for parameter widgets

use egui::{Response, Ui, Vec2};

/// Layout configuration for parameter widgets
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    /// Maximum width for a single widget
    pub max_width: Option<f32>,
    /// Minimum width for a single widget
    pub min_width: Option<f32>,
    /// Number of columns in grid layout
    pub columns: usize,
    /// Spacing between columns
    pub column_spacing: f32,
    /// Whether to use responsive columns
    pub responsive: bool,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            max_width: Some(400.0),  // Reasonable max width
            min_width: Some(250.0),  // Minimum usable width
            columns: 1,
            column_spacing: 16.0,
            responsive: true,
        }
    }
}

impl LayoutConfig {
    /// Create a single column layout
    pub fn single_column() -> Self {
        Self {
            columns: 1,
            responsive: true,
            ..Default::default()
        }
    }
    
    /// Create a two column layout
    pub fn two_columns() -> Self {
        Self {
            columns: 2,
            responsive: true,
            ..Default::default()
        }
    }
    
    /// Create a three column layout
    pub fn three_columns() -> Self {
        Self {
            columns: 3,
            responsive: true,
            ..Default::default()
        }
    }
    
    /// Create a responsive layout that adapts to available width
    pub fn responsive(max_columns: usize) -> Self {
        Self {
            columns: max_columns,
            responsive: true,
            ..Default::default()
        }
    }
    
    /// Calculate optimal number of columns based on available width
    pub fn calculate_columns(&self, available_width: f32) -> usize {
        if !self.responsive {
            return self.columns;
        }
        
        let min_width = self.min_width.unwrap_or(250.0);
        let spacing = self.column_spacing;
        
        // Calculate how many columns can fit
        let max_possible = ((available_width + spacing) / (min_width + spacing)).floor() as usize;
        
        // Use the smaller of desired columns and max possible
        self.columns.min(max_possible).max(1)
    }
    
    /// Calculate column width based on available space
    pub fn calculate_column_width(&self, available_width: f32, column_count: usize) -> f32 {
        let spacing = self.column_spacing * (column_count - 1) as f32;
        let content_width = available_width - spacing;
        let base_width = content_width / column_count as f32;
        
        // Apply min/max constraints
        let min_width = self.min_width.unwrap_or(0.0);
        let max_width = self.max_width.unwrap_or(f32::INFINITY);
        
        base_width.clamp(min_width, max_width)
    }
}

/// Container widget that automatically adapts to screen size
pub struct AdaptiveContainer<'a, F> {
    config: LayoutConfig,
    render_fn: F,
    _phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a, F> AdaptiveContainer<'a, F>
where
    F: FnMut(&mut Ui) -> Response,
{
    pub fn new(render_fn: F) -> Self {
        Self {
            config: LayoutConfig::default(),
            render_fn,
            _phantom: std::marker::PhantomData,
        }
    }
    
    pub fn with_config(mut self, config: LayoutConfig) -> Self {
        self.config = config;
        self
    }
    
    pub fn single_column(mut self) -> Self {
        self.config = LayoutConfig::single_column();
        self
    }
    
    pub fn two_columns(mut self) -> Self {
        self.config = LayoutConfig::two_columns();
        self
    }
    
    pub fn three_columns(mut self) -> Self {
        self.config = LayoutConfig::three_columns();
        self
    }
    
    pub fn responsive(mut self, max_columns: usize) -> Self {
        self.config = LayoutConfig::responsive(max_columns);
        self
    }
    
    pub fn max_width(mut self, width: f32) -> Self {
        self.config.max_width = Some(width);
        self
    }
    
    pub fn min_width(mut self, width: f32) -> Self {
        self.config.min_width = Some(width);
        self
    }
    
    pub fn show(mut self, ui: &mut Ui) -> Response {
        let available_width = ui.available_width();
        let actual_columns = self.config.calculate_columns(available_width);
        let column_width = self.config.calculate_column_width(available_width, actual_columns);
        
        // Create a frame with proper alignment (flex-start equivalent)
        let response = ui.allocate_ui(Vec2::new(column_width, 0.0), |ui| {
            (self.render_fn)(ui)
        });
        
        response.response
    }
}

/// Helper function to create an adaptive container
pub fn adaptive_container<F>(render_fn: F) -> AdaptiveContainer<'static, F>
where
    F: FnMut(&mut Ui) -> Response,
{
    AdaptiveContainer::new(render_fn)
}