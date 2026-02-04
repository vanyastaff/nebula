//! Specialized color palettes for data types and node categories.

use egui::Color32;

/// Colors for different data types (used in pins and connections)
#[derive(Clone, Debug)]
pub struct DataTypeColors {
    /// Execution/control flow (white/light)
    pub execution: Color32,
    /// String type (pink/magenta)
    pub string: Color32,
    /// Number type (cyan/teal)
    pub number: Color32,
    /// Boolean type (red)
    pub boolean: Color32,
    /// Array type (blue)
    pub array: Color32,
    /// Object type (yellow/gold)
    pub object: Color32,
    /// Generic/any type (gray)
    pub generic: Color32,
    /// Struct type (orange)
    pub structure: Color32,
    /// Bytes/binary type (purple)
    pub bytes: Color32,
}

impl Default for DataTypeColors {
    fn default() -> Self {
        Self {
            execution: Color32::from_rgb(255, 255, 255), // White
            string: Color32::from_rgb(236, 72, 153),     // Pink-500
            number: Color32::from_rgb(34, 211, 238),     // Cyan-400
            boolean: Color32::from_rgb(239, 68, 68),     // Red-500
            array: Color32::from_rgb(59, 130, 246),      // Blue-500
            object: Color32::from_rgb(250, 204, 21),     // Yellow-400
            generic: Color32::from_rgb(161, 161, 170),   // Zinc-400
            structure: Color32::from_rgb(249, 115, 22),  // Orange-500
            bytes: Color32::from_rgb(168, 85, 247),      // Purple-500
        }
    }
}

impl DataTypeColors {
    /// Create a high-contrast variant for light themes
    #[must_use]
    pub fn high_contrast() -> Self {
        Self {
            execution: Color32::from_rgb(50, 50, 50),  // Dark gray
            string: Color32::from_rgb(190, 24, 93),    // Pink-700
            number: Color32::from_rgb(8, 145, 178),    // Cyan-600
            boolean: Color32::from_rgb(185, 28, 28),   // Red-700
            array: Color32::from_rgb(29, 78, 216),     // Blue-700
            object: Color32::from_rgb(161, 98, 7),     // Yellow-700
            generic: Color32::from_rgb(82, 82, 91),    // Zinc-600
            structure: Color32::from_rgb(194, 65, 12), // Orange-700
            bytes: Color32::from_rgb(126, 34, 206),    // Purple-700
        }
    }

    /// Get a dimmed version of a color (for unconnected pins)
    #[must_use]
    pub fn dimmed(color: Color32, factor: f32) -> Color32 {
        Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            (f32::from(color.a()) * factor) as u8,
        )
    }
}

/// Colors for different node categories
#[derive(Clone, Debug)]
pub struct NodeCategoryColors {
    /// Control flow nodes (if, loop, branch) - Purple
    pub control_flow: Color32,
    /// Data nodes (get, set, transform) - Green
    pub data: Color32,
    /// I/O nodes (file, http, database) - Blue
    pub io: Color32,
    /// AI/ML nodes (llm, embedding) - Pink
    pub ai: Color32,
    /// Utility nodes (log, debug, comment) - Gray
    pub utility: Color32,
    /// Event nodes (trigger, webhook) - Orange
    pub event: Color32,
    /// Custom/user-defined nodes - Indigo
    pub custom: Color32,
}

impl Default for NodeCategoryColors {
    fn default() -> Self {
        Self {
            control_flow: Color32::from_rgb(168, 85, 247), // Purple-500
            data: Color32::from_rgb(34, 197, 94),          // Green-500
            io: Color32::from_rgb(59, 130, 246),           // Blue-500
            ai: Color32::from_rgb(236, 72, 153),           // Pink-500
            utility: Color32::from_rgb(113, 113, 122),     // Zinc-500
            event: Color32::from_rgb(249, 115, 22),        // Orange-500
            custom: Color32::from_rgb(99, 102, 241),       // Indigo-500
        }
    }
}

impl NodeCategoryColors {
    /// Create a muted variant (for backgrounds)
    #[must_use]
    pub fn muted(&self) -> Self {
        Self {
            control_flow: Self::with_alpha(self.control_flow, 40),
            data: Self::with_alpha(self.data, 40),
            io: Self::with_alpha(self.io, 40),
            ai: Self::with_alpha(self.ai, 40),
            utility: Self::with_alpha(self.utility, 40),
            event: Self::with_alpha(self.event, 40),
            custom: Self::with_alpha(self.custom, 40),
        }
    }

    fn with_alpha(color: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
    }
}

/// Execution state colors (for visual feedback during flow execution)
#[derive(Clone, Debug)]
pub struct ExecutionStateColors {
    /// Idle/not running
    pub idle: Color32,
    /// Currently executing
    pub running: Color32,
    /// Completed successfully
    pub success: Color32,
    /// Failed with error
    pub error: Color32,
    /// Waiting/paused
    pub waiting: Color32,
    /// Skipped (branch not taken)
    pub skipped: Color32,
}

impl Default for ExecutionStateColors {
    fn default() -> Self {
        Self {
            idle: Color32::from_rgb(113, 113, 122),    // Zinc-500
            running: Color32::from_rgb(59, 130, 246),  // Blue-500
            success: Color32::from_rgb(34, 197, 94),   // Green-500
            error: Color32::from_rgb(239, 68, 68),     // Red-500
            waiting: Color32::from_rgb(234, 179, 8),   // Yellow-500
            skipped: Color32::from_rgb(161, 161, 170), // Zinc-400
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_type_colors_default() {
        let colors = DataTypeColors::default();
        // Execution should be white-ish
        assert!(colors.execution.r() > 200);
        // Generic should be gray-ish
        assert!(colors.generic.r() > 100 && colors.generic.r() < 200);
    }

    #[test]
    fn test_dimmed_color() {
        let color = Color32::from_rgb(255, 0, 0);
        let dimmed = DataTypeColors::dimmed(color, 0.5);
        assert_eq!(dimmed.a(), 127);
    }

    #[test]
    fn test_node_category_muted() {
        let colors = NodeCategoryColors::default();
        let muted = colors.muted();
        // Muted colors should have low alpha
        assert!(muted.control_flow.a() < 50);
    }
}
