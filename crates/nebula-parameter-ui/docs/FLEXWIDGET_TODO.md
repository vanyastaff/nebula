# FlexWidget Implementation TODO

## Overview
This document tracks the implementation of FlexWidget trait across all nebula-parameter-ui widgets.

## Status Legend
- ✅ Complete
- 🚧 In Progress
- ⏸️ Planned
- ❌ Not Needed

---

## Widget Implementation Status

### Completed
- ✅ **NoticeWidget** - Reference implementation with full documentation

### High Priority (Complex Layouts)

#### 🚧 ListWidget
**File:** `src/widgets/list.rs`  
**Complexity:** Medium  
**Benefits:** High

**Current Issues:**
- Uses manual `ui.horizontal()` for headers (line 69-105)
- Item layout uses nested groups (line 132-161)
- Add/remove buttons need better alignment

**Recommended FlexItem:**
```rust
FlexItem::new()
    .grow(0.0)     // Don't grow - list has intrinsic size
    .shrink(1.0)   // Can shrink if container is small
```

**Layout Improvements:**
- Header: icon + title (grow) + item count (fixed)
- Items: number (fixed) + content (grow) + delete button (fixed)
- Footer: add button centered or left-aligned

---

#### ⏸️ PanelWidget
**File:** `src/widgets/panel.rs`  
**Complexity:** Medium  
**Benefits:** High

**Current Issues:**
- Tab buttons use `ui.horizontal()` (line 83-109)
- No consistent spacing between tabs
- Active tab highlighting needs better visual separation

**Recommended FlexItem:**
```rust
FlexItem::new()
    .grow(0.0)     // Panel header fixed, content variable
    .basis(0.0)    // Use intrinsic size
```

**Layout Improvements:**
- Tab bar: `Flex::horizontal().gap(4.0)` with even spacing
- Tabs can wrap on small screens with `flex_wrap(FlexWrap::Wrap)`
- Panel content area can use nested flex for child widgets

---

#### ⏸️ GroupWidget
**File:** `src/widgets/group.rs`  
**Complexity:** Medium  
**Benefits:** High

**Current Issues:**
- Field labels use manual `ui.horizontal()` (line 48-68)
- Inconsistent spacing between label and required marker
- Fields could benefit from flex alignment

**Recommended FlexItem:**
```rust
FlexItem::new()
    .grow(0.0)     // Group has intrinsic size
    .basis(0.0)
```

**Layout Improvements:**
- Field labels: `Flex::horizontal()` with label (grow) + required marker (fixed)
- Field layout: label column (fixed width) + input column (grow)
- Better alignment of validation messages

---

#### ⏸️ ObjectWidget
**File:** `src/widgets/object.rs`  
**Complexity:** Low  
**Benefits:** Medium

**Current Issues:**
- Property display uses basic `ui.vertical()` (line 113-131)
- Similar structure to GroupWidget

**Recommended FlexItem:**
```rust
FlexItem::new()
    .grow(0.0)     // Object has intrinsic size
    .shrink(1.0)   // Can shrink if needed
```

**Layout Improvements:**
- Property name + value in horizontal flex
- Consistent spacing between properties
- Better collapsible sections

---

### Medium Priority (Enhanced Layouts)

#### ⏸️ MultiSelectWidget
**File:** `src/widgets/multi_select.rs`  
**Complexity:** Low  
**Benefits:** Medium

**Recommended FlexItem:**
```rust
FlexItem::new()
    .grow(0.0)
    .basis(0.0)
```

**Improvements:**
- Grid layout for checkboxes using flex
- Responsive column count based on width
- Better visual grouping

---

#### ⏸️ RadioWidget
**File:** `src/widgets/radio.rs`  
**Complexity:** Low  
**Benefits:** Medium

**Recommended FlexItem:**
```rust
FlexItem::new()
    .grow(0.0)
    .basis(0.0)
```

**Improvements:**
- Horizontal or vertical radio layout
- Consistent spacing between options
- Better alignment with labels

---

#### ⏸️ ExpirableWidget
**File:** `src/widgets/expirable.rs`  
**Complexity:** Medium  
**Benefits:** High

**Recommended FlexItem:**
```rust
FlexItem::new()
    .grow(0.0)
    .basis(0.0)
```

**Improvements:**
- Input field (grow) + expiration date (fixed)
- Better visual indication of expiration status
- Responsive layout for small screens

---

### Low Priority (Simple Widgets)

These widgets use helper functions and have simple layouts. FlexWidget implementation provides less benefit:

#### ❌ TextWidget
**File:** `src/widgets/text.rs`  
**Reason:** Single input field, already uses helper

#### ❌ TextareaWidget
**File:** `src/widgets/textarea.rs`  
**Reason:** Single input field

#### ❌ NumberWidget
**File:** `src/widgets/number.rs`  
**Reason:** Single drag value widget

#### ❌ SliderWidget
**File:** `src/widgets/slider.rs`  
**Reason:** Single slider widget

#### ❌ CheckboxWidget
**File:** `src/widgets/checkbox.rs`  
**Reason:** Single checkbox

#### ❌ SelectWidget
**File:** `src/widgets/select.rs`  
**Reason:** Single combo box

#### ❌ DateWidget, TimeWidget, DateTimeWidget
**Files:** `src/widgets/date.rs`, `src/widgets/time.rs`, `src/widgets/datetime.rs`  
**Reason:** Use external picker widgets

#### ❌ ColorWidget
**File:** `src/widgets/color.rs`  
**Reason:** Single color picker

#### ❌ CodeWidget
**File:** `src/widgets/code.rs`  
**Reason:** Single text editor

#### ❌ FileWidget
**File:** `src/widgets/file.rs`  
**Reason:** Single file picker

