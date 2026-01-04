# Auto-Dismiss Feature for NoticeWidget

## Overview

NoticeWidget теперь поддерживает автоматическое закрытие через заданное время с визуальным индикатором прогресса!

## Feature Details

### What is Auto-Dismiss?

Auto-dismiss позволяет уведомлениям автоматически исчезать через определенное время. Это полезно для:
- ✅ Toast-уведомлений (кратковременные сообщения)
- ✅ Success notifications (не требуют подтверждения)
- ✅ Информационных сообщений
- ✅ Временных предупреждений

### Visual Feedback

Когда включен auto-dismiss, виджет показывает:
- 📊 **Progress bar** внизу уведомления
- ⏱️ **Animated progress** - плавное заполнение слева направо
- 🎨 **Color-coded** - цвет прогресс бара соответствует типу уведомления

## Usage

### Basic Example

```rust
use nebula_parameter::{NoticeParameter, NoticeParameterOptions, NoticeType, ParameterMetadata};
use nebula_parameter_ui::{NoticeWidget, ParameterWidget};
use std::time::Duration;

let notice = NoticeParameter {
    metadata: ParameterMetadata::builder()
        .key("success_notice")
        .name("Success")
        .build()
        .unwrap(),
    content: "Operation completed successfully!".to_string(),
    options: Some(NoticeParameterOptions {
        notice_type: Some(NoticeType::Success),
        dismissible: true,  // User can also dismiss manually
        auto_dismiss: Some(Duration::from_secs(5)),  // Auto-dismiss after 5 seconds
    }),
    display: None,
};

let mut widget = NoticeWidget::new(notice);
// widget.render(ui);
```

### Different Durations

```rust
// Quick toast (1 second)
auto_dismiss: Some(Duration::from_secs(1))

// Standard notification (3-5 seconds)
auto_dismiss: Some(Duration::from_secs(3))

// Important message (7-10 seconds)
auto_dismiss: Some(Duration::from_secs(10))

// Very long (use with caution)
auto_dismiss: Some(Duration::from_secs(30))
```

### Combining with Notice Types

```rust
// Success - короткое время (3s)
NoticeParameterOptions {
    notice_type: Some(NoticeType::Success),
    auto_dismiss: Some(Duration::from_secs(3)),
    dismissible: true,
}

// Info - среднее время (5s)
NoticeParameterOptions {
    notice_type: Some(NoticeType::Info),
    auto_dismiss: Some(Duration::from_secs(5)),
    dismissible: true,
}

// Warning - длинное время (7s)
NoticeParameterOptions {
    notice_type: Some(NoticeType::Warning),
    auto_dismiss: Some(Duration::from_secs(7)),
    dismissible: true,
}

// Error - очень длинное или без auto-dismiss
NoticeParameterOptions {
    notice_type: Some(NoticeType::Error),
    auto_dismiss: None,  // User must dismiss manually
    dismissible: true,
}
```

## Implementation Details

### How It Works

1. **Timer Initialization**: При первом рендере виджет запоминает текущее время
2. **Progress Calculation**: При каждом рендере вычисляется прогресс (elapsed / duration)
3. **Auto Repaint**: Виджет запрашивает перерисовку когда нужно обновить прогресс бар
4. **Auto Dismiss**: Когда время истекло, виджет автоматически закрывается

### Progress Bar Design

```
┌─────────────────────────────────────┐
│ ℹ  Information                    ✖ │
│ This notice will auto-dismiss       │
├─────────────────────────────────────┤
│ ████████████░░░░░░░░░░░░░░░░░░░░░░ │ ← Progress bar (60% elapsed)
└─────────────────────────────────────┘
```

**Design Properties:**
- Height: 2px (subtle, не отвлекает)
- Background: notice_color.gamma_multiply(0.2) (светлый фон)
- Fill: notice_color.gamma_multiply(0.6) (яркое заполнение)
- Animation: Smooth, updates every frame

### Performance

- **Efficient Repaints**: Виджет запрашивает перерисовку только когда необходимо
- **Minimal CPU Usage**: Использует egui's time system (очень легковесный)
- **No Background Threads**: Всё работает в main UI thread

### State Management

```rust
pub struct NoticeWidget<'a> {
    parameter: NoticeParameter,
    changed: bool,
    dismissed: bool,
    created_at: Option<f64>,  // Timestamp when widget was created
}
```

**Lifecycle:**
1. `created_at = None` - initial state
2. First render → `created_at = Some(current_time)` - timer starts
3. Each render → check if elapsed >= duration
4. If expired → `dismissed = true`, `changed = true`
5. Return zero-sized response (widget hidden)

### Reset Behavior

```rust
widget.reset_dismissed();
// Resets:
// - dismissed = false
// - changed = false  
// - created_at = None  ← Timer reset!
```

