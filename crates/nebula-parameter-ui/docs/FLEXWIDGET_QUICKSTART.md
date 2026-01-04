# FlexWidget Quick Start

## TL;DR

`NoticeWidget` теперь поддерживает `FlexWidget` trait для современных, responsive макетов! 🎉

## Что это дает?

### До (традиционный подход)
```rust
ui.vertical(|ui| {
    ui.horizontal(|ui| {
        ui.label("ℹ");
        ui.label("Сообщение");
        ui.button("✖");
    });
});
```
**Проблемы:** Ручные отступы, сложное выравнивание, не responsive

### После (с FlexWidget)
```rust
use nebula_parameter_ui::{Flex, FlexItem, FlexWidget};

Flex::horizontal()
    .gap(8.0)
    .align_items(FlexAlign::Center)
    .show(ui, |flex| {
        let widget = NoticeWidget::new(notice);
        widget.flex_ui(FlexItem::new().grow(0.0), flex);
    });
```
**Преимущества:** Автоматические отступы, CSS-like API, responsive из коробки

## Быстрый старт

### 1. Импорты
```rust
use nebula_parameter_ui::{
    NoticeWidget,
    Flex,           // Flex контейнер
    FlexItem,       // Настройки элемента
    FlexAlign,      // Выравнивание
    FlexWidget,     // Trait
};
```

### 2. Создай виджет
```rust
let notice = NoticeParameter::info("Система обновлена");
let widget = NoticeWidget::new(notice);
```

### 3. Добавь в Flex контейнер
```rust
Flex::vertical().gap(8.0).show(ui, |flex| {
    widget.flex_ui(FlexItem::new().grow(0.0), flex);
});
```

## Запусти примеры

### Flex Layout
```bash
cargo run --example notice_flex -p nebula-parameter-ui
```

Пример показывает:
- ✅ Вертикальные стеки уведомлений
- ✅ Горизонтальные макеты
- ✅ Responsive поведение
- ✅ Nested flex контейнеры

### Auto-Dismiss
```bash
cargo run --example notice_auto_dismiss -p nebula-parameter-ui
```

Пример показывает:
- ✅ Автоматическое закрытие через заданное время
- ✅ Визуальный прогресс бар
- ✅ Ручное закрытие
- ✅ Разные типы уведомлений

## FlexItem свойства

| Свойство | Описание | Пример |
|----------|----------|--------|
| `grow(f32)` | Насколько элемент растет | `grow(1.0)` = заполнить пространство |
| `shrink(f32)` | Насколько элемент сжимается | `shrink(0.0)` = не сжимать |
| `basis(f32)` | Начальный размер | `basis(200.0)` = 200px |
| `align_self()` | Индивидуальное выравнивание | `align_self(FlexAlign::Center)` |

## Рекомендации для NoticeWidget

```rust
FlexItem::new()
    .grow(0.0)    // Не растягивать - у уведомления фиксированная высота
    .basis(0.0)   // Использовать размер контента
```

## Популярные паттерны

### Вертикальный стек
```rust
Flex::vertical()
    .gap(8.0)
    .show(ui, |flex| {
        widget1.flex_ui(FlexItem::new(), flex);
        widget2.flex_ui(FlexItem::new(), flex);
    });
```

### Sidebar + Content
```rust
Flex::horizontal()
    .gap(12.0)
    .show(ui, |flex| {
        // Sidebar (фиксированная ширина)
        flex.add_ui(FlexItem::new().basis(200.0).grow(0.0), |ui| {
            ui.label("Sidebar");
        });
        
        // Уведомление (растет)
        widget.flex_ui(FlexItem::new().grow(1.0), flex);
    });
```

### Responsive layout
```rust
let flex = if ui.available_width() > 600.0 {
    Flex::horizontal()
} else {
    Flex::vertical()
};

flex.gap(8.0).show(ui, |flex| {
    for widget in widgets {
        widget.flex_ui(FlexItem::new().grow(1.0), flex);
    }
});
```

## Следующие шаги

### Для пользователей
1. ✅ Попробуй пример: `cargo run --example notice_flex`
2. ✅ Прочитай [FlexWidget Guide](./FLEX_WIDGET_GUIDE.md) для деталей
3. ✅ Используй в своих проектах!

### Для разработчиков виджетов
1. ✅ Изучи реализацию в [`notice.rs`](../src/widgets/notice.rs)
2. ✅ Прочитай [Implementation Summary](./FLEXWIDGET_IMPLEMENTATION_SUMMARY.md)
3. ✅ Посмотри [TODO](./FLEXWIDGET_TODO.md) для других виджетов

## Совместимость

✅ **100% обратно совместимо**
- Старый код продолжает работать
- `ParameterWidget::render()` не изменился
- FlexWidget - опциональное расширение

## Когда использовать FlexWidget?

### ✅ Используй FlexWidget когда:
- Строишь сложные макеты
- Нужен responsive UI
- Хочешь декларативный код
- Комбинируешь несколько виджетов

### ❌ Используй обычный render() когда:
- Простой виджет в форме
- Уже используешь helpers
- Не нужен сложный layout

## Помощь

- 🚀 [egui-flex Справочник](./EGUI_FLEX_REFERENCE.md) - **Официальная документация по egui-flex 0.5.0**
- 📖 [Полный гайд](./FLEX_WIDGET_GUIDE.md) - Как использовать FlexWidget
- 📝 [Детали реализации](./FLEXWIDGET_IMPLEMENTATION_SUMMARY.md)
- 📋 [План развития](./FLEXWIDGET_TODO.md)
- 💻 [Пример](../examples/notice_flex.rs)

## Вопросы?

1. Начни с [EGUI_FLEX_REFERENCE.md](./EGUI_FLEX_REFERENCE.md) - полный справочник
2. Посмотри примеры в `examples/`
3. Официальная документация: https://docs.rs/egui-flex/0.5.0
4. CSS Flexbox гайд: https://css-tricks.com/snippets/css/a-guide-to-flexbox/

---

**Построено с ❤️ для Nebula workflow engine**

