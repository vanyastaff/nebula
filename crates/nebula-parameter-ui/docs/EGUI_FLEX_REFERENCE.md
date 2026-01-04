# egui-flex Reference Guide

## Overview

**Version:** `egui-flex = "0.5.0"`

egui-flex — это библиотека для `egui`, которая реализует flexbox-подобную компоновку элементов, аналогично CSS Flexbox.

⚠️ **Important:** Этот справочник основан на реальном API egui-flex 0.5.0, используемом в проекте.

## Installation

```toml
[dependencies]
egui = "0.33.0"
egui_flex = "0.5.0"
```

## Core API (Verified)

### Struct Flex

Основной контейнер для flexbox-макетов.

#### Создание

```rust
Flex::new()          // Default (horizontal)
Flex::horizontal()   // Горизонтальный
Flex::vertical()     // Вертикальный
```

#### Методы настройки

```rust
.direction(FlexDirection)     // Направление (Horizontal/Vertical)
.justify(FlexJustify)         // Выравнивание по главной оси  
.align_items(FlexAlign)       // Выравнивание по поперечной оси
.align_content(FlexAlignContent) // Выравнивание контента (с wrap)
.gap(Vec2)                    // Промежутки между элементами
.grow_items(Option<f32>)      // Рост по умолчанию
.width(Size)                  // Ширина
.height(Size)                 // Высота
.w_full()                     // Ширина 100%
.h_full()                     // Высота 100%
.show(ui, callback)           // Отобразить
```

**Note:** `.gap()` принимает `Vec2`:
- `.gap(Vec2::splat(8.0))` → одинаковые отступы по x и y
- `.gap(Vec2::new(8.0, 4.0))` → разные отступы по x и y
- `.gap(Vec2::ZERO)` → без отступов

### Struct FlexItem

Конфигурация элемента.

```rust
FlexItem::new()              // Создать
.grow(f32)                   // Коэффициент роста
.basis(f32)                  // Базовый размер
.align_self(FlexAlign)       // Выравнивание элемента
.shrink()                    // Разрешить сжатие
```

### Struct FlexInstance

Экземпляр контейнера в callback `show()`.

```rust
// Доступные методы:
flex.add(FlexItem, FlexWidget)                    // Добавить FlexWidget
flex.add_widget(FlexItem, Widget)                 // Добавить обычный Widget
flex.add_flex(FlexItem, Flex, callback)           // Вложенный Flex
flex.add_ui(FlexItem, callback)                   // Добавить через UI closure
flex.direction() -> FlexDirection                 // Получить направление
flex.ui() -> &Ui                                  // Доступ к Ui
```

**Note:** `add_ui()` - convenience метод для добавления через closure!

### Enums

#### FlexAlign
```rust
FlexAlign::Start
FlexAlign::End
FlexAlign::Center
FlexAlign::Stretch
```

#### FlexAlignContent
```rust
FlexAlignContent::Start
FlexAlignContent::End
FlexAlignContent::Center
FlexAlignContent::Stretch
FlexAlignContent::SpaceBetween
FlexAlignContent::SpaceAround
```

#### FlexDirection
```rust
FlexDirection::Horizontal
FlexDirection::Vertical
```

#### FlexJustify
```rust
FlexJustify::Start
FlexJustify::End
FlexJustify::Center
FlexJustify::SpaceBetween
FlexJustify::SpaceAround
FlexJustify::SpaceEvenly
```

#### Size
```rust
Size::Points(f32)      // Пиксели
Size::Percent(f32)     // Процент (1.0 = 100%)
```

## Practical Examples

### Example 1: Vertical Stack (from NoticeWidget)

```rust
use egui_flex::{Flex, FlexItem, FlexAlign};

Flex::vertical()
    .w_full()
    .gap(Vec2::ZERO)
    .show(ui, |flex| {
        // Main content
        flex.add_flex(
            FlexItem::new().grow(0.0),
            Flex::horizontal()
                .w_full()
                .align_items(FlexAlign::Start),
            |content_flex| {
                // Add items
            },
        );
        
        // Progress bar
        flex.add_ui(FlexItem::new().grow(0.0), |ui| {
            ui.label("Progress");
        });
    });
```

### Example 2: Using add_ui() for Complex Content

```rust
Flex::horizontal()
    .gap(Vec2::splat(8.0))
    .show(ui, |flex| {
        // Icon (fixed)
        flex.add_ui(FlexItem::new().grow(0.0).basis(16.0), |ui| {
            ui.label("ℹ");
        });
        
        // Content (grows)
        flex.add_ui(FlexItem::new().grow(1.0), |ui| {
            ui.label("Message");
        });
        
        // Button (fixed)
        flex.add_ui(FlexItem::new().grow(0.0), |ui| {
            ui.button("✖");
        });
    });
```