После reset, виджет начнет auto-dismiss заново при следующем рендере.

## Best Practices

### ✅ Good Use Cases

```rust
// Success toast
Duration::from_secs(3)  // User sees confirmation, then auto-hide

// Info notification
Duration::from_secs(5)  // Enough time to read, then auto-hide

// Progress update
Duration::from_secs(2)  // Quick update, auto-hide
```

### ⚠️ Caution

```rust
// Warning - longer duration
Duration::from_secs(7)  // User should have time to react

// Error - manual dismiss preferred
auto_dismiss: None  // Important errors shouldn't auto-hide
```

### ❌ Avoid

```rust
// Too short - user can't read
Duration::from_millis(500)  // ❌ Too fast!

// Too long - defeats purpose
Duration::from_secs(60)  // ❌ Just use manual dismiss

// Critical errors - never auto-dismiss
NoticeParameterOptions {
    notice_type: Some(NoticeType::Error),
    auto_dismiss: Some(Duration::from_secs(3)),  // ❌ Critical errors shouldn't auto-hide!
    dismissible: false,  // ❌ And user can't dismiss manually!
}
```

## Examples

### Toast Notifications

```rust
// Success toast pattern
fn show_success_toast(message: &str) -> NoticeWidget<'static> {
    let notice = NoticeParameter {
        metadata: ParameterMetadata::builder()
            .key("toast")
            .name("")  // No title for toasts
            .build()
            .unwrap(),
        content: message.to_string(),
        options: Some(NoticeParameterOptions {
            notice_type: Some(NoticeType::Success),
            dismissible: false,  // Toasts typically don't have close button
            auto_dismiss: Some(Duration::from_secs(3)),
        }),
        display: None,
    };
    NoticeWidget::new(notice)
}

// Usage
let toast = show_success_toast("File saved successfully!");
```

### Notification Center

```rust
// Different durations based on priority
fn create_notification(severity: NoticeType, message: &str) -> NoticeWidget<'static> {
    let duration = match severity {
        NoticeType::Success => Some(Duration::from_secs(3)),
        NoticeType::Info => Some(Duration::from_secs(5)),
        NoticeType::Warning => Some(Duration::from_secs(8)),
        NoticeType::Error => None,  // Manual dismiss
    };

    let notice = NoticeParameter {
        metadata: ParameterMetadata::builder()
            .key("notification")
            .name(format!("{:?}", severity))
            .build()
            .unwrap(),
        content: message.to_string(),
        options: Some(NoticeParameterOptions {
            notice_type: Some(severity),
            dismissible: true,
            auto_dismiss: duration,
        }),
        display: None,
    };
    NoticeWidget::new(notice)
}
```

### Progress Updates

```rust
// Quick progress updates
fn show_progress_update(step: &str) -> NoticeWidget<'static> {
    let notice = NoticeParameter {
        metadata: ParameterMetadata::builder()
            .key("progress")
            .name("Progress")
            .build()
            .unwrap(),
        content: format!("Step completed: {}", step),
        options: Some(NoticeParameterOptions {
            notice_type: Some(NoticeType::Info),
            dismissible: true,
            auto_dismiss: Some(Duration::from_secs(2)),  // Quick update
        }),
        display: None,
    };
    NoticeWidget::new(notice)
}
```

## Running the Example

```bash
cargo run --example notice_auto_dismiss -p nebula-parameter-ui
```

The example demonstrates:
- Creating notices with different durations
- Visual progress bars
- Combining auto-dismiss with manual dismiss
- Different notice types
- Statistics tracking

## Integration with NoticeParameter

Auto-dismiss использует существующую структуру `NoticeParameterOptions`:

```rust
pub struct NoticeParameterOptions {
    pub notice_type: Option<NoticeType>,
    pub dismissible: bool,
    pub auto_dismiss: Option<Duration>,  // ← This field!
}
```

Нет breaking changes - `auto_dismiss` опциональный!

## Future Enhancements

Potential improvements:
- 🎯 Pause on hover (keep notice visible while user reads)
- 🎨 Customizable progress bar style
- ⏸️ Pause/resume API
- 📊 Callbacks on dismiss (know when notice was dismissed)
- 🔔 Sound effects on show/dismiss
- 📱 Stack management (limit max visible notices)

## Summary

✅ **Feature Complete**
- Auto-dismiss after configurable duration
- Visual progress bar
- Smooth animations
- Efficient repaints
- Works with all notice types
- Combines with manual dismiss

✅ **Production Ready**
- No breaking changes
- Backward compatible
- Well documented
- Example included
- Performance optimized

✅ **User Friendly**
- Clear visual feedback
- Intuitive behavior
- Accessible patterns
- Best practices documented

---

**Built with ❤️ for Nebula workflow engine**

