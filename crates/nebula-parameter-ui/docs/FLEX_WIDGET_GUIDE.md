# FlexWidget Integration Guide

## Overview

This guide explains how to use the `egui_flex::FlexWidget` trait with Nebula parameter widgets for more flexible and responsive layouts.

## Why Use FlexWidget?

### Traditional Approach (Basic egui)
```rust
ui.vertical(|ui| {
    ui.horizontal(|ui| {
        ui.label("Icon");
        ui.label("Content");
        ui.button("Action");
    });
});
```

**Problems:**
- ❌ Manual spacing calculations
- ❌ Hard to make responsive
- ❌ No automatic alignment
- ❌ Difficult to control growth/shrink behavior

### Modern Approach (egui-flex + FlexWidget)
```rust
Flex::horizontal()
    .gap(8.0)
    .align_items(FlexAlign::Center)
    .show(ui, |flex| {
        flex.add_ui(FlexItem::new().grow(0.0).basis(24.0), |ui| {
            ui.label("Icon");
        });
        flex.add_ui(FlexItem::new().grow(1.0), |ui| {
            ui.label("Content");
        });
        flex.add_ui(FlexItem::new().grow(0.0), |ui| {
            ui.button("Action");
        });
    });
```

**Benefits:**
- ✅ Automatic spacing with `gap()`
- ✅ Responsive by default
- ✅ CSS Flexbox-like alignment
- ✅ Explicit grow/shrink control
- ✅ Cleaner, more declarative code

## FlexWidget Trait

The `FlexWidget` trait allows widgets to define their own default flex properties and integrate seamlessly with flex containers.

```rust
pub trait FlexWidget {
    type Response;
    
    fn default_item() -> FlexItem<'static> {
        FlexItem::new()
    }
    
    fn flex_ui(self, item: FlexItem, flex_instance: &mut FlexInstance) -> Self::Response;
}
```

### Key Concepts

