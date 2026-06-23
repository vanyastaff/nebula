use serde::{Deserialize, Serialize};

/// Rendering **mode** for a `StringField` (single- vs multi-line).
///
/// The *semantic* kind of a string input (email, URL, date, color, …) lives in
/// [`InputHint`](crate::InputHint), the single source of that hint — `StringWidget`
/// carries only the line-mode (symmetric with [`SecretWidget`]). A field's
/// semantic type and its line-mode are orthogonal: a multi-line markdown editor is
/// `widget = Multiline` + `hint = InputHint::Markdown`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StringWidget {
    /// Single-line text input.
    #[default]
    Plain,
    /// Multi-line text input.
    Multiline,
}

/// Widget hints for `SecretField`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretWidget {
    /// Single-line masked input.
    #[default]
    Plain,
    /// Multi-line masked input.
    Multiline,
}

/// Widget hints for `NumberField`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberWidget {
    /// Standard numeric input.
    #[default]
    Plain,
    /// Slider control.
    Slider,
    /// Stepper control.
    Stepper,
    /// Percent-oriented control.
    Percent,
    /// Currency-oriented control.
    Currency,
    /// Duration-oriented control.
    Duration,
    /// Byte-size-oriented control.
    Bytes,
}

/// Widget hints for `BooleanField`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanWidget {
    /// Toggle switch.
    #[default]
    Toggle,
    /// Checkbox.
    Checkbox,
    /// Radio control.
    Radio,
}

/// Widget hints for `SelectField`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectWidget {
    /// Dropdown list.
    #[default]
    Dropdown,
    /// Radio group.
    Radio,
    /// Checkbox list.
    Checkboxes,
    /// Searchable combobox.
    Combobox,
    /// Tag-chip selector.
    Tags,
}

/// Widget hints for `ObjectField`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectWidget {
    /// All fields shown inline.
    #[default]
    Inline,
    /// Collapsible section.
    Collapsed,
    /// Add-field picker.
    PickFields,
    /// Grouped sections view.
    Sections,
    /// Tabbed view.
    Tabs,
}

/// Widget hints for `ListField`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListWidget {
    /// Standard list renderer.
    #[default]
    Plain,
    /// Drag-sortable list.
    Sortable,
    /// Tag-chip list.
    Tags,
    /// Key/value pair list.
    KeyValue,
    /// Accordion list.
    Accordion,
}

/// Widget hints for `CodeField`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeWidget {
    /// Monaco-style rich editor.
    #[default]
    Monaco,
    /// Simple plain editor.
    Simple,
}

#[cfg(test)]
mod tests {
    use super::{BooleanWidget, CodeWidget, NumberWidget, StringWidget};

    #[test]
    fn widget_defaults_are_stable() {
        assert_eq!(StringWidget::default(), StringWidget::Plain);
        assert_eq!(CodeWidget::default(), CodeWidget::Monaco);
    }

    #[test]
    fn widgets_are_non_exhaustive_and_small() {
        use std::mem::size_of;
        assert!(size_of::<StringWidget>() <= 1);
        assert!(size_of::<NumberWidget>() <= 1);
        assert!(size_of::<BooleanWidget>() <= 1);
    }
}