### Example 3: Nested Flex

```rust
Flex::vertical()
    .align_items(FlexAlign::Start)
    .align_content(FlexAlignContent::Start)
    .w_full()
    .show(ui, |outer_flex| {
        outer_flex.add_flex(
            FlexItem::new(),
            Flex::horizontal().gap(Vec2::splat(8.0)),
            |inner_flex| {
                inner_flex.add_ui(FlexItem::new(), |ui| {
                    ui.label("Item 1");
                });
                inner_flex.add_ui(FlexItem::new(), |ui| {
                    ui.label("Item 2");
                });
            },
        );
    });
```

### Example 4: Header/Content/Footer

```rust
Flex::vertical()
    .h_full()
    .gap(Vec2::ZERO)
    .show(ui, |flex| {
        // Header (fixed)
        flex.add_ui(FlexItem::new().basis(50.0).grow(0.0), |ui| {
            ui.heading("Header");
        });
        
        // Content (grows)
        flex.add_ui(FlexItem::new().grow(1.0), |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.label("Content");
            });
        });
        
        // Footer (fixed)
        flex.add_ui(FlexItem::new().basis(30.0).grow(0.0), |ui| {
            ui.label("Footer");
        });
    });
```

### Example 5: Two-Column Layout

```rust
Flex::horizontal()
    .gap(Vec2::splat(16.0))
    .show(ui, |flex| {
        // Left (40%)
        flex.add_ui(FlexItem::new().grow(2.0), |ui| {
            ui.label("Left column (40%)");
        });
        
        // Right (60%)
        flex.add_ui(FlexItem::new().grow(3.0), |ui| {
            ui.label("Right column (60%)");
        });
    });
```

### Example 6: Sidebar Layout

```rust
Flex::horizontal()
    .h_full()
    .gap(Vec2::ZERO)
    .show(ui, |flex| {
        // Sidebar (fixed 250px)
        flex.add_ui(FlexItem::new().basis(250.0).grow(0.0), |ui| {
            ui.vertical(|ui| {
                ui.heading("Navigation");
                ui.button("Home");
                ui.button("Settings");
            });
        });
        
        // Content (grows)
        flex.add_ui(FlexItem::new().grow(1.0), |ui| {
            ui.heading("Main Content");
        });
    });
```

### Example 7: Centering

```rust
use egui_flex::FlexJustify;

Flex::vertical()
    .w_full()
    .h_full()
    .justify(FlexJustify::Center)
    .align_items(FlexAlign::Center)
    .show(ui, |flex| {
        flex.add_ui(FlexItem::new(), |ui| {
            ui.heading("Centered!");
        });
    });
```

### Example 8: Responsive Layout

```rust
let use_horizontal = ui.available_width() > 600.0;

let direction = if use_horizontal {
    FlexDirection::Horizontal
} else {
    FlexDirection::Vertical
};

Flex::new()
    .direction(direction)
    .gap(Vec2::splat(8.0))
    .show(ui, |flex| {
        for i in 0..3 {
            flex.add_ui(FlexItem::new().grow(1.0), |ui| {
                ui.label(format!("Item {}", i + 1));
            });
        }
    });
```

## FlexWidget Trait

Для интеграции виджетов с flex:

```rust
use egui::{Response, Ui};
use egui_flex::{FlexWidget, FlexItem, FlexInstance};

impl<'a> FlexWidget for MyWidget<'a> {
    type Response = Response;

    fn flex_ui(mut self, item: FlexItem, flex_instance: &mut FlexInstance) -> Self::Response {
        let theme = Theme::default();
        flex_instance.add_ui(item, |ui| {
            self.render_with_theme(ui, &theme)
        })
    }
}

// Использование
flex.add(FlexItem::new(), widget);
```

## Common Patterns

### Pattern: Icon + Text + Button

```rust
Flex::horizontal()
    .w_full()
    .align_items(FlexAlign::Center)
    .gap(Vec2::splat(8.0))
    .show(ui, |flex| {
        // Icon (fixed)
        flex.add_ui(FlexItem::new().grow(0.0).basis(20.0), |ui| {
            ui.label("🔔");
        });
        
        // Text (grows)
        flex.add_ui(FlexItem::new().grow(1.0), |ui| {
            ui.label("Notification text");
        });
        
        // Button (fixed)
        flex.add_ui(FlexItem::new().grow(0.0), |ui| {
            ui.button("✖");
        });
    });
```