#### ❌ SecretWidget
**File:** `src/widgets/secret.rs`  
**Reason:** Single password field

#### ❌ HiddenWidget
**File:** `src/widgets/hidden.rs`  
**Reason:** No visual output

#### ❌ ModeWidget
**File:** `src/widgets/mode.rs`  
**Reason:** Simple state widget

---

## Implementation Checklist

For each widget implementation, follow these steps:

### 1. Code Changes
- [ ] Import FlexWidget and related types
- [ ] Implement `FlexWidget::default_item()`
- [ ] Implement `FlexWidget::flex_ui()`
- [ ] Add comprehensive doc comments
- [ ] Add usage examples in doc comments

### 2. Internal Layout Migration (Optional)
- [ ] Identify manual `ui.horizontal()`/`ui.vertical()` calls
- [ ] Replace with `Flex::horizontal()`/`Flex::vertical()`
- [ ] Use `FlexItem` for explicit sizing
- [ ] Add appropriate `gap()` for spacing
- [ ] Use `align_items()` for alignment

### 3. Testing
- [ ] Verify widget compiles
- [ ] Check no linter warnings
- [ ] Test basic render works
- [ ] Test with different themes
- [ ] Test in flex container
- [ ] Test responsive behavior

### 4. Documentation
- [ ] Update widget doc comments
- [ ] Add example to FlexWidget guide
- [ ] Update README if needed
- [ ] Add to examples if significant

### 5. Example (If Complex)
- [ ] Create dedicated example file
- [ ] Show multiple use cases
- [ ] Demonstrate responsive behavior
- [ ] Show theme integration

---

## Common Patterns

### Pattern 1: Header with Actions
```rust
Flex::horizontal()
    .w_full()
    .align_items(FlexAlign::Center)
    .show(ui, |flex| {
        // Icon (fixed)
        flex.add_ui(FlexItem::new().grow(0.0).basis(20.0), |ui| {
            ui.label("🔔");
        });
        
        // Title (grows)
        flex.add_ui(FlexItem::new().grow(1.0), |ui| {
            ui.heading("Title");
        });
        
        // Actions (fixed)
        flex.add_ui(FlexItem::new().grow(0.0), |ui| {
            ui.button("Action");
        });
    });
```

### Pattern 2: Form Field
```rust
Flex::horizontal()
    .gap(8.0)
    .align_items(FlexAlign::Center)
    .show(ui, |flex| {
        // Label (fixed width)
        flex.add_ui(FlexItem::new().grow(0.0).basis(120.0), |ui| {
            ui.label("Field Name:");
        });
        
        // Input (grows)
        flex.add_ui(FlexItem::new().grow(1.0), |ui| {
            ui.text_edit_singleline(&mut value);
        });
        
        // Validation icon (fixed)
        flex.add_ui(FlexItem::new().grow(0.0).basis(20.0), |ui| {
            ui.label("✓");
        });
    });
```

### Pattern 3: Vertical Stack
```rust
Flex::vertical()
    .gap(8.0)
    .align_items(FlexAlign::Stretch)
    .show(ui, |flex| {
        for item in items {
            flex.add_ui(FlexItem::new().grow(0.0), |ui| {
                // Item content
            });
        }
    });
```

### Pattern 4: Responsive Layout
```rust
let direction = if ui.available_width() > 600.0 {
    Flex::horizontal()
} else {
    Flex::vertical()
};

direction.gap(12.0).show(ui, |flex| {
    // Items adapt to direction
});
```

---

## Timeline

### Phase 1: Core Complex Widgets (Week 1)
- ListWidget
- PanelWidget
- GroupWidget

### Phase 2: Secondary Widgets (Week 2)
- ObjectWidget
- MultiSelectWidget
- RadioWidget

### Phase 3: Specialized Widgets (Week 3)
- ExpirableWidget
- Any remaining widgets that would benefit

### Phase 4: Polish (Week 4)
- Update all examples
- Comprehensive documentation review
- Performance testing
- User feedback integration

---

## Questions & Decisions

### Q: Should simple widgets implement FlexWidget?
**A:** Yes, even if they don't benefit from internal flex layouts. This provides:
- Consistent API across all widgets
- Users can add any widget to flex containers
- Future flexibility for enhancements

### Q: Should we convert internal layouts to flex?
**A:** Only where it provides clear benefits:
- ✅ Complex multi-element layouts
- ✅ Alignment challenges
- ✅ Responsive requirements
- ❌ Simple single-element widgets
- ❌ Where egui helpers are cleaner

### Q: What about performance?
**A:** Flex has minimal overhead:
- Layout calculations are cached
- No additional allocations
- Compile-time abstraction
- Use sparingly in hot paths (thousands of widgets)

---

## Resources

- [FlexWidget Guide](./FLEX_WIDGET_GUIDE.md)
- [FlexWidget Implementation Summary](./FLEXWIDGET_IMPLEMENTATION_SUMMARY.md)
- [NoticeWidget Reference](../src/widgets/notice.rs)
- [Notice Flex Example](../examples/notice_flex.rs)

---

## Notes

### Migration Strategy
1. Start with `NoticeWidget` as reference
2. Implement one widget at a time
3. Update tests and examples
4. Gather user feedback
5. Iterate on patterns

### Breaking Changes
None expected - FlexWidget is additive:
- Existing `ParameterWidget::render()` continues to work
- FlexWidget is opt-in via `flex_ui()`
- No changes to widget constructors or public API

### Future Enhancements
- Helper functions for common flex patterns
- Widget-specific flex builders
- Flex layout presets (form, dashboard, card)
- Performance benchmarks
- Accessibility improvements