1. **`default_item()`** - Define sensible defaults for your widget
   - For notices: `grow(0.0)` (don't grow vertically)
   - For text inputs: `grow(1.0)` (fill available space)
   - For buttons: `grow(0.0).basis(auto)` (intrinsic size)

2. **`flex_ui()`** - Render the widget within a flex context
   - Access to `FlexInstance` for adding child items
   - Can override defaults via the `item` parameter

## Implementation Example: NoticeWidget

### 1. Import Required Types
```rust
use egui_flex::{Flex, FlexInstance, FlexItem, FlexWidget};
```

### 2. Implement FlexWidget
```rust
impl<'a> FlexWidget for NoticeWidget<'a> {
    type Response = Response;

    fn default_item() -> FlexItem<'static> {
        FlexItem::new()
            .grow(0.0)    // Don't grow vertically
            .basis(0.0)   // Use intrinsic size
    }

    fn flex_ui(mut self, item: FlexItem, flex_instance: &mut FlexInstance) -> Self::Response {
        let theme = ParameterTheme::default();
        
        flex_instance.add_ui(item, |ui| {
            self.render_with_theme(ui, &theme)
        })
    }
}
```

### 3. Usage Examples

#### Simple Vertical Stack
```rust
Flex::vertical()
    .gap(8.0)
    .show(ui, |flex| {
        let notice1 = NoticeWidget::new(NoticeParameter::info("Info message"));
        let notice2 = NoticeWidget::new(NoticeParameter::warning("Warning"));
        
        notice1.flex_ui(FlexItem::new(), flex);
        notice2.flex_ui(FlexItem::new(), flex);
    });
```

#### Mixed Layout with Custom Items
```rust
Flex::horizontal()
    .gap(12.0)
    .show(ui, |flex| {
        // Sidebar (fixed width)
        flex.add_ui(FlexItem::new().basis(200.0).grow(0.0), |ui| {
            ui.label("Sidebar");
        });
        
        // Notice (grows to fill)
        let notice = NoticeWidget::new(NoticeParameter::error("Error!"));
        notice.flex_ui(FlexItem::new().grow(1.0), flex);
    });
```

#### Responsive Layout
```rust
let direction = if ui.available_width() > 600.0 {
    Flex::horizontal()
} else {
    Flex::vertical()
};

direction
    .gap(8.0)
    .align_items(FlexAlign::Stretch)
    .show(ui, |flex| {
        for notice in notices {
            let widget = NoticeWidget::new(notice);
            widget.flex_ui(FlexItem::new().grow(1.0), flex);
        }
    });
```

## FlexItem Properties

### Common Properties

| Property | Description | Use Case |
|----------|-------------|----------|
| `grow(f32)` | How much to grow relative to siblings | `grow(1.0)` = fill space, `grow(0.0)` = don't grow |
| `shrink(f32)` | How much to shrink when space is limited | `shrink(1.0)` = can shrink, `shrink(0.0)` = keep size |
| `basis(f32)` | Initial size before growing/shrinking | `basis(200.0)` = start at 200px |
| `align_self(FlexAlign)` | Override container alignment | Center specific item |

### Widget-Specific Defaults

#### NoticeWidget
```rust
FlexItem::new()
    .grow(0.0)      // Don't grow vertically
    .basis(0.0)     // Use intrinsic height
```
**Rationale:** Notices should take only the space they need for their content.

#### TextWidget (Recommended)
```rust
FlexItem::new()
    .grow(1.0)      // Fill available horizontal space
    .basis(0.0)     // No minimum width
```
**Rationale:** Text inputs should expand to fill available width.

#### ListWidget (Recommended)
```rust
FlexItem::new()
    .grow(0.0)      // Don't grow beyond content
    .shrink(1.0)    // Can shrink if needed
```
**Rationale:** Lists have variable height based on items.

## Migration Guide

### Before (Traditional egui)
```rust
ui.vertical(|ui| {
    ui.horizontal(|ui| {
        ui.label("📌");
        ui.label("Important message");
        if ui.button("✖").clicked() {
            // dismiss
        }
    });
});
```

### After (egui-flex + FlexWidget)
```rust
Flex::horizontal()
    .w_full()
    .align_items(FlexAlign::Center)
    .show(ui, |flex| {
        // Icon (fixed)
        flex.add_ui(FlexItem::new().grow(0.0).basis(20.0), |ui| {
            ui.label("📌");
        });
        
        // Message (grows)
        flex.add_ui(FlexItem::new().grow(1.0), |ui| {
            ui.label("Important message");
        });
        
        // Button (fixed)
        flex.add_ui(FlexItem::new().grow(0.0), |ui| {
            if ui.button("✖").clicked() {
                // dismiss
            }
        });
    });
```

## Best Practices

### 1. Use Semantic Defaults
```rust
// Good: Clear intent
FlexItem::new().grow(0.0).basis(200.0)  // Fixed 200px width

// Bad: Magic numbers
FlexItem::new().grow(0.5).basis(137.0)  // Why 137?
```

### 2. Document Your Flex Defaults
```rust
impl FlexWidget for MyWidget {
    fn default_item() -> FlexItem<'static> {
        FlexItem::new()
            .grow(0.0)  // Don't grow - widget has intrinsic size
            .basis(0.0) // Use content size
    }
}
```

### 3. Test Responsive Behavior
```rust
// Test at different widths
if ui.available_width() > 800.0 {
    // Desktop layout
} else if ui.available_width() > 400.0 {
    // Tablet layout
} else {
    // Mobile layout
}
```

### 4. Use Appropriate Gaps
```rust
// Form layouts: smaller gaps
Flex::vertical().gap(4.0)

// Section separators: medium gaps
Flex::vertical().gap(12.0)

// Page sections: larger gaps
Flex::vertical().gap(24.0)
```

## Common Patterns

### Dashboard Layout
```rust
Flex::vertical().gap(16.0).show(ui, |flex| {
    // Header notices (fixed)
    for notice in header_notices {
        NoticeWidget::new(notice).flex_ui(FlexItem::new().grow(0.0), flex);
    }
    
    // Main content (grows)
    flex.add_ui(FlexItem::new().grow(1.0), |ui| {
        // Main app content
    });
    
    // Footer notices (fixed)
    for notice in footer_notices {
        NoticeWidget::new(notice).flex_ui(FlexItem::new().grow(0.0), flex);
    }
});
```

### Two-Column Form
```rust
Flex::horizontal().gap(16.0).show(ui, |flex| {
    // Left column
    flex.add_flex(
        FlexItem::new().grow(1.0),
        Flex::vertical().gap(8.0),
        |col1| {
            // Add form fields
        }
    );
    
    // Right column
    flex.add_flex(
        FlexItem::new().grow(1.0),
        Flex::vertical().gap(8.0),
        |col2| {
            // Add notices/help
        }
    );
});
```

## Performance Considerations

1. **FlexWidget is zero-cost** - It's a compile-time abstraction
2. **Flex layouts are cached** - egui caches layout calculations
3. **Use sparingly in hot paths** - Prefer for structure, not per-item

## Running the Example

```bash
cargo run --example notice_flex -p nebula-parameter-ui
```

## Next Steps

1. Implement `FlexWidget` for other widgets:
   - `ListWidget`
   - `GroupWidget`
   - `PanelWidget`
   - `ObjectWidget`

2. Create widget-specific flex utilities:
   ```rust
   // Helper for common notice layouts
   pub fn notice_stack() -> Flex {
       Flex::vertical()
           .gap(8.0)
           .align_items(FlexAlign::Stretch)
   }
   ```

3. Document flex patterns in widget examples

## References

- [egui-flex Reference](./EGUI_FLEX_REFERENCE.md) - **START HERE** - Полный справочник по egui-flex 0.5.0
- [egui-flex Documentation](https://docs.rs/egui-flex/0.5.0) - Официальная документация
- [CSS Flexbox Guide](https://css-tricks.com/snippets/css/a-guide-to-flexbox/) - CSS аналог
- [egui Layout Guide](https://docs.rs/egui/latest/egui/struct.Layout.html) - Базовые layouts в egui