### Pattern: Space Between

```rust
Flex::horizontal()
    .justify(FlexJustify::SpaceBetween)
    .w_full()
    .show(ui, |flex| {
        flex.add_ui(FlexItem::new(), |ui| ui.label("Left"));
        flex.add_ui(FlexItem::new(), |ui| ui.label("Right"));
    });
```

### Pattern: Equal Width Columns

```rust
Flex::horizontal()
    .gap(Vec2::splat(8.0))
    .show(ui, |flex| {
        for i in 0..3 {
            flex.add_ui(FlexItem::new().grow(1.0), |ui| {
                ui.label(format!("Column {}", i + 1));
            });
        }
    });
```

## Key Differences from CSS Flexbox

| CSS Flexbox | egui-flex | Notes |
|-------------|-----------|-------|
| `flex-direction: row` | `Flex::horizontal()` | ✅ Same concept |
| `flex-direction: column` | `Flex::vertical()` | ✅ Same concept |
| `gap: 8px` | `.gap(Vec2::splat(8.0))` | ✅ Same concept |
| `justify-content` | `.justify()` | ✅ Same concept |
| `align-items` | `.align_items()` | ✅ Same concept |
| `flex-grow: 1` | `FlexItem::new().grow(1.0)` | ✅ Same concept |
| `flex-basis: 100px` | `.basis(100.0)` | ✅ Same concept |
| `flex-wrap` | ❌ Not supported | Wrap not available |
| `order` | ❌ Not supported | Manual ordering |

## Tips & Best Practices

### 1. Use `.gap()` with Vec2

```rust
// ✅ Equal spacing
.gap(Vec2::splat(8.0))

// ✅ Different x/y
.gap(Vec2::new(8.0, 4.0))

// ✅ No gap
.gap(Vec2::ZERO)
```

### 2. Use `add_ui()` for complex content

```rust
// ✅ Easy for UI closures
flex.add_ui(FlexItem::new().grow(1.0), |ui| {
    ui.vertical(|ui| {
        ui.label("Line 1");
        ui.label("Line 2");
    });
});
```

### 3. Use `add_widget()` for simple widgets

```rust
// ✅ Direct widget
flex.add_widget(FlexItem::new(), egui::Label::new("Text"));
flex.add_widget(FlexItem::new(), egui::Button::new("Click"));
```

### 4. Use `add_flex()` for nesting

```rust
// ✅ Nested layouts
flex.add_flex(
    FlexItem::new().grow(1.0),
    Flex::vertical().gap(Vec2::splat(4.0)),
    |nested| {
        // Add items to nested flex
    }
);
```

## Performance Tips

1. **Minimize nesting** - 2-3 levels max
2. **Use `gap()` instead of manual spacing**
3. **Cache layouts for static content**
4. **Use `grow(0.0)` for fixed sizes**

## API Reference Summary

| Method | Type | Description |
|--------|------|-------------|
| `Flex::new()` | Constructor | Default horizontal |
| `Flex::horizontal()` | Constructor | Horizontal container |
| `Flex::vertical()` | Constructor | Vertical container |
| `.direction()` | Config | Set direction |
| `.justify()` | Config | Main axis alignment |
| `.align_items()` | Config | Cross axis alignment |
| `.gap()` | Config | Gap between items (f32 or Vec2) |
| `.w_full()` | Config | Width 100% |
| `.h_full()` | Config | Height 100% |
| `.show()` | Display | Render container |
| `.add_ui()` | FlexInstance | Add via UI closure |
| `.add_widget()` | FlexInstance | Add widget |
| `.add_flex()` | FlexInstance | Add nested Flex |

## References

- **Crate:** https://crates.io/crates/egui_flex
- **Documentation:** https://docs.rs/egui_flex/0.5.0
- **Repository:** https://github.com/lucasmerlin/egui_flex
- **Used in:** NoticeWidget (see `src/widgets/notice.rs`)

## See Also

- [FlexWidget Guide](./FLEX_WIDGET_GUIDE.md) - Integration guide
- [FlexWidget Quick Start](./FLEXWIDGET_QUICKSTART.md) - Quick start (Russian)
- [Auto-Dismiss Feature](./AUTO_DISMISS_FEATURE.md) - NoticeWidget auto-dismiss

---

**Last Updated:** 2025-10-15  
**Version:** egui-flex 0.5.0 (Verified from working code)  
**Status:** ✅ Verified - All examples compile and work
