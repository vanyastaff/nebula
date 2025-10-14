//! Flex-like layout system for parameter widgets

use egui::{Response, Ui, Vec2, Layout, Align};

/// Flex layout configuration similar to CSS Flexbox
#[derive(Debug, Clone)]
pub struct FlexConfig {
    /// Direction of the flex container
    pub direction: FlexDirection,
    /// Alignment along the main axis
    pub justify_content: JustifyContent,
    /// Alignment along the cross axis (like align-items)
    pub align_items: AlignItems,
    /// Gap between items
    pub gap: f32,
    /// Whether items should wrap to new lines
    pub wrap: bool,
    /// Maximum width for each item
    pub item_max_width: Option<f32>,
    /// Minimum width for each item
    pub item_min_width: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlexDirection {
    Row,
    Column,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JustifyContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
}

impl Default for FlexConfig {
    fn default() -> Self {
        Self {
            direction: FlexDirection::Row,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::FlexStart,
            gap: 16.0,
            wrap: true,
            item_max_width: Some(400.0),
            item_min_width: Some(250.0),
        }
    }
}

impl FlexConfig {
    /// Create a row layout with flex-start alignment
    pub fn row_flex_start() -> Self {
        Self {
            direction: FlexDirection::Row,
            align_items: AlignItems::FlexStart,
            ..Default::default()
        }
    }
    
    /// Create a column layout with flex-start alignment
    pub fn column_flex_start() -> Self {
        Self {
            direction: FlexDirection::Column,
            align_items: AlignItems::FlexStart,
            ..Default::default()
        }
    }
    
    /// Set the gap between items
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
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
    
    /// Enable or disable wrapping
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }
}

/// Flex container that arranges items like CSS Flexbox
pub struct FlexContainer<'a, F> {
    config: FlexConfig,
    render_fn: F,
    _phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a, F> FlexContainer<'a, F>
where
    F: FnMut(&mut Ui) -> Response,
{
    pub fn new(render_fn: F) -> Self {
        Self {
            config: FlexConfig::default(),
            render_fn,
            _phantom: std::marker::PhantomData,
        }
    }
    
    pub fn with_config(mut self, config: FlexConfig) -> Self {
        self.config = config;
        self
    }
    
    pub fn row_flex_start(mut self) -> Self {
        self.config = FlexConfig::row_flex_start();
        self
    }
    
    pub fn column_flex_start(mut self) -> Self {
        self.config = FlexConfig::column_flex_start();
        self
    }
    
    pub fn gap(mut self, gap: f32) -> Self {
        self.config.gap = gap;
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
    
    pub fn show(mut self, ui: &mut Ui) -> Response {
        let available_width = ui.available_width();
        let item_width = self.calculate_item_width(available_width);
        
        // For stretch alignment, we need to calculate the height of the tallest item
        let min_height = if self.config.align_items == AlignItems::Stretch {
            self.calculate_stretch_height(ui)
        } else {
            0.0
        };
        
        // Determine layout based on direction and alignment
        let layout = match self.config.direction {
            FlexDirection::Row => {
                if self.config.align_items == AlignItems::FlexStart {
                    Layout::left_to_right(Align::LEFT)
                } else if self.config.align_items == AlignItems::FlexEnd {
                    Layout::left_to_right(Align::RIGHT)
                } else if self.config.align_items == AlignItems::Stretch {
                    Layout::left_to_right(Align::LEFT)
                } else {
                    Layout::left_to_right(Align::Center)
                }
            }
            FlexDirection::Column => {
                if self.config.align_items == AlignItems::FlexStart {
                    Layout::top_down(Align::LEFT)
                } else if self.config.align_items == AlignItems::FlexEnd {
                    Layout::top_down(Align::RIGHT)
                } else if self.config.align_items == AlignItems::Stretch {
                    Layout::top_down(Align::LEFT)
                } else {
                    Layout::top_down(Align::Center)
                }
            }
        };
        
        let response = ui.allocate_ui_with_layout(
            Vec2::new(item_width, min_height),
            layout,
            |ui| {
                if self.config.align_items == AlignItems::Stretch && min_height > 0.0 {
                    // Create a frame that fills the allocated height
                    ui.allocate_ui(Vec2::new(item_width, min_height), |ui| {
                        (self.render_fn)(ui)
                    }).response
                } else {
                    (self.render_fn)(ui)
                }
            }
        );
        
        response.response
    }
    
    fn calculate_item_width(&self, available_width: f32) -> f32 {
        let min_width = self.config.item_min_width.unwrap_or(200.0);
        let max_width = self.config.item_max_width.unwrap_or(f32::INFINITY);
        
        // For now, use a simple calculation
        // In a more complex implementation, this would consider the number of items
        let base_width = (available_width - self.config.gap) / 2.0; // Assume 2 items per row
        base_width.clamp(min_width, max_width)
    }
    
    fn calculate_stretch_height(&self, ui: &mut Ui) -> f32 {
        // For stretch alignment, we need to estimate the height
        // This is a simplified approach - in a real implementation,
        // we would measure the actual heights of all items in the row
        let style = ui.style();
        let spacing = style.spacing.item_spacing.y;
        let frame_margin = style.spacing.window_margin.top + style.spacing.window_margin.bottom;
        
        // Estimate height based on typical widget content
        // This includes: label, input field, description, hints, validation messages
        let estimated_height = 120.0; // Base height for most widgets
        estimated_height + spacing + frame_margin
    }
}

/// Helper function to create a flex container
pub fn flex_container<F>(render_fn: F) -> FlexContainer<'static, F>
where
    F: FnMut(&mut Ui) -> Response,
{
    FlexContainer::new(render_fn)
}
