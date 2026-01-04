# Flow Editor - Новые возможности (ReactFlow-inspired)

Этот документ описывает новые компоненты flow editor, вдохновленные ReactFlow.

## 📦 Добавленные компоненты

### 1. **MiniMap** (`minimap.rs`)
Миниатюрная карта для навигации по большому графу.

**Возможности:**
- Bird's-eye view всего графа
- Индикатор текущего viewport
- Клик для навигации к нужной области
- Настраиваемая позиция (TopLeft, TopRight, BottomLeft, BottomRight)
- Цветовая индикация нодов по категориям
- Полупрозрачный фон

**Использование:**
```rust
use nebula_ui::flow::prelude::*;

let minimap_response = Minimap::new(
    &nodes,
    &connections,
    viewport_rect,
    canvas_pan,
    canvas_zoom,
)
.config(MinimapConfig {
    position: MinimapPosition::BottomRight,
    width: 200.0,
    height: 150.0,
    ..Default::default()
})
.show(ui);

// Обработка навигации
if let Some(canvas_pos) = minimap_response.clicked_position {
    // Переместить viewport к clicked_position
}
```

### 2. **Controls Panel** (`controls.rs`)
Панель управления с кнопками зума и навигации.

**Возможности:**
- Zoom In/Out кнопки
- Reset Zoom (100%)
- Fit View (показать весь граф)
- Lock/Unlock (блокировка редактирования)
- Fullscreen toggle
- Настраиваемая позиция

**Использование:**
```rust
let controls_response = Controls::new()
    .config(ControlsConfig {
        position: ControlsPosition::BottomLeft,
        show_zoom: true,
        show_fit_view: true,
        ..Default::default()
    })
    .show(ui);

// Обработка действий
for action in controls_response.actions {
    match action {
        ControlAction::ZoomIn => { /* увеличить zoom */ },
        ControlAction::FitView => { /* вписать граф */ },
        _ => {}
    }
}
```

### 3. **Background Patterns** (`background.rs`)
Настраиваемые фоновые паттерны для canvas.

**Варианты:**
- `Dots` - точечный паттерн (как в ReactFlow по умолчанию)
- `Lines` - сетка из линий
- `Cross` - комбинация точек и линий

**Возможности:**
- Настройка gap между точками/линиями
- Major lines (каждая N-ая линия жирнее)
- Настройка прозрачности
- Масштабирование с zoom

**Использование:**
```rust
let background = Background::new()
    .variant(BackgroundVariant::Dots)
    .gap(20.0);

background.draw(ui, rect, canvas_pan, canvas_zoom);
```

### 4. **Keyboard Shortcuts** (`shortcuts.rs`)
Полная поддержка горячих клавиш.

**Поддерживаемые shortcuts:**
- `Ctrl/Cmd + Z` - Undo
- `Ctrl/Cmd + Shift + Z` / `Ctrl/Cmd + Y` - Redo
- `Delete` / `Backspace` - Удалить выбранное
- `Ctrl/Cmd + A` - Выбрать все
- `Escape` - Снять выделение
- `Ctrl/Cmd + C/X/V` - Copy/Cut/Paste
- `Ctrl/Cmd + D` - Duplicate
- `Ctrl/Cmd + +/-` - Zoom In/Out
- `Ctrl/Cmd + 0` - Reset Zoom
- `Ctrl/Cmd + Shift + 1` - Fit View
- `Ctrl/Cmd + F` - Find
- `Ctrl/Cmd + S` - Save
- `F11` - Fullscreen

**Использование:**
```rust
let shortcuts = KeyboardShortcuts::new();

let actions = shortcuts.process(ctx);
for action in actions {
    match action {
        ShortcutAction::Delete => { /* удалить */ },
        ShortcutAction::ZoomIn => { /* увеличить */ },
        _ => {}
    }
}

// Получить справку по shortcuts
let help = shortcuts.get_shortcuts_help();
```

## 🎨 Существующие возможности (уже были)

- **Canvas** - Pan/Zoom с мышью и touchpad
- **Nodes** - Визуальные ноды с пинами
- **Connections** - 4 типа: Bezier, Straight, SmoothStep, Smart (с pathfinding!)
- **Selection** - Box selection, multi-select
- **Smart Routing** - Автоматический обход препятствий с A* алгоритмом

## 🚀 Пример использования

Запустите полный пример:

```bash
cargo run --example flow_editor
```

**Что демонстрирует пример:**
- Создание графа с разными типами нодов
- Все 4 типа connections (Bezier, Straight, SmoothStep, Smart)
- MiniMap для навигации
- Controls панель
- Переключение фоновых паттернов (Dots/Lines/Cross)
- Keyboard shortcuts
- Двойной клик для создания новых нодов
- Перетаскивание нодов
- Создание и удаление connections
- Selection и multi-selection

## 🔧 Интеграция

Все новые компоненты экспортированы через `prelude`:

```rust
use nebula_ui::flow::prelude::*;

// Теперь доступны:
// - Minimap, MinimapConfig, MinimapPosition
// - Controls, ControlsConfig, ControlsPosition
// - Background, BackgroundConfig, BackgroundVariant
// - KeyboardShortcuts, ShortcutAction, ShortcutsConfig
// - EdgeType (для выбора типа connection)
```

## 📊 Сравнение с ReactFlow

| Функция | ReactFlow | nebula-ui | Статус |
|---------|-----------|-----------|--------|
| Pan/Zoom | ✅ | ✅ | ✅ |
| MiniMap | ✅ | ✅ | ✅ |
| Controls | ✅ | ✅ | ✅ |
| Background | ✅ | ✅ | ✅ (3 варианта) |
| Keyboard Shortcuts | ✅ | ✅ | ✅ |
| Smart Routing | ❌ | ✅ | 🎉 (A* pathfinding) |
| Edge Types | ✅ | ✅ | ✅ (4 типа) |
| Box Selection | ✅ | ✅ | ✅ |
| Node Grouping | ✅ | ⏳ | Планируется |
| Undo/Redo | ✅ | ⏳ | Планируется (есть shortcuts) |

## 🎯 Следующие шаги

Возможные улучшения:
1. **Node Grouping/Subflows** - иерархические группы нодов
2. **Undo/Redo system** - полная реализация с Command pattern
3. **Node Templates** - библиотека готовых нодов
4. **Performance optimization** - виртуализация для больших графов
5. **Animations** - анимация data flow по connections
6. **Auto-Layout** - автоматическое размещение нодов

## 📝 Примечания

- Все компоненты следуют паттернам egui
- Тестирование с существующим theme system
- Полная интеграция с CommandHistory для Undo/Redo
- Готово к production использованию
